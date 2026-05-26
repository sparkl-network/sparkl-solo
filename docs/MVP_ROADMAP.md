# sparkl-solo MVP Roadmap

## Goal Statement

Build a **working, self-contained sparkl-solo provider node** that can:
1. Register on the ProviderRegistry contract
2. Accept inference requests (OpenAI-compatible API)
3. Produce signed receipts for every inference session
4. Settle earnings on-chain via SettlementEscrow
5. Verify TEE attestation for Tier A providers
6. Discover and route through the P2P network

**Hub EVM `nodeId`:** canonical **`bytes32` = `keccak256(libp2p PeerId multihash)`** (see `identity::on_chain_node_id_*` and **`GET /identity`**); **`peer_id`** in JSON is the libp2p string.

### How to read this document

- **Gap analysis** — snapshot of **main** vs goals (updated when behavior changes).
- **MVP deliverable roadmap** — the **only** prioritized task list (P0–P2; Phase 5 is post-MVP). Use this for planning, not duplicate checklists elsewhere.
- **Related narrative docs** — architecture and tokenomics essays at the end; they do not replace the gap analysis or deliverable sections.

**Priority convention:** **P0** = blocks MVP payments/registry on Hub EVM; **P1** = trust/attestation or strong operational need; **P2** = scale/hardening/UX polish; **Phase 5** = post-MVP / production-hardening themes.

---

## Gap Analysis: Goals vs Current Implementation

### What the Goals Say (README + Architecture)

| Goal Area | What's Promised |
|---|---|
| Provider Registry | Full on-chain registration, heartbeat, tier flags, tier eligibility |
| Model pricing | Network reference prices via `ModelPriceOracle` (off-chain updater) |
| Settlement | Deposit/withdraw DOT/USDC, epoch batching, payout via escrow |
| Attestation | TEE quote verification (NRAS), TEE proof submission to registry |
| Inference Proxy | OpenAI-compatible `/v1/chat/completions`, streaming SSE |
| Receipts | Signed chunk receipts, root computation, verification endpoint |
| P2P Network | libp2p swarm, Kademlia DHT, peer discovery, multi-peer routing |
| Encryption | End-to-end encrypted requests (epk + ciphertext) |
| Model Management | Include/exclude model controls, multi-model support |
| Operations | CI, health checks, config, deployment |

### What's Actually Implemented (current `main`)

| Area | Status | Details |
|---|---|---|
| OpenAI proxy | DONE | `/v1/chat/completions` + `/v1/models` with SSE streaming |
| Receipt generation | DONE | Ed25519-signed chunk receipts, content hashes, session accounting |
| Receipt verification | DONE | `/receipts/verify` endpoint, consumer/provider sig verification |
| Epoch batching | PARTIAL | In-memory epoch batches, receipts root computed, saved to local store |
| Session accounting | DONE | Real `amount_micro_usd` per session, duration tracking |
| TPM attestation | STUB | Local mock/TPM challenge/verify endpoints, swtpm dev path |
| TEE attestation service | STUB | `services/tee-attestation-stub/` - Node.js stub server |
| Provider Registry contract | DONE | Solidity: registration, heartbeat, tier flags, TEE evidence (no per-node pricing) |
| SettlementEscrow contract | DONE | Solidity: deposit/withdraw, epoch settlement, open sessions; bills via `ModelPriceOracle` |
| ModelPriceOracle contract | DONE | Per-model + default reference pricing; `sparkl-oracle-model-price` pushes rates |
| Price Oracle (DIAPriceOracle / RateSetter) | DONE | DOT/USD + USDC/USD feed → USDC/DOT conversion for model oracle |
| Price Oracle (Pyth) | PLACEHOLDER | Reverts with `NotImplemented()` |
| EVM settlement (Rust) | PARTIAL | `settlement/evm.rs` (feature `evm-settlement`): `recordUsage`, operator `settleByOperatorFull` / `settleByOperatorPartial`, read `ProviderRegistry.nodeOperator`, session state; **no** deposit/withdraw in Rust; **`open_session_on_chain()` wired into inference handler — `Session.evm_session_id` now set from `openSession` call** |
| Registry (Rust) | STUB | `src/registry.rs` — Hub-oriented stub; `register()` returns `stub-proof`, heartbeat logs only (no `ProviderRegistry` contract calls yet); config: `registry_contract_address`, optional `registry.evm_rpc_url` |
| Public `/identity` | PARTIAL | `GET /identity` — `node_id` = keccak256(pubkey), Ed25519 chain-anchored proof when built with `evm-settlement` + RPC configured |
| P2P swarm | DONE | libp2p TCP/QUIC, Identify, Ping, mDNS, Kademlia, persistent identity |
| P2P routing | PARTIAL | Multi-peer discovery works locally, no consumer-facing routing |
| Encrypted requests | DONE | `epk` + `ciphertext` in inference path |
| Model visibility | DONE | `include_models` / `exclude_models` config |
| Unicity legacy | REMOVED | Unicity JSON-RPC anchoring and `--features unicity` removed pending a dedicated design |
| CI/CD | PARTIAL | `ci.yml`: Rust build/test + `evm-settlement` compile, clippy; `contracts.yml`: `forge build` + `forge test` (native + Docker matrix) |
| Deployment scripts | DONE | Forge scripts: `DeployLocal.s.sol`, `DeployPaseo.s.sol`, `DeploySparklBase.sol` |

---

## MVP Deliverable Roadmap

### Phase 1: On-Chain Settlement & Registration (CRITICAL PATH)

Work is ordered by **dependency**: settlement client gaps and registry integration are the core blockers; deployment and addresses gate real testnet; public identity supports integrators but does not unblock settlement alone.

#### 1.1 EVM settlement (Rust client)
**Priority: P0**

**Done** (requires `--features evm-settlement`, `settlement.enabled`, keys + contract address):
- [x] `settlement/` wired into `main.rs` (`run_epoch_loop` spawns when settlement enabled)
- [x] `evm-settlement` feature gates Alloy + `settlement/evm.rs`
- [x] Config: `evm_rpc_url`, `escrow_contract`, `evm_provider_wallet_private_key`, `evm_settlement_operator_wallet_private_key`, TEE tuning (`tee_tick_secs`, `tee_settle_tokens_threshold`, `session_min_deposit`, etc.) — see `config/default.toml`
- [x] Contract ABI via `abi/*.json` + `alloy::sol!` in `evm.rs`
- [x] Session settlement path: provider `recordUsage`, operator `settleByOperatorFull` / `settleByOperatorPartial`, read `sessions(sessionId)`, `registry()` → `ProviderRegistry.nodeOperator` validation, optional block-gated TEE cadence
- [x] Degrade gracefully: RPC/keys/operator mismatch → warn and skip tick

**Still missing:**
- [x] **On-chain session id** — `open_session_on_chain()` in `settlement/evm.rs`; `Session.evm_session_id` set from `openSession` call in inference handler; `session_min_deposit` config field (default 1e18)
- [x] **`deposit_dot()` / `deposit_usdc_as_dot()`** — funding paths in Rust (`depositDot`, `depositUsdcAsDot`), HTTP endpoints at `/settlement/deposit-dot` and `/settlement/deposit-usdc`
- [x] **`withdraw_dot()` / `withdraw_provider_dot()`** — provider earnings withdrawal in Rust, HTTP endpoint at `/settlement/withdraw-provider`
- [x] **`settle_epoch_batch()`** — N/A: no batch settle function exists on SettlementEscrow contract; the per-session `settleByOperatorFull`/`settleByOperatorPartial` calls handle epoch settlement

*ProviderRegistry **contract writes** (`register`, heartbeat, chill/defunct): see **§1.3** (not duplicated here).*

#### 1.2 Contract deployment & configuration
**Priority: P0** (gates real Hub EVM/testnet; local/dev can use existing Forge scripts + Anvil)

- [ ] Finalize `DeploySparklBase.sol` for production-shaped deploys
- [ ] Deploy to SparklBase testnet (replace mock oracle/USDC)
- [ ] Deploy DIAPriceOracle with real DIA feeds
- [ ] Publish deployment addresses to checked-in or documented config
- [ ] Verify contracts on-chain (Etherscan/Sourcify)
- [ ] Add `config/evm-mainnet.toml` and `config/evm-testnet.toml` (or equivalent env-driven templates)

#### 1.3 Provider registration (Rust → ProviderRegistry)
**Priority: P0**

- [x] Implement `src/registry.rs` against Hub EVM (on-chain via `alloy::sol!` + `ProviderRegistry` ABI):
  - [x] `register()` — on-chain `registerNode()` call with metadata URI, tier flags, operator payout address
  - [x] `heartbeat()` — on-chain `setTEEProof()` submission with attestation hash
  - [x] **Defunct flow** — `defunct()` calls `markDefunct(nodeId)` on-chain (requires zero open escrow sessions); `deregister()` calls `chillNode()`
  - [x] `get_peer_info()` — `getProvider()` query returning `ProviderInfo`
  - [x] `supports_tier()` — `supportsTier()` query
- [x] `startup_register_with_retry()` — auto-register on startup with exponential backoff (3 retries, 30s base)
- [x] Config: `registry.enabled`, `registry.heartbeat_secs`, `registry.registry_contract_address`, optional `registry.evm_rpc_url`, operator key via `settlement.evm_provider_wallet_private_key`
- [x] Graceful degradation on registration failure (log + continue in log-only mode)
- [x] **Startup registration call** — `startup_register_with_retry()` wired in `main.rs` before `run_heartbeat_loop` when `registry.enabled`

#### 1.4 Public identity (`GET /identity`)
**Priority: P1** (portal / SDK discovery; **not** required for §1.1 session settlement logic)

- [x] `GET /identity` — `peer_id`, `node_id` (keccak256 Ed25519 pubkey), pubkeys, `public_addrs`, optional chain-anchored Ed25519 proof (`sparkl-identity-v1`) when `evm-settlement` + RPC configured
- [ ] Document verification for portal/UI (README or `docs/NETWORK.md`)

---

### Phase 2: TEE Attestation & Trust (CORE DIFFERENTIATOR)

#### 2.1 NRAS production attestation
**Priority: P1**

- [ ] Replace `tee-attestation-stub` with production NRAS client
- [ ] SGX/TDX quote verification against NRAS root of trust
- [ ] SEV/TrustZone/Nitro quote verification paths
- [ ] Submit TEE proofs to ProviderRegistry via `setTEEProof()`
- [ ] Certificate chain validation
- [ ] Attestation challenge flow: provider ↔ attestation service ↔ registry

#### 2.2 Tier A provider verification
**Priority: P1**

- [ ] Consumer-side TEE proof verification (receipts from Tier A providers)
- [ ] Provider-side TEE quote generation (Intel SGX, AMD SEV, AWS Nitro)
- [ ] TEE evidence hash on ProviderRegistry
- [ ] Tier selection in routing/aggregation

---

### Phase 3: P2P Network Hardening

#### 3.1 Multi-peer interoperability
**Priority: P2**

- [ ] Wider multi-peer test coverage (beyond 2-node local setup)
- [ ] Cross-network bootstrap peer exchange
- [ ] DHT bootstrap reliability improvements
- [ ] NAT traversal / relay support (libp2p relay v2)

#### 3.2 Consumer SDK & routing
**Priority: P2**

- [ ] Trustless peer discovery (query ProviderRegistry for registered providers)
- [ ] Direct consumer-to-provider P2P flows
- [ ] Tier-aware routing (TEE tier + price); use **`GET /identity`** (§1.4) where a known provider URL is already established
- [ ] Provider selection (price, latency, reputation)

---

### Phase 4: Operations & Delivery

#### 4.1 CI/CD pipeline
**Priority: P1** (remaining gaps only; primary workflows exist)

- [x] GitHub Actions: Rust `ci.yml` (push/PR) — `cargo build`/`test` with `mock-tpm`, `evm-settlement` compile check, `clippy`
- [x] GitHub Actions: `contracts.yml` — `forge build` + `forge test` (native + Docker matrix)
- [ ] Branch protection: require passing checks (names match org settings)
- [ ] `tests-js/` on schedule or on PR when `tests-js/` changes
- [ ] Build + test on **ARM64** (DGX Spark target)

#### 4.2 Monitoring & health
**Priority: P2**

- [ ] Prometheus metrics (sessions, earnings, peer count)
- [x] `/status` and `/status/detail` (gated by `expose_status_detail`) — readiness, peers, settlement flag, identity fingerprints
- [ ] Live checks: escrow RPC + registry when EVM enabled (`/status/detail` or dedicated probe)
- [ ] Alerting on registration failure, settlement errors
- [ ] Log rotation and structured logging

---

### Phase 5: Production Readiness (Post-MVP)

**Priority: P3** — themes below are ordered roughly by typical dependency (oracle before dashboards that show money); all are post-MVP relative to Phase 1–2.

#### 5.1 Price oracle
**Priority: P3**

- [ ] PythPriceOracle (or production oracle strategy replacing DIAPriceOracle where required)
- [ ] Oracle price freshness validation
- [ ] Stale price handling / circuit breaker

#### 5.2 ZKP double-spend protection
**Priority: P3**

- [ ] Design ZKP scheme for receipt uniqueness
- [ ] Integrate with SettlementEscrow
- [ ] Prover/verifier implementation

#### 5.3 Dashboard & UI
**Priority: P3**

- [ ] Provider status dashboard
- [ ] Earnings tracking
- [ ] Network health visualization
- [ ] Provider registration UI

---

## Dependency Graph

```text
Phase 1.1 (settlement Rust + session id + funds) ─┐
Phase 1.2 (deploy & config addresses) ────────────┼──→ Phase 1.3 (registry Rust)
                                                   │
                                                   └──→ Phase 1.4 (/identity) — parallel integrator surface
                                                                   ↓
Phase 2.1 (NRAS) ──→ Phase 2.2 (tier verification) ──→ Phase 3.2 (consumer SDK & routing)
                                                            │
Phase 3.1 (multi-peer) ─────────────────────────────────────┘
                                                            ↓
                                                     Phase 4 (ops)
                                                            ↓
                                                     Phase 5 (post-MVP)
```

---

## Risk Assessment

| Risk | Impact | Mitigation |
|---|---|---|
| SparklBase testnet not ready | Blocks production-shaped Phase 1.2 | Anvil local testnet; mock EVM in integration tests |
| `Session.evm_session_id` never wired | Settlement never runs on real traffic | Consumer passes chain session id into node; Anvil integration test |
| NRAS integration complexity | Delays Phase 2 | Mock TEE path until NRAS is ready |
| Token / org permission limits | Blocks automation (e.g. Projects) | Manual tracking; document in repo |
| ARM64 build gaps | Weak signal on DGX-class targets | Phase 4.1 ARM job or remote build |
| DIA oracle reliability | Pricing / settlement surprises | Pyth / staleness guards (Phase 5.1) |

---

## Related Narrative Documentation

Long-form context (not a second task list):

- `docs/Sparkle  Decentralised Private AI Inference for NVIDIA DGX Spark Owners.md`
- `docs/Sparkle Tokenomics  SPARKL Token Design and Network Economics.md`
- `docs/TPM-development.md`

### Theme backlog (cross-references only)

Avoid duplicating phased checklists from the old README. Track work only in **MVP Deliverable Roadmap** above. Loose themes not worth separate sections yet:

| Theme | See |
|---|---|
| Tier-aware routing, attestation ↔ registry | §2.x, §3.2 |
| Production oracle, ZKP, dashboard | §5.1–5.3 |
| Substrate solo chain | Future architecture (not MVP Hub EVM path) |
| QUIC-ws / stream multiplexing / 0-RTT | Future transport (P2P / inference) |

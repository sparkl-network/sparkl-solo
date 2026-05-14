# DEVELOPER GUIDE

This guide covers local development for `sparkl-solo`, including running two local nodes with different config files so they can discover each other.

## Target architecture (updated)

**Goals**

- Payments and escrow on **Polkadot Hub EVM** (`pallet_revive`): **native DOT** first; **USDC** later via the Hub **ERC-20 precompile**.
- Two provider tiers:
  - **Tier A — TEE-verified:** confidential, verifiable inference (hardware-backed attestation).
  - **Tier B — Best-effort:** no hardware guarantees; cheaper.
- The **consumer picks the tier**; pricing can differ by tier.

**On-chain (Polkadot Hub)**

- **SettlementEscrow** — DOT escrow; USDC path credits internal DOT balances via the price oracle.
- **ProviderRegistry** — payout addresses, per-tier pricing, active flag, and on-chain **TEE verified** state (evidence hash).
- **PriceOracle** — `IPriceOracle` for USDC↔DOT (MVP: DIA feeds; later: Pyth).

Solidity sources for these contracts live under [`contracts/`](./contracts/) (Foundry layout).

**Local EVM:** from `contracts/`, install [Foundry](https://book.getfoundry.sh/getting-started/installation) (or run `forge`/`anvil` via the official Docker image). Pull `forge-std` if needed: `forge install --no-git foundry-rs/forge-std`. Run `forge test` for unit tests (`ProviderRegistry`, `SettlementEscrow`, mocks). Start `anvil` (or any JSON-RPC dev node), then broadcast `script/DeployLocal.s.sol` with `forge script ... --rpc-url http://127.0.0.1:8545 --broadcast`.

**Paseo (Hub testnet):** broadcast `contracts/script/DeployPaseo.s.sol` against the Paseo Hub EVM RPC; exported addresses land in [`contracts/deployments/paseo.json`](./contracts/deployments/paseo.json). See **Deploy to Paseo** below for a repeatable checklist.

**Off-chain**

- **Attestation service** — verifies TEE quotes and calls the registry to record **TEE verified** + evidence hash. Until hardware attestation lands, use the MVP stub service in [`services/tee-attestation-stub/`](./services/tee-attestation-stub/) (challenge/signature PoP → `setTEEProof`; see **[TEE-tier onboarding](#tee-tier-onboarding-stub-attestation)** below).
- **Aggregators** — route traffic by user tier, declared pricing, and eligibility from the registry.

**Provider nodes**

- **Tier A:** SGX / TDX / SEV / TrustZone / Nitro (and similar).
- **Tier B:** ordinary GPU/CPU serving without TEE guarantees.

**Legacy / transitional**

- Unicity receipt anchoring has been **removed** from this tree for now; new work uses **Polkadot Hub EVM** (`[registry]` + `[settlement]`).

## TEE-tier onboarding (stub attestation)

**On-chain.** [`ProviderRegistry.setTEEProof(bytes32 nodeId, bytes32 teeReportHash)`](./contracts/src/ProviderRegistry.sol) records the Tier A (**`TEE_VERIFIED`**) flag and evidence digest. Only the registry **`attestationService`** address may call it; the owner sets that address via `setAttestationService` (deploy-time constructor + optional updates). The hash is typically **`keccak256(attestationReportBytes)`** for an opaque `report` blob (stub today; later DCAP / SEV-SNP / Nitro verification fills in that blob).

**Off-chain stub.** [`services/tee-attestation-stub/`](./services/tee-attestation-stub/README.md) is a tiny HTTP service that:

1. Issues a short-lived **challenge** (`GET /v1/challenge`).
2. Verifies the **node operator** wallet signed that challenge (EIP-191 `personal_sign` / `Wallet.signMessage` of the returned `message`). The recovered signer must equal **`nodeOperator(nodeId)`** on the registry.
3. Submits **`setTEEProof(nodeId, keccak256(report))`** with **`ethers.js`** using **`ADMIN_PRIVATE_KEY`**, which **must** be the same key as `registry.attestationService()` on the target chain.

**Provider checklist (pre–real TEE):**

1. Deploy `ProviderRegistry` + `SettlementEscrow` (e.g. [`contracts/script/DeployLocal.s.sol`](./contracts/script/DeployLocal.s.sol) on Anvil, or [`contracts/script/DeployPaseo.s.sol`](./contracts/script/DeployPaseo.s.sol) on Paseo). Note `ProviderRegistry` and **`attestationService`** on deploy.
2. From the **operator key**, call **`registerNode(nodeId, ...)`** so the node row exists (required for `setTEEProof`). **`nodeId`** is a **`bytes32`** on-chain identity (e.g. Substrate PeerId hash), **not** the operator EOA.
3. Configure the stub: `RPC_URL`, `PROVIDER_REGISTRY_ADDRESS`, `ADMIN_PRIVATE_KEY` (**attestation signer** env var name is historical — it is the **`attestationService`** key). Run `yarn start` in `services/tee-attestation-stub/`.
4. Client: `GET /v1/challenge` → operator signs **`message`** with the wallet that is **`nodeOperator(nodeId)`** → `POST /v1/attest` with **`nodeId`** (`0x` + 64 hex, the same **`bytes32`** passed to **`registerNode`**), `report` (hex stub bytes), `challengeId`, `signature`.
5. Confirm on-chain (`supportsTier(nodeId, TEE_VERIFIED)`, `teeReportHash`) via `cast`, explorer, or your indexer.
6. **Later:** Replace stub `report` handling with real verifier logic; keep the same on-chain **`keccak256(report)`** commitment pattern or migrate via a governance-approved registry upgrade path.

Detailed API and snippets: **`services/tee-attestation-stub/README.md`**.

## Prerequisites

- Rust toolchain (stable)
- From this directory: `sparkl-solo/`

## Build and test

- Build with mock TPM mode:
  - `cargo build --features mock-tpm`
- Build with TPM feature:
  - `cargo build --features tpm`
- Run tests:
  - `cargo test --features mock-tpm`
  - `cargo test --features tpm`

## Deploy to Paseo (Hub testnet — manual checklist)

Use Foundry scripts under [`contracts/`](./contracts/); deployed addresses default to **`contracts/deployments/paseo.json`**. This path uses **mock oracle + mock USDC** embedded in Solidity (MVP for testnet rehearsal); switching to Hub precompiles feeds is follow-up.

1. **Tooling:** Install [Foundry](https://book.getfoundry.sh/getting-started/installation). From `contracts/`: run `forge build` and `forge test` locally.
2. **RPC:** Decide the **Paseo Hub EVM** JSON-RPC endpoint (often kept in `PASEO_RPC`). Confirm chain id matches expectations on that network.
3. **Deploy account:** Obtain a funded Hub-EVM key for broadcasts (native DOT/test tokens on Paseo for gas). Prefer a **dedicated deployment key**, not a hot production key.
4. **Secrets:** `export PRIVATE_KEY=0x...` in your shell session only (`cast wallet import`, hardware wallet QR, etc.). Never commit `.env` with keys into git.
5. **Registry attestation signer (optional):** If the address allowed to call `ProviderRegistry.setTEEProof` should differ from the deployer (owner remains deployer unless you migrate ownership later), export `ATTESTATION_SERVICE=0x...`. Otherwise omit it; the deployer is used at deploy time ([`contracts/script/DeployPaseo.s.sol`](./contracts/script/DeployPaseo.s.sol)).
6. **Simulate:** `forge script script/DeployPaseo.s.sol:DeployPaseo --rpc-url "$PASEO_RPC"` (omit `--broadcast`) and confirm gas / revert output.
7. **Broadcast:** `forge script script/DeployPaseo.s.sol:DeployPaseo --rpc-url "$PASEO_RPC" --broadcast`
8. **Artifacts:** Confirm `contracts/deployments/paseo.json` exists and captures `providerRegistry`, `settlementEscrow`, **`sparklNetworkConfig`** (CREATE2 bootstrap), **`networkConfigSalt`**, `mockOracle`, `mockUsdc`, timestamps, `chainId`. See **[SparklNetworkConfig bootstrap](#sparklnetworkconfig-bootstrap)** below. Optionally commit addresses (no secrets) for team parity.
9. **Node wiring:** Set `settlement.evm_rpc_url` to `"$PASEO_RPC"` (or canonical endpoint). Either bake **`sparklNetworkConfig`** into **`src/network_config.rs`** (`SPARKL_NETWORK_CONFIG_ADDRESS`) and enable **`settlement.enabled`** with **`evm-settlement`**, **or** set `settlement.escrow_contract` and `registry.registry_contract_address` manually from the JSON. Toggle `settlement.enabled` when integration is enabled.
10. **Verification:** When Hub EVM block explorers support Solidity verification compatible with Forge, retry with Forge’s `--verify` flags and your chain’s API key docs.
11. **Output path:** To write elsewhere set `DEPLOYMENTS_OUT=my/path.json` (relative to `contracts/`).

Detailed one-liners and env table: [`contracts/README.md`](./contracts/README.md) (section **Paseo testnet**).

## SparklNetworkConfig bootstrap

Deploy scripts CREATE2-deploy **`SparklNetworkConfig`** with fixed salt **`keccak256(bytes("sparkl.network.config.v1"))`** (see **`contracts/script/DeploySparklBase.sol`**) and write **`sparklNetworkConfig`** plus **`networkConfigSalt`** to **`contracts/deployments/paseo.json`**. Field shape: **`contracts/deployments/paseo.example.json`**.

For **`sparkl-solo`**: update **`SPARKL_NETWORK_CONFIG_ADDRESS`** in **`src/network_config.rs`** to match the deployment, then build with **`evm-settlement`**. With **`settlement.enabled`**, **`main`** resolves **`providerRegistry`** and **`settlementEscrow`** via **`eth_call`** and patches the in-memory config before registry/settlement tasks start. Placeholder **`0x000…000`** skips bootstrap and uses **`[registry]`** / **`[settlement]`** TOML (and **`--registry-contract`** / **`--escrow-contract`** overrides) as before.

**Deferred (phase 2):** background polling of **`version()`** and hot-reloading resolved addresses inside long-running loops (`Arc<RwLock<…>>` or watch channel) is follow-up work.

## macOS TPM2 tooling note

- `swtpm` is available on macOS via Homebrew:
  - `brew install swtpm`
- `tpm2-tools` is currently not available in Homebrew core (`brew install tpm2-tools` fails with "No available formula").
- Current node behavior with `--features tpm` on macOS:
  - if `TCTI`/`TPM2TOOLS_TCTI` is set and `tpm2_getrandom` exists, identity is marked TPM-backed (`cert_type: "swtpm"`).
  - if `tpm2_getrandom` is missing, node automatically falls back to software identity (`cert_type: "mock-software"`).
- For full TPM2 CLI validation (`tpm2_getcap`, `tpm2_getrandom`, etc.), use a Linux machine/container with `tpm2-tools` installed.

## Create local config files

Create a `dev-config/` folder:

```bash
mkdir -p dev-config
```

Create `dev-config/node1.toml`:

```toml
[node]
name = "local-node-1"
data_dir = "./dev-data/node1"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/31001"]
inference_port = 9944
bootstrap_peers = []
public_addr = []
expose_status_detail = true
allow_non_globals_in_dht = true

[backend]
url = "http://127.0.0.1:11434"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 120

[attestation]
nras_url = "https://nras.attestation.nvidia.com"
nras_enabled = false
cert_ttl_days = 7

[registry]
registry_contract_address = "0x0000000000000000000000000000000000000000"
evm_rpc_url = ""
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
# Polkadot Hub EVM (pallet_revive) — set your RPC and deployed SettlementEscrow
evm_rpc_url = "https://YOUR_POLKADOT_HUB_EVM_RPC"
escrow_contract = "0x0000000000000000000000000000000000000000"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
```

Start node 1 once so it prints/logs its libp2p peer ID. You will use that in node 2 config.

Create `dev-config/node2.toml` (replace `<NODE1_PEER_ID>`):

```toml
[node]
name = "local-node-2"
data_dir = "./dev-data/node2"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/31002"]
inference_port = 9945
bootstrap_peers = ["/ip4/127.0.0.1/tcp/31001/p2p/<NODE1_PEER_ID>"]
public_addr = []
expose_status_detail = true
allow_non_globals_in_dht = true

[backend]
url = "http://127.0.0.1:11434"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 120

[attestation]
nras_url = "https://nras.attestation.nvidia.com"
nras_enabled = false
cert_ttl_days = 7

[registry]
registry_contract_address = "0x0000000000000000000000000000000000000000"
evm_rpc_url = ""
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
# Polkadot Hub EVM (pallet_revive) — set your RPC and deployed SettlementEscrow
evm_rpc_url = "https://YOUR_POLKADOT_HUB_EVM_RPC"
escrow_contract = "0x0000000000000000000000000000000000000000"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
```

## Run two local nodes

In terminal 1:

```bash
cargo run --features mock-tpm -- --config dev-config/node1.toml
```

In terminal 2:

```bash
cargo run --features mock-tpm -- --config dev-config/node2.toml
```

## Verify local connectivity

- Watch logs in both terminals for:
  - new listen address events
  - connection established events
  - identify/kademlia events
- Hit health/status:
  - `curl http://127.0.0.1:9944/health`
  - `curl http://127.0.0.1:9945/health`
  - `curl http://127.0.0.1:9944/status` (minimal public readiness)
  - `curl http://127.0.0.1:9945/status` (minimal public readiness)
  - `curl http://127.0.0.1:9944/status/detail` (operator diagnostics)
  - `curl http://127.0.0.1:9945/status/detail` (operator diagnostics)

## Verify with automated test

Run the two-node integration test:

```bash
cargo test --features mock-tpm two_nodes_discover_each_other_with_separate_configs -- --nocapture
```

## Verify with JavaScript tests

Run operational checks from the `tests-js/` folder:

```bash
cd tests-js
yarn install
yarn status
yarn attestation
yarn encrypted
yarn tpm:suite
```

## Notes

- Config overrides are available by both CLI flags and environment variables.
  - env var format: `SPARKLE__SECTION__KEY=value`
  - example: `SPARKLE__NETWORK__INFERENCE_PORT=19944`
  - example: `SPARKLE__NETWORK__EXPOSE_STATUS_DETAIL=true`
  - for array values, pass JSON: `SPARKLE__NETWORK__PUBLIC_ADDR='["/ip4/203.0.113.10/tcp/30333"]'`
  - **Polkadot Hub EVM:** point `settlement.evm_rpc_url` (and `escrow_contract`) at your Hub deployment; optional `registry.registry_contract_address` and `registry.evm_rpc_url` (empty = same settlement RPC); use `SPARKLE__SETTLEMENT__*` / `SPARKLE__REGISTRY__*` overrides as needed
- `mock-tpm` is required for laptop/dev workflows.
- Registry and settlement are disabled in these local configs.
- If using `llama-swap` on `:8000`, set `backend.url = "http://127.0.0.1:8000"`.
- libp2p peer identity is persisted under `<data_dir>/network/`:
  - `secret_ed25519` (private key, generated once if missing)
  - `peer_id` (derived PeerId text)
- Receipt cadence can be tuned per node:
  - config: `node.receipt_cadence_tokens = 50`
  - CLI override: `--receipt-cadence NUM_TOKS`
- Model exposure can be filtered per node:
  - config allow-list: `node.include_models = ["model/a", "model/b"]`
  - config block-list: `node.exclude_models = ["model/private"]`
  - CLI overrides: `--include-models model/a,model/b` and `--exclude-models model/private`
  - filtering order: include first (or all models when include is empty), then exclude
- You can advertise internet-facing node addresses for DHT propagation:
  - config: `network.public_addr = ["/ip4/<ext-ip>/tcp/<ext-port>", "/ip4/<ext-ip>/tcp/<ext-port2>/ws", "/ip4/<ext-ip>/tcp/<ext-port>/p2p/<peer-id>"]`
  - peer id in `public_addr` is optional; if omitted, local peer id is used for DHT insertion
  - for `bootstrap_peers`, keep `/p2p/<peer-id>` present so dial targets are identity-pinned
  - these addresses are added to external swarm addresses and inserted into Kademlia on startup
- Detailed status endpoint exposure:
  - config: `network.expose_status_detail = false` (default)
  - config: `network.expose_status_detail = true` to enable `/status/detail`
  - CLI override: `--expose-status-detail true|false`
- Substrate-style DHT filtering:
  - config: `network.allow_non_globals_in_dht = false` (default)
  - when false, private/local IPs (e.g. `127.0.0.1`, `10.0.0.0/8`, `192.168.0.0/16`) learned from peers are not inserted into DHT
  - set true for local/dev networks where non-global addresses are expected

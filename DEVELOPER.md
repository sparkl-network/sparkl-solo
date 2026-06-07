# DEVELOPER GUIDE

This guide covers local development for `sparkl-solo`, including running two local nodes with different config files so they can discover each other.

**Wallets and keys:** See [../docs/Wallets-and-Keys.md](../docs/Wallets-and-Keys.md) for operator vs settlement vs consumer EVM roles, libp2p `peer_id`, and multi-node patterns.

## Target architecture (updated)

**Goals**

- Payments and escrow on **Polkadot Hub EVM** (`pallet_revive`): **native DOT** first; **USDC** later via the Hub **ERC-20 precompile**.
- Two provider tiers:
  - **Tier A — TEE-verified:** confidential, verifiable inference (hardware-backed attestation).
  - **Tier B — Best-effort:** no hardware guarantees; cheaper.
- The **consumer picks the tier**; TEE sessions bill at oracle rates × on-chain TEE multiplier.

**On-chain (Polkadot Hub)**

- **SettlementEscrow** — DOT escrow; bills sessions from token usage × `ModelPriceOracle` rates.
- **ProviderRegistry** — payout addresses, tier eligibility, active flag, and on-chain **TEE verified** state (evidence hash).
- **ModelPriceOracle** — per-model reference pricing (and on-chain defaults for unknown models).
- **PriceOracle** — `IPriceOracle` for USDC↔DOT (MVP: DIA feeds; later: Pyth).

Solidity sources for these contracts live under `[contracts/](./contracts/)` (Foundry layout).

**Local EVM:** from `contracts/`, install [Foundry](https://book.getfoundry.sh/getting-started/installation) (or run `forge`/`anvil` via the official Docker image). Pull `forge-std` if needed: `forge install --no-git foundry-rs/forge-std`. Run `forge test` for unit tests (`ProviderRegistry`, `SettlementEscrow`, mocks).

**Launch service individually**

from sparkl-network:

Anvil:
```bash
cd /sparkl-solo/contracts && anvil --state ../.launch/anvil-state.json
```

Contracts (deploy + sync addresses into portal, oracles, router, and solo configs):
```bash
# Anvil must be listening on 127.0.0.1:8545 (or use --start-anvil)
./scripts/deploy-local-sync-env.sh
# Or start Anvil automatically:
./scripts/deploy-local-sync-env.sh --start-anvil
```

Manual deploy only (no env sync):
```bash
cd contracts && forge build
forge script script/DeployLocal.s.sol:DeployLocal --rpc-url http://127.0.0.1:8545 --broadcast
```

Router:
```bash
cd sparkl-router && cargo run -- config/default.toml
```

```bash
cd sparkl-solo && cargo run -- --config dev-config/launch.toml
```

```bash
cd sparkl-portal && yarn dev
```

Test the api key against the router api:
```bash
export API_KEY=sk_11111111111111111111111111111111HVWf3mkzwrQhcm5PC9LB2oSiH9V7t7DT8EiggHC22ZB
# test openai inference
curl -X POST http://localhost:3001/v1/chat/completions \
-H "Authorization: Bearer $API_KEY" \
-H "Content-Type: application/json" \
-d '{
  "model": "qwen/qwen3.6-27b",
  "messages": [{"role": "user", "content": "Hello, how are you?"}]
}'
```

**One-command local stack** (Anvil + deploy + Rust tests + sparkl-router + solo node + portal/oracle env hints):

```bash
./scripts/launch-local.sh
```

**Interactive tmux grid** (2×2 panes: Anvil, router, solo, portal — Ctrl+C and up-arrow per pane). Run from **`sparkl-network/`**:

```bash
cd sparkl-network && ./scripts/launch-grid.sh
```

Prep deploys contracts (if needed), syncs ABIs, writes `sparkl-solo/dev-config/launch.toml`, `sparkl-solo/dev-config/router-launch.toml`, and `sparkl-portal/.env.local`, then starts a detached `sparkl-grid` tmux session. Use `--no-attach` to leave tmux in the background; `--attach-only` to reattach.

Options:

| Flag | Effect |
|------|--------|
| `--skip-tests` | Skip Forge + Rust tests |
| `--skip-node` | Deploy + tests only; leave Anvil running |
| `--skip-router` | Do not start sparkl-router; solo `[router].enabled = false` |
| `--keep-anvil` | Do not stop Anvil on exit |
| `--no-state` | Ephemeral Anvil (default persists to `.launch/anvil-state.json`) |
| `--force-deploy` | Always run `DeployLocal` |
| `--skip-deploy` | Never broadcast; require matching on-chain deployments |

Skips `forge script` when artifact fingerprint (`.launch/deploy-fingerprint`) and on-chain bytecode/linkage match `contracts/deployments/local.json`. After launch, the script writes `dev-config/launch.toml` and `dev-config/router-launch.toml`, starts **sparkl-router** (unless `--skip-router`) with `chain.enabled = true` against local Anvil contracts, starts a solo node with `[router]` tunnel enabled, and prints **sparkl-portal** `NEXT_PUBLIC_*` / router env values and **sparkl-oracle-rates** `.env` (DOT/USD `RateSetter`; no `sparkl-oracle-model-price` service — default model price is seeded once on-chain if unset).

**Router WSS:** with the default local router config, the node must be **commercially registered** on portal `/node/register` before the tunnel is accepted. Use `curl http://127.0.0.1:19950/identity` for `node_id` after the node starts.

Deploy-only: `cd contracts && anvil --state ../.launch/anvil-state.json &` then `forge script script/DeployLocal.s.sol:DeployLocal --rpc-url http://127.0.0.1:8545 --broadcast` (writes `contracts/deployments/local.json`).

**Paseo (Hub testnet):** broadcast `contracts/script/DeployPaseo.s.sol` against the Paseo Hub EVM RPC; exported addresses land in `[contracts/deployments/paseo.json](./contracts/deployments/paseo.json)`. See **Deploy to Paseo** below for a repeatable checklist.

**Off-chain**

- **Attestation service** — verifies TEE quotes and calls the registry to record **TEE verified** + evidence hash. Until hardware attestation lands, use the MVP stub service in `[services/tee-attestation-stub/](./services/tee-attestation-stub/)` (challenge/signature PoP → `setTEEProof`; see **[TEE-tier onboarding](#tee-tier-onboarding-stub-attestation)** below).
- **Aggregators** — route traffic by user tier, declared pricing, and eligibility from the registry.

**Provider nodes**

- **Tier A:** SGX / TDX / SEV / TrustZone / Nitro (and similar).
- **Tier B:** ordinary GPU/CPU serving without TEE guarantees.

**Legacy / transitional**

- Unicity receipt anchoring has been **removed** from this tree for now; new work uses **Polkadot Hub EVM** (`[registry]` + `[settlement]`).

## TEE-tier onboarding (stub attestation)

**On-chain.** `[ProviderRegistry.setTEEProof(bytes32 nodeId, bytes32 teeReportHash)](./contracts/src/ProviderRegistry.sol)` records the Tier A (`**TEE_VERIFIED`**) flag and evidence digest. Only the registry `**attestationService`** address may call it; the owner sets that address via `setAttestationService` (deploy-time constructor + optional updates). The hash is typically `**keccak256(attestationReportBytes)**` for an opaque `report` blob (stub today; later DCAP / SEV-SNP / Nitro verification fills in that blob).

**Off-chain stub.** `[services/tee-attestation-stub/](./services/tee-attestation-stub/README.md)` is a tiny HTTP service that:

1. Issues a short-lived **challenge** (`GET /v1/challenge`).
2. Verifies the **node operator** wallet signed that challenge (EIP-191 `personal_sign` / `Wallet.signMessage` of the returned `message`). The recovered signer must equal `**nodeOperator(nodeId)`** on the registry.
3. Submits `**setTEEProof(nodeId, keccak256(report))`** with `**ethers.js**` using `**ADMIN_PRIVATE_KEY**`, which **must** be the same key as `registry.attestationService()` on the target chain.

**Provider checklist (pre–real TEE):**

1. Deploy `ProviderRegistry` + `SettlementEscrow` (e.g. `[contracts/script/DeployLocal.s.sol](./contracts/script/DeployLocal.s.sol)` on Anvil, or `[contracts/script/DeployPaseo.s.sol](./contracts/script/DeployPaseo.s.sol)` on Paseo). Note `ProviderRegistry` and `**attestationService`** on deploy.
2. **Commercially register** on the portal (`/node/register` → `registerNode`) so the node row exists (required for `setTEEProof` and router WSS subscription). `**nodeId`** is a `**bytes32**` from the libp2p peer id, **not** the operator EOA.
3. Configure the stub: `RPC_URL`, `PROVIDER_REGISTRY_ADDRESS`, `ADMIN_PRIVATE_KEY` (**attestation signer** env var name is historical — it is the `**attestationService`** key). Run `yarn start` in `services/tee-attestation-stub/`.
4. Client: `GET /v1/challenge` → operator signs `**message`** with the wallet that is `**nodeOperator(nodeId)**` → `POST /v1/attest` with `**nodeId**` (`0x` + 64 hex, the same `**bytes32**` passed to `**registerNode**`), `report` (hex stub bytes), `challengeId`, `signature`.
5. Confirm on-chain (`supportsTier(nodeId, TEE_VERIFIED)`, `teeReportHash`) via `cast`, explorer, or your indexer.
6. **Later:** Replace stub `report` handling with real verifier logic; keep the same on-chain `**keccak256(report)`** commitment pattern or migrate via a governance-approved registry upgrade path.

Detailed API and snippets: `**services/tee-attestation-stub/README.md`**.

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

Use Foundry scripts under `[contracts/](./contracts/)`; deployed addresses default to `**contracts/deployments/paseo.json**`. This path uses **mock oracle + mock USDC** embedded in Solidity (MVP for testnet rehearsal); switching to Hub precompiles feeds is follow-up.

1. **Tooling:** Install [Foundry](https://book.getfoundry.sh/getting-started/installation). From `contracts/`: run `forge build` and `forge test` locally.
2. **RPC:** Decide the **Paseo Hub EVM** JSON-RPC endpoint (often kept in `PASEO_RPC`). Confirm chain id matches expectations on that network.
3. **Deploy account:** Obtain a funded Hub-EVM key for broadcasts (native DOT/test tokens on Paseo for gas). Prefer a **dedicated deployment key**, not a hot production key.
4. **Secrets:** `export PRIVATE_KEY=0x...` in your shell session only (`cast wallet import`, hardware wallet QR, etc.). Never commit `.env` with keys into git.
5. **Registry attestation signer (optional):** If the address allowed to call `ProviderRegistry.setTEEProof` should differ from the deployer (owner remains deployer unless you migrate ownership later), export `ATTESTATION_SERVICE=0x...`. Otherwise omit it; the deployer is used at deploy time (`[contracts/script/DeployPaseo.s.sol](./contracts/script/DeployPaseo.s.sol)`).
6. **Simulate:** `forge script script/DeployPaseo.s.sol:DeployPaseo --rpc-url "$PASEO_RPC"` (omit `--broadcast`) and confirm gas / revert output.
7. **Broadcast:** `forge script script/DeployPaseo.s.sol:DeployPaseo --rpc-url "$PASEO_RPC" --broadcast`
8. **Artifacts:** Confirm `contracts/deployments/paseo.json` exists and captures `providerRegistry`, `settlementEscrow`, `**sparklNetworkConfig`** (CREATE2 bootstrap), `**networkConfigSalt`**, `mockOracle`, `mockUsdc`, timestamps, `chainId`. See **[SparklNetworkConfig bootstrap](#sparklnetworkconfig-bootstrap)** below. Optionally commit addresses (no secrets) for team parity.
9. **Node wiring:** Set `settlement.evm_rpc_url` to `"$PASEO_RPC"` (or canonical endpoint). Either bake `**sparklNetworkConfig`** into `**src/network_config.rs`** (`SPARKL_NETWORK_CONFIG_ADDRESS`) and enable `**settlement.enabled**` with `**evm-settlement**`, **or** set `settlement.escrow_contract` and `registry.registry_contract_address` manually from the JSON. Toggle `settlement.enabled` when integration is enabled.
10. **Verification:** When Hub EVM block explorers support Solidity verification compatible with Forge, retry with Forge’s `--verify` flags and your chain’s API key docs.
11. **Output path:** To write elsewhere set `DEPLOYMENTS_OUT=my/path.json` (relative to `contracts/`).

Detailed one-liners and env table: `[contracts/README.md](./contracts/README.md)` (section **Paseo testnet**).

## SparklNetworkConfig bootstrap

Deploy scripts CREATE2-deploy `**SparklNetworkConfig`** with fixed salt `**keccak256(bytes("sparkl.network.config.v1"))`** (see `**contracts/script/DeploySparklBase.sol**`) and write `**sparklNetworkConfig**` plus `**networkConfigSalt**` to `**contracts/deployments/paseo.json**`. Field shape: `**contracts/deployments/paseo.example.json**`.

For `**sparkl-solo**`: update `**SPARKL_NETWORK_CONFIG_ADDRESS**` in `**src/network_config.rs**` to match the deployment, then build with `**evm-settlement**`. With `**settlement.enabled**`, `**main**` resolves `**providerRegistry**` and `**settlementEscrow**` via `**eth_call**` and patches the in-memory config before registry/settlement tasks start. Placeholder `**0x000…000**` skips bootstrap and uses `**[registry]**` / `**[settlement]**` TOML (and `**--registry-contract**` / `**--escrow-contract**` overrides) as before.

**Deferred (phase 2):** background polling of `**version()`** and hot-reloading resolved addresses inside long-running loops (`Arc<RwLock<…>>` or watch channel) is follow-up work.

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
```

Billing is oracle-driven: `SettlementEscrow` prices sessions from `ModelPriceOracle` (see `/model` in sparkl-portal). No local `[pricing]` TOML.

**Router path (default with `launch-local.sh`):** sparkl-router parses upstream `usage`, batches `recordUsage` on-chain (defaults: **10k tokens** or **60s** per session, see `[settlement]` in `dev-config/router-launch.toml`), and signs as `recordUsageRole`. On startup (when `[chain] enabled`), the router **loads or generates** a secp256k1 key in `data_dir/record-usage-key.json` (same idea as solo `identity.json`). If `settlement.registry_owner_private_key` is set (registry owner / deployer on local Anvil), it submits **`setRecordUsage(router_address)`** when the on-chain role differs. Solo skips provider `recordUsage` when `[router].enabled` and `[settlement].router_usage_metering = true`.

**Direct node path:** provider `nodeOperator` submits `recordUsage` from the settlement loop.

**Protocol fee:** on settle, **1%** of gross provider payout accrues to `protocolBalances` (treasury); provider net is 99%. Fund router gas from treasury via `withdrawProtocolDot`.

**MVP model pricing:** one network-wide rate via `ModelPriceOracle.defaultPrice` (**10¢ input / 50¢ output per 1 million tokens**), pushed by `sparkl-oracle-model-price` with `MODEL_PRICE_SOURCES=flat`. The model name on `openSession` only identifies the session; price does not vary by model until per-model oracle rows exist (post-MVP).

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

## sparkl-router tunnel (provider registration)

Consumers reach your node through **sparkl-router**, not by dialing your inference port directly. Solo opens an **outbound** WebSocket to the router (`/node/connect`), signs a challenge with the node Ed25519 key, sends **`moniker`** from `[node].moniker` (max 128 chars) on the auth frame, and forwards multiplexed HTTP to the local inference server. The portal reads monikers from router tunnel status (not from on-chain registration). **`GET /identity`** includes `moniker` for local tooling.

**One-command stack:** `./scripts/launch-local.sh` starts sparkl-router and enables the tunnel in `dev-config/launch.toml` (skip with `--skip-router`).

**Manual setup** (`config/default.toml` or `dev-config/*.toml`):

```toml
[router]
enabled = true
url = "ws://127.0.0.1:3001/node/connect"   # or wss:// in production
```

**CLI overrides** (win over TOML): `--router-url`, `--router-enabled`. **Env:** `SPARKLE_ROUTER__URL`, `SPARKLE_ROUTER__ENABLED`.

```bash
# Manual: start router separately (not needed when using launch-local.sh)
cd ../sparkl-router && cp config.example.toml config.toml && cargo run -- config.toml

cargo run --features mock-tpm -- \
  --config dev-config/node1.toml \
  --router-enabled true \
  --router-url ws://127.0.0.1:3001/node/connect
```

**launch-local.sh** generates `dev-config/router-launch.toml` with `chain.enabled = true` and local Anvil contract addresses — register the node on portal `/node/register` before the WSS tunnel is accepted. For ad-hoc testing without registration, set router `[chain] enabled = false` in a custom config.

**Verify:** `GET http://127.0.0.1:3001/v1/models` (aggregated) and `GET /status/nodes` with router admin bearer show the connected node as `online`.

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
  - **Polkadot Hub EVM:** point `settlement.evm_rpc_url` (and `escrow_contract`) at your Hub deployment; optional `registry.registry_contract_address` and `registry.evm_rpc_url` (empty = same settlement RPC); use `SPARKLE__SETTLEMENT__`* / `SPARKLE__REGISTRY__`* overrides as needed
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

## Working with Paseo Testnet

**Asset Hub Paseo** (parachain id 1000) is the testnet home for Sparkl Hub EVM contracts. The JSON-RPC endpoint used here is **Eth Asset Hub** (Hub EVM / `pallet_revive`), not the relay chain.


| Item                                  | Value                                                                                      |
| ------------------------------------- | ------------------------------------------------------------------------------------------ |
| Hub EVM RPC (HTTPS, MetaMask)         | `https://eth-asset-hub-paseo.dotters.network`                                              |
| Hub EVM RPC (WSS, `cast`)             | `wss://eth-asset-hub-paseo.dotters.network`                                                |
| Chain ID                              | `420420417` (`0x190f1b41`)                                                                 |
| Native gas token                      | **PAS** (testnet; no real value)                                                           |
| Block explorer (accounts, extrinsics) | [assethub-paseo.subscan.io](https://assethub-paseo.subscan.io/)                            |
| Faucet                                | [faucet.polkadot.io](https://faucet.polkadot.io/) — Network **Paseo**, Chain **Asset Hub** |
| More RPCs / specs                     | [paseo.site/developers](https://paseo.site/developers)                                     |


Confirm RPC connectivity:

```bash
export PASEO_RPC=https://eth-asset-hub-paseo.dotters.network

cast chain-id --rpc-url "$PASEO_RPC"
cast rpc eth_chainId --rpc-url "$PASEO_RPC"
```

Expected chain id: **420420417**.

### Wallets you need (keep them separate)

Use **different hot keys** for each role. Never commit private keys or `.env` files.


| Role                               | Used for                                                         | Where to configure                                                                                                                                                     |
| ---------------------------------- | ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Deployer** (one-time)            | `forge script … DeployPaseo --broadcast`                         | Shell: `export PRIVATE_KEY=0x…`                                                                                                                                        |
| **Node operator**                  | `registerNode`, registry/escrow txs, TEE `personal_sign`, portal | `settlement.evm_provider_wallet_private_key` in node TOML / `SPARKLE__SETTLEMENT__EVM_PROVIDER_WALLET_PRIVATE_KEY`; must equal `ProviderRegistry.nodeOperator(nodeId)` |
| **Oracle** (`sparkl-oracle-rates`) | `RateSetter.setRate` only                                        | `ORACLE_PRIVATE_KEY` in `sparkl-oracle-rates/.env`; must equal `RateSetter.updater`                                                                                    |
| **Model price oracle** (`sparkl-oracle-model-price`) | `ModelPriceOracle.setModelPrice` | Same `ORACLE_PRIVATE_KEY` / `updater` wallet as rates oracle is fine; configure `sparkl-oracle-model-price/.env` |
| **Attestation** (optional)         | `ProviderRegistry.setTEEProof`                                   | `ATTESTATION_SERVICE` at deploy, or deployer default                                                                                                                   |


#### Create a node operator wallet

```bash
mkdir -p wallet/node-operator-paseo
cast wallet new wallet/node-operator-paseo
Enter password: 
# your choice - you can enter an empty password - NOT RECOMMENDED
# Save your password in your password app!!
```

`cast wallet new <dir>` writes an **encrypted keystore** into that directory (filename = UUID, e.g. `0db77360-1b4e-478b-aa06-b87887c969d8`). The private key is not stored in plain text.

Recover the key (prompts for the password you set at creation):

```bash
cast wallet decrypt-keystore 0db77360-1b4e-478b-aa06-b87887c969d8 \
  --keystore-dir wallet/node-operator-paseo

Enter password: 
0db77360-1b4e-478b-aa06-b87887c969d8's private key is: 0x7d57-redacted-xxxfe7ae4

# !! THIS PRIVATE KEY IS *VERY* SECRET !!
# !! do not share it with anyone !!
# !! do not store it in github !!
```

Address from keystore (same password):

```bash
cast wallet address \
  --keystore wallet/node-operator-paseo/0db77360-1b4e-478b-aa06-b87887c969d8
```

`decrypt-keystore` without `--keystore-dir` only looks under `~/.foundry/keystores/`, which causes “Keystore file does not exist” if your file lives in `wallet/…`.

Save the **address** (`0x…`) and **private key** offline. Wire the key into the solo node:

```toml
# config/your-node.toml (or env override)
[settlement]
evm_rpc_url = "https://eth-asset-hub-paseo.dotters.network"
evm_provider_wallet_private_key = "0x…"
```

Or:

```bash
export SPARKLE__SETTLEMENT__EVM_PROVIDER_WALLET_PRIVATE_KEY=0x…
```

After deploy, **commercially register** on the portal (`/node/register`) so `msg.sender` becomes `**nodeOperator(nodeId)**`. TEE attestation must be signed with the **same** EOA (`personal_sign` on the challenge message). MVP: solo does not call `registerNode` at startup.

Verify the address:

```bash
cast wallet address --private-key "$NODE_OPERATOR_PRIVATE_KEY"
```

#### Create an oracle wallet (`sparkl-oracle-rates`)

```bash
mkdir -p wallet/oracle-updater-paseo
cast wallet new wallet/oracle-updater-paseo
```

Set `**ORACLE_UPDATER_ADDRESS**` to this address when broadcasting `[contracts/script/DeployPaseo.s.sol](./contracts/script/DeployPaseo.s.sol)`, or have the `RateSetter` **owner** call `setUpdater(oracleAddress)` after deploy.

Copy the private key into `**sparkl-oracle-rates/.env`** as `ORACLE_PRIVATE_KEY`. The same wallet can push **`ModelPriceOracle`** via **`sparkl-oracle-model-price`**. Full checklist: [sparkl-oracle-rates/README.md — Oracle wallet](../sparkl-oracle-rates/README.md#oracle-wallet).

Verify updater matches:

```bash
cast wallet address --private-key "$ORACLE_PRIVATE_KEY"
cast call "$RATE_SETTER_ADDRESS" "updater()(address)" --rpc-url "$PASEO_RPC"
```

### Fund accounts with PAS (faucet)

Hub EVM gas is paid in **PAS** on the **Asset Hub** balance tied to your account.

1. Open the [Polkadot Faucet](https://faucet.polkadot.io/) (also linked from [paseo.site/developers](https://paseo.site/developers)).
2. **Network:** **Paseo**.
3. **Chain:** **Asset Hub** (not relay-only; relay PAS does not pay Hub EVM txs).
4. Paste the **destination address** for the wallet you are funding (`0x…` from `cast wallet address`, or the SS58 address shown by a Substrate wallet for the same key).
5. Click **Get some PASs** and wait for confirmation (typically a few thousand PAS per drip; rate limits apply).

Fund **each** role that sends transactions: deployer (deploy), node operator (register / heartbeat / settle), oracle (periodic `setRate`). Re-request from the faucet when balances run low.

Check EVM balance (raw base units; Asset Hub native uses **10** decimals in portal config):

```bash
cast balance 0xYourAddress --rpc-url "$PASEO_RPC"
# human-readable (10 decimals on Hub):
cast --to-unit "$(cast balance 0xYourAddress --rpc-url "$PASEO_RPC")" 10
```

Send PAS between your own EOAs on Hub EVM with `cast send` if one wallet was funded and others were not.

### Check balance and transaction history

**Subscan (recommended for account overview):**

- Open [assethub-paseo.subscan.io](https://assethub-paseo.subscan.io/).
- Search your account (SS58 or, where supported, the linked `0x` address).
- Review **Transfers**, **Extrinsics**, and account balance history on Asset Hub Paseo.

**CLI (quick balance):**

```bash
cast balance 0xYourAddress --rpc-url "$PASEO_RPC"
```

**CLI (recent txs for an address):** use Subscan or your wallet’s activity tab; `cast` does not index history.

### MetaMask and other wallets

#### MetaMask (Hub EVM `0x` address)

1. **Add network** → **Add a network manually**:
  - **Network name:** `Paseo Hub EVM` (or `Eth Asset Hub Paseo`)
  - **RPC URL:** `https://eth-asset-hub-paseo.dotters.network`
  - **Chain ID:** `420420417`
  - **Currency symbol:** `PAS`
  - **Block explorer URL:** `https://assethub-paseo.subscan.io/`
2. **Import account:** paste the **private key** from `cast wallet new` (or import the mnemonic if you saved it).
3. Confirm the address matches `cast wallet address --private-key …` before funding or registering on-chain.

Use this wallet in **[sparkl-portal](https://github.com/sparkl-network/sparkl-portal)** with `NEXT_PUBLIC_CHAIN_ENV=paseo` and RPC/chain id from `[contracts/deployments/paseo.json](./contracts/deployments/paseo.json)` (see portal `.env.example`).

#### Talisman / SubWallet / Polkadot.js

These show the **Substrate (SS58)** view of the same key on Paseo Asset Hub. Useful for faucet UI, Subscan, and teleports. For contract calls (`registerNode`, portal writes), prefer **MetaMask** (or another EIP-1193 wallet) on chain id **420420417**.

#### Security

- Use **dedicated** test hot wallets; do not reuse mainnet or deployer keys for the oracle service.
- `chmod 600` on `.env` files; never commit keys.
- Rotate any key that was pasted into chat or CI logs.


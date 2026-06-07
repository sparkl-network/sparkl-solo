# sparkl-solo — agent guide

**Repository:** [github.com/sparkl-network/sparkl-solo](https://github.com/sparkl-network/sparkl-solo)

Rust **provider node** (`sparkl-solo` binary), **Foundry** Hub EVM contracts, optional **TEE attestation stub**, and JS operational tests. This is the core execution and on-chain definition layer for Sparkl Network.

## Ecosystem position

| Repo | Relationship |
|------|----------------|
| **sparkl-portal** | UI for registry/escrow; syncs ABIs from `contracts/`; probes node HTTP for registration |
| **sparkl-oracle-rates** | Pushes DOT/USD to `RateSetter` deployed by scripts here |
| **sparkl-oracle-model-price** | Pushes model reference prices to `ModelPriceOracle` |
| **sparkl-router** | Consumer API gateway; nodes dial outbound WSS `/node/connect` |
| **Workspace** | Sibling checkout layout — see [../AGENTS.md](../AGENTS.md) |

On-chain: `ProviderRegistry`, `SettlementEscrow`, `RateSetter`, `ModelPriceOracle`, `SparklNetworkConfig` (CREATE2 bootstrap). Off-chain: libp2p + OpenAI-compatible inference proxy.

## Prerequisites

- **Rust** (stable)
- **Foundry** (`forge`, `anvil`, `cast`) for `contracts/`
- **Node.js 20+** and **Yarn** for `tests-js/` and `services/tee-attestation-stub/`
- Optional: **Ollama** or another backend at `backend.url` (default `http://127.0.0.1:11434`)

## Quick start (verify compile + tests)

```bash
cargo build --features mock-tpm
cargo test --features mock-tpm
cd contracts && forge test
cd ../tests-js && yarn install && yarn tpm:suite
```

## Run the solution

### One-command local stack (recommended for agents)

Starts Anvil (if needed; **persists** to `.launch/anvil-state.json` by default), deploys `DeployLocal` when artifacts or chain drift, runs Forge + Rust tests, writes `dev-config/launch.toml` and `dev-config/router-launch.toml`, starts **sparkl-router** and a solo node (unless skipped), prints **portal**, **router**, and **sparkl-oracle-rates** env hints:

```bash
./scripts/launch-local.sh              # full stack until Ctrl+C
./scripts/launch-local.sh --skip-node  # chain + deploy + tests only
./scripts/launch-local.sh --skip-router # no sparkl-router; solo tunnel disabled
./scripts/launch-local.sh --skip-tests # faster iteration
./scripts/launch-local.sh --no-state   # ephemeral Anvil (no state file)
./scripts/launch-local.sh --force-deploy
```

Local MVP: run **sparkl-oracle-rates** for live DOT/USD; model default price is seeded on-chain at launch (no **sparkl-oracle-model-price** service required). Default router uses `chain.enabled = true` — **commercially register** the node on portal `/node/register` before WSS tunnel shows online.

After launch: health `curl http://127.0.0.1:19950/health`, router `curl http://127.0.0.1:3001/health`, deployments in `contracts/deployments/local.json`.

### Manual: two nodes (libp2p discovery)

See **[DEVELOPER.md](./DEVELOPER.md)** — create `dev-config/node1.toml` and `node2.toml`, run:

```bash
cargo run --features mock-tpm -- --config dev-config/node1.toml
cargo run --features mock-tpm -- --config dev-config/node2.toml
```

### Router tunnel (sparkl-router)

Nodes subscribe to **sparkl-router** over outbound WebSocket so consumers can reach inference without inbound ports.

**launch-local.sh** starts the router and sets `[router] enabled = true` in `dev-config/launch.toml` (unless `--skip-router`). Generated router config uses `chain.enabled = true` — register on portal first.

**Manual** (two-node or custom configs):

1. Start router: `cd ../sparkl-router && cargo run -- config.toml` (see router `config.example.toml`), or use `./scripts/launch-local.sh` without `--skip-router`.
2. Enable tunnel on solo — TOML `[router] enabled = true` or CLI:

```bash
cargo run --features mock-tpm -- \
  --config dev-config/node1.toml \
  --router-enabled true \
  --router-url ws://127.0.0.1:3001/node/connect
```

Env override: `SPARKLE_ROUTER__URL`, `SPARKLE_ROUTER__ENABLED`.

3. With `chain.enabled = true`, **commercially register** the node on the portal first, then solo **WSS-subscribes** to the router (`router.enabled` + matching `nodeId` from libp2p peer id). `registry.enabled` is for on-chain heartbeat/TEE only (no startup `registerNode`). Set `chain.enabled = false` in a custom router config to skip the on-chain gate for local mock tests.
4. Verify: `curl -H "Authorization: Bearer $ADMIN_TOKEN" http://127.0.0.1:3001/status/nodes` shows `online`; `curl http://127.0.0.1:3001/v1/models` lists models from connected tunnels.

Session activate (`POST /sessions/{id}/activate` on router) is forwarded as `activate_request`; solo mints deterministic `sk_` bearer keys when built with `--features evm-settlement`.

Integration test: `cargo test --features mock-tpm two_nodes_discover_each_other_with_separate_configs -- --nocapture`

### Contracts only (Anvil)

```bash
anvil --host 0.0.0.0   # terminal 1
cd contracts
forge script script/DeployLocal.s.sol:DeployLocal \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast
```

Addresses: `contracts/deployments/local.json` and script logs. Portal must use these after every Anvil restart.

### Paseo (Hub testnet)

Checklist in **[DEVELOPER.md — Deploy to Paseo](./DEVELOPER.md#deploy-to-paseo-hub-testnet--manual-checklist)** and **`contracts/README.md`**. Output: `contracts/deployments/paseo.json`.

### Hub EVM on the node (`evm-settlement`)

Build with `mock-tpm,evm-settlement`, set `settlement.enabled`, registry addresses (or non-zero `SparklNetworkConfig` in `src/network_config.rs`). Operator wallet must match on-chain `nodeOperator(nodeId)`.

## Tests

| Layer | Command | Location |
|-------|---------|----------|
| Rust unit + integration | `cargo test --features mock-tpm` | `tests/`, `tests/integration_test.rs` |
| TPM feature path | `cargo test --features tpm` | same |
| Solidity | `cd contracts && forge test` | `contracts/test/*.t.sol` |
| Live node ops | `cd tests-js && yarn status` / `yarn attestation` / `yarn encrypted` / `yarn tpm:suite` | [tests-js/README.md](./tests-js/README.md) |
| TEE stub service | `cd services/tee-attestation-stub && yarn install && yarn start` | [services/tee-attestation-stub/README.md](./services/tee-attestation-stub/README.md) |

**Launcher** runs Forge + Rust tests automatically unless `--skip-tests`.

## Configuration (agents)

- Default TOML: `config/default.toml`; override with `--config path.toml` or `SPARKLE__SECTION__KEY` env vars.
- **`nodeId` on-chain:** `bytes32` = `keccak256(libp2p PeerId multihash bytes)` — same as **`GET /identity`** (`identity::on_chain_node_id_*`). **`peer_id`** in JSON is the libp2p `12D3Koo…` string.
- Config overrides: `SPARKLE__NETWORK__INFERENCE_PORT=19944`, etc. (see DEVELOPER.md).
- Local dev: use **`mock-tpm`**; do not commit `dev-data/**/network/secret_ed25519` or operator keys.

## High-signal code map

| Path | Purpose |
|------|---------|
| `src/server/inference.rs` | Inference hot path |
| `src/server/mod.rs` | HTTP route registration |
| `src/receipts.rs` | Signing / verification (consensus-critical) |
| `src/identity.rs` | Keys; TPM gates |
| `src/registry.rs` | Hub registry client (stub — extend per [docs/MVP_ROADMAP.md](./docs/MVP_ROADMAP.md)) |
| `src/settlement/` | Epoch loop; `evm.rs` with `evm-settlement` |
| `contracts/src/` | `ProviderRegistry`, `SettlementEscrow`, `RateSetter`, `ModelPriceOracle`, … |
| `scripts/launch-local.sh` | Local Anvil + deploy + sparkl-router + solo node + portal/router/oracle hints |

Priorities and gaps: **[docs/MVP_ROADMAP.md](./docs/MVP_ROADMAP.md)**.

## Contributing

1. Read **MVP_ROADMAP.md** for P0/P1 scope.
2. Implement focused changes; match existing Rust/Solidity style.
3. Run: `cargo test --features mock-tpm`, `forge test`, and relevant `tests-js` scripts.
4. Update **DEVELOPER.md**, **contracts/README.md**, or **tests-js/README.md** when behavior changes.
5. Open a PR on `sparkl-network/sparkl-solo` with summary + test output.

**Do not touch without explicit request:** `Cargo.lock` (let Cargo manage).

**Never commit:** `identity-secret.json`, `services/tee-attestation-stub/.env`, deployer/oracle private keys.

## Related documentation

- **[DEVELOPER.md](./DEVELOPER.md)** — multi-node dev, Paseo deploy, TEE onboarding, SparklNetworkConfig bootstrap
- **[contracts/README.md](./contracts/README.md)** — Foundry layout, Anvil deploy, ABI sync to portal
- **[README.md](./README.md)** — product overview and contribute checklist
- **[AGENTS.md](./AGENTS.md)** — short alias pointing here (legacy filename)

Sibling repos: **[sparkl-portal/AGENTS.md](https://github.com/sparkl-network/sparkl-portal/blob/main/AGENTS.md)** · **[sparkl-oracle-rates/AGENTS.md](https://github.com/sparkl-network/sparkl-oracle-rates/blob/main/AGENTS.md)** · **[sparkl-oracle-model-price/AGENTS.md](../sparkl-oracle-model-price/AGENTS.md)**

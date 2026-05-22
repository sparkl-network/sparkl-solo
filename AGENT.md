# sparkl-solo — agent guide

**Repository:** [github.com/sparkl-network/sparkl-solo](https://github.com/sparkl-network/sparkl-solo)

Rust **provider node** (`sparkl-solo` binary), **Foundry** Hub EVM contracts, optional **TEE attestation stub**, and JS operational tests. This is the core execution and on-chain definition layer for Sparkl Network.

## Ecosystem position

| Repo | Relationship |
|------|----------------|
| **sparkl-portal** | UI for registry/escrow; syncs ABIs from `contracts/`; probes node HTTP for registration |
| **sparkl-oracle-rates** | Pushes DOT/USD to `RateSetter` deployed by scripts here |
| **Workspace** | Sibling checkout layout — see [../AGENT.md](../AGENT.md) |

On-chain: `ProviderRegistry`, `SettlementEscrow`, `RateSetter`, `SparklNetworkConfig` (CREATE2 bootstrap). Off-chain: libp2p + OpenAI-compatible inference proxy.

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

Starts Anvil (if needed), deploys `DeployLocal`, runs Forge + Rust tests, writes `dev-config/launch.toml`, starts a node, prints **oracle** and **portal** env hints:

```bash
./scripts/launch-local.sh              # full stack until Ctrl+C
./scripts/launch-local.sh --skip-node  # chain + deploy + tests only
./scripts/launch-local.sh --skip-tests # faster iteration
```

After launch: health `curl http://127.0.0.1:19950/health`, deployments in `contracts/deployments/local.json`.

### Manual: two nodes (libp2p discovery)

See **[DEVELOPER.md](./DEVELOPER.md)** — create `dev-config/node1.toml` and `node2.toml`, run:

```bash
cargo run --features mock-tpm -- --config dev-config/node1.toml
cargo run --features mock-tpm -- --config dev-config/node2.toml
```

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
- **`nodeId` on-chain:** `bytes32` = `keccak256(ed25519_pubkey)` — same as **`GET /identity`** (`identity::on_chain_node_id_*`). Do not use other hashes for registry/escrow ids.
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
| `contracts/src/` | `ProviderRegistry`, `SettlementEscrow`, `RateSetter`, … |
| `scripts/launch-local.sh` | Local Anvil + deploy + node + oracle hints |

Priorities and gaps: **[docs/MVP_ROADMAP.md](./docs/MVP_ROADMAP.md)**.

## Contributing

1. Read **MVP_ROADMAP.md** for P0/P1 scope.
2. Implement focused changes; match existing Rust/Solidity style.
3. Run: `cargo test --features mock-tpm`, `forge test`, and relevant `tests-js` scripts.
4. Update **DEVELOPER.md**, **contracts/README.md**, or **tests-js/README.md** when behavior changes.
5. Open a PR on `sparkl-network/sparkl-solo` with summary + test output.

**Do not touch without explicit request:** `Cargo.lock` (let Cargo manage), `config/default.toml` pricing defaults (tests depend on them).

**Never commit:** `identity-secret.json`, `services/tee-attestation-stub/.env`, deployer/oracle private keys.

## Related documentation

- **[DEVELOPER.md](./DEVELOPER.md)** — multi-node dev, Paseo deploy, TEE onboarding, SparklNetworkConfig bootstrap
- **[contracts/README.md](./contracts/README.md)** — Foundry layout, Anvil deploy, ABI sync to portal
- **[README.md](./README.md)** — product overview and contribute checklist
- **[AGENTS.md](./AGENTS.md)** — short alias pointing here (legacy filename)

Sibling repos: **[sparkl-portal/AGENT.md](https://github.com/sparkl-network/sparkl-portal/blob/main/AGENT.md)** · **[sparkl-oracle-rates/AGENT.md](https://github.com/sparkl-network/sparkl-oracle-rates/blob/main/AGENT.md)**

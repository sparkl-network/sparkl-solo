# sparkl-solo prototype

Rust prototype for the `sparkl-solo` binary.

## What is Sparkl

Sparkl is a decentralized private AI inference network for routing model requests across independent provider nodes.

- see also [sparkl-portal](https://github.com/sparkl-network/sparkl-portal) for the portal/UI

## Target architecture

**Goals:** payments and escrow on **Polkadot Hub EVM** (`pallet_revive`) using **DOT** first and **USDC** later (Hub **ERC-20 precompile**). Two provider tiers: **Tier A** (TEE-verified, confidential / verifiable inference) and **Tier B** (best-effort, cheaper). Consumers choose the tier; pricing may differ by tier.

**On-chain:** `SettlementEscrow`, `ProviderRegistry` (payouts, per-tier pricing, TEE flags / evidence hash), and `IPriceOracle` (USDC↔DOT; DIA for MVP, Pyth later) — see [`contracts/`](./contracts/).

**Off-chain:** an attestation service that verifies TEE quotes and writes **TEE verified** on-chain (MVP stub: [`services/tee-attestation-stub/`](./services/tee-attestation-stub/README.md)); aggregators that route by tier and price.

**Provider nodes:** Tier A on SGX/TDX/SEV/TrustZone/Nitro-class hardware; Tier B on any GPU/CPU platform.

**PAOW:** Proven Attestation of Work, a proof of work that is used to verify the work of a provider.
- see [`docs/TRUST.md`](./docs/TRUST.md) for more details

Legacy Unicity-oriented options remain in config for transitional use; new integration work should target Hub EVM.

The `sparkl-solo` node implements:

- OpenAI-compatible inference proxying (`/v1/chat/completions`, `/v1/models`)
- Streaming chunk receipts with provider signatures
- libp2p-based peer discovery
- local mock/TPM-oriented attestation and receipt verification endpoints

### What is solo?

A solo node is a single node that runs on a single machine. It is the simplest form of a Sparkl node.
Planned: farm or gateway node, for running one sparkl-node and multiple provider nodes

## Quick Start (Agent)
MUST: Read the AGENTS.md!
```bash
cargo build --features mock-tpm          # verify it compiles
cargo test --features mock-tpm           # unit + integration tests
cd tests-js && yarn install              # install JS harness
yarn tpm:suite                           # end-to-end 2-node test
```

## Development

- `cargo check --features mock-tpm`
- `cargo test --features mock-tpm`
- `cargo run --features mock-tpm`

Config defaults are in `config/default.toml`, and can be overridden with env vars prefixed by `SPARKLE__`.

## Configuration Overrides

You can override config with either CLI flags or environment variables.

- **CLI precedence:** CLI flags override file values loaded from `--config`.
- **Env var prefix:** `SPARKLE__SECTION__KEY`
  - example: `SPARKLE__NETWORK__INFERENCE_PORT=19944`
- **Arrays via env vars:** use JSON array strings
  - example: `SPARKLE__NETWORK__PUBLIC_ADDR='["/ip4/1.2.3.4/tcp/30333"]'`

Unicity anchoring (`--features unicity`) is **legacy / optional**: it uses `registry.unicity_aggregator_url` as the JSON-RPC base URL. Override with `SPARKLE__REGISTRY__UNICITY_AGGREGATOR_URL` (and optional `SPARKLE__REGISTRY__UNICITY_API_KEY`). **Primary settlement direction:** Polkadot Hub EVM — set `settlement.evm_rpc_url` and `settlement.escrow_contract` to your Hub deployment.

```bash
SPARKLE__REGISTRY__UNICITY_AGGREGATOR_URL=https://goggregator-test.unicity.network/ \
  cargo run --features unicity
```

Common CLI examples:

- `--config dev-config/node1.toml`
- `--receipt-cadence 50`
- `--include-models qwen/qwen3.5-9b,meta-llama/llama-3.1-8b-instruct`
- `--exclude-models internal/research-model`
- `--public-addr /ip4/203.0.113.10/tcp/30333,/ip4/203.0.113.10/tcp/443/ws`
- `--allow-non-globals-in-dht false`

## JavaScript Operational Tests

Use the `tests-js/` harness for periodic runtime checks against live nodes:

- `cd tests-js && yarn install`
- `yarn status`
- `yarn attestation`
- `yarn encrypted`
- `yarn tpm:suite`

## Roadmap

The **MVP roadmap** (goals, gap analysis, prioritized deliverables, dependency graph, risks, and pointers to narrative docs) lives in **[docs/MVP_ROADMAP.md](docs/MVP_ROADMAP.md)**. Long-form product narratives: `docs/Sparkle  Decentralised Private AI Inference for NVIDIA DGX Spark Owners.md`, `docs/Sparkle Tokenomics  SPARKL Token Design and Network Economics.md`, and `docs/TPM-development.md`.

## How to contribute

1. Fork the repository and create a feature branch.
2. Implement your change with focused commits.
3. Run local checks before opening a PR:
   - `cargo test --features mock-tpm`
   - `cargo test --features tpm`
   - `cd tests-js && yarn status && yarn attestation && yarn encrypted && yarn tpm:suite`
4. Update relevant docs (`DEVELOPER.md`, `docs/MVP_ROADMAP.md`, `docs/TPM-development.md`, and `tests-js/README.md`) when behavior changes.
5. Open a pull request with:
   - a short summary of the change
   - test evidence (command output or screenshots where relevant)
   - any follow-up work items

# Qudos and Shouts

- [darkbloom.dev](https://darkbloom.dev) - darkbloom is a great project and provided some inspiration for the architecture and approach
- Nvidia DGX Spark - a very capable and piece of hardware, this project is a great way to put it to use

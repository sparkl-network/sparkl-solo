# sparkl-solo prototype

Rust prototype for the `sparkl-solo` binary.

## What is Sparkl

Sparkl is a decentralized private AI inference network for routing model requests across independent provider nodes.  
The `sparkl-solo` node implements:

- OpenAI-compatible inference proxying (`/v1/chat/completions`, `/v1/models`)
- Streaming chunk receipts with provider signatures
- libp2p-based peer discovery
- local mock/TPM-oriented attestation and receipt verification endpoints

### What is solo?

A solo node is a single node that runs on a single machine. It is the simplest form of a Sparkl node.
Planned: farm or gateway node, for running one sparkl-node and multiple provider nodes

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

Progress is mapped to the multi-phase plan in `docs/Sparkle  Decentralised Private AI Inference for NVIDIA DGX Spark Owners.md`, `docs/Sparkle Tokenomics  SPARKL Token Design and Network Economics.md`, and `docs/TPM-development.md`.

### Phase 1 - Core MVP (mostly complete in this repo)

- [x] OpenAI-compatible node endpoints: `/v1/chat/completions` and `/v1/models`
- [x] Streaming SSE proxy path with signed chunk receipts
- [x] Local two-node dev configs and discovery test flow
- [x] Rust integration tests and JS operational test harness (`tests-js/`)
- [x] TPM development path validated with `swtpm` workflows and challenge/verify APIs

### Phase 2 - Privacy and decentralization foundations (in progress)

- [x] Encrypted request handling (`epk` + `ciphertext`) in inference path
- [x] Receipt verification endpoint (`/receipts/verify`) and attestation challenge endpoint (`/attestation/challenge`)
- [x] Configurable model visibility controls (`include_models` then `exclude_models`)
- [x] Session accounting now uses real token pricing (`amount_micro_usd`)
- [x] Settlement epoch batches now compute a receipts root (non-zero placeholder removed)
- [x] Session completion logs include tokens, receipts, earnings, and duration
- [ ] Live NRAS verification and production attestation certificate flow
- [ ] Unicity registry heartbeats/state transitions beyond local stub behavior
- [ ] On-chain escrow/payment settlement integration beyond disabled local config

### Phase 3 - P2P network hardening (in progress)

- [x] Real `libp2p` swarm with Identify/Ping/mDNS/Kademlia
- [x] Persistent peer identity on disk across restarts
- [x] Protocol-aware peer filtering (`sparkl/*`) for known-peer reporting
- [x] Recovery test coverage for active sessions after restart
- [ ] Wider multi-peer interoperability validation beyond current local/test-peer runs
- [ ] SDK-level trustless discovery and direct consumer P2P flows

### Operations and delivery

- [x] `SETUP.md` production runbook (config + port forwarding)
- [x] Expanded macOS TPM guidance in `DEVELOPER.md` and `docs/TPM-development.md`
- [ ] GitHub Actions CI workflow activation (requires token/repo permission for workflow updates)

### TBC
- Oracle design for pricing
- Integration with Unicity for state transition and proof generation
- Integration with ZKP layer for Double-spend protection
- Dashboard and UI
- Substrate solo chain
- quic-ws support
  - Per-stream loss recovery — one stalled token doesn't affect other concurrent sessions
  - 0-RTT resumption — meaningful for short-context requests where connection setup is a significant fraction of latency
  - Native multiplexing — multiple concurrent streaming sessions over one QUIC connection

## How to contribute

1. Fork the repository and create a feature branch.
2. Implement your change with focused commits.
3. Run local checks before opening a PR:
   - `cargo test --features mock-tpm`
   - `cargo test --features tpm`
   - `cd tests-js && yarn status && yarn attestation && yarn encrypted && yarn tpm:suite`
4. Update relevant docs (`DEVELOPER.md`, `docs/TPM-development.md`, and `tests-js/README.md`) when behavior changes.
5. Open a pull request with:
   - a short summary of the change
   - test evidence (command output or screenshots where relevant)
   - any follow-up work items

# Qudos and Shouts

- [darkbloom.dev](https://darkbloom.dev) - darkbloom is a great project and provided some inspiration for the architecture and approach
- Nvidia DGX Spark - a very capable and piece of hardware, this project is a great way to put it to use

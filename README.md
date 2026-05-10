# sparkl-solo prototype

Rust prototype for the `sparkl-solo` binary.

## What is Sparkl

Sparkl is a decentralized private AI inference network for routing model requests across independent provider nodes.  
The `sparkl-solo` node implements:

- OpenAI-compatible inference proxying (`/v1/chat/completions`, `/v1/models`)
- Streaming chunk receipts with provider signatures
- libp2p-based peer discovery
- local mock/TPM-oriented attestation and receipt verification endpoints

## Development

- `cargo check --features mock-tpm`
- `cargo test --features mock-tpm`
- `cargo run --features mock-tpm`

Config defaults are in `config/default.toml`, and can be overridden with env vars prefixed by `SPARKLE__`.

## JavaScript Operational Tests

Use the `tests-js/` harness for periodic runtime checks against live nodes:

- `cd tests-js && yarn install`
- `yarn status`
- `yarn attestation`
- `yarn encrypted`
- `yarn tpm:suite`

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

# AGENTS.md

Instructions for humans and coding agents working on **sparkl-solo**. Primary integration target is **Polkadot Hub EVM** (`SettlementEscrow`, `ProviderRegistry`). New work should align with **[docs/MVP_ROADMAP.md](docs/MVP_ROADMAP.md)** (priorities, gap analysis, and dependency order).

## How to contribute

1. **Read** `docs/MVP_ROADMAP.md` for P0/P1 scope and what is already implemented vs stubbed.
2. **Implement** features or **fix** defects in focused changes; match existing style and keep diffs minimal.
3. **Verify** with the build and test commands below. **Add or extend test cases** where it helps: Rust integration (`tests/integration_test.rs` and new `tests/*.rs` if appropriate), Solidity (`contracts/test/*.sol`), and/or JS harness (`tests-js/`). New tests are welcome in PRs when they lock in behavior or reproduce a fixed defect.
4. **Open a pull request** with a short summary, test evidence (e.g. `cargo test` / `forge test` / `yarn …` output), and any follow-ups called out in the roadmap.

For repository conventions and review expectations, see **[README.md — How to contribute](README.md#how-to-contribute)**.

## Build commands

- `cargo build --features mock-tpm`
- `cargo build --features mock-tpm,evm-settlement`
- `cargo test --features mock-tpm`
- `cargo test --features tpm`
- TEE attestation stub (Node): `cd services/tee-attestation-stub && yarn install && yarn start`

## Test commands (JS integration suite)

- `cd tests-js && yarn install && yarn tpm:suite`

**Where tests live**

- Rust: `tests/integration_test.rs` (and `cargo test` from repo root)
- Solidity: `cd contracts && forge test`
- JS harness: `tests-js/` (see `tests-js/README.md`)

## Local data directory

- `./dev-data/<node-name>/network/` — preserve libp2p peer identity when possible
- `./dev-config/<node-name>.toml` — multi-node configs

## Important constraints

- **Never** commit `identity-secret.json` (private keys).
- **Never** commit `services/tee-attestation-stub/.env` (`ADMIN_PRIVATE_KEY`).
- **Never** change receipt signing without updating `tests/integration_test.rs`.
- Hub EVM settlement: `src/settlement/evm.rs` behind `--features evm-settlement` (provider key + settlement operator key must match on-chain `settlementOperator` when `settlement.enabled`).
- Hub EVM registry (`[registry]`): **`registry_contract_address`** and optional **`evm_rpc_url`** (empty = use **`settlement.evm_rpc_url`**). Operator signing uses **`settlement.evm_provider_wallet_private_key`** for both registry and escrow provider calls.
- `identity.rs` has a `#[cfg(feature = "tpm")]` path — when touching identity, validate `mock-tpm` (and `tpm` if relevant).

## Architecture map

**Hub EVM `nodeId` (`bytes32`):** **`keccak256(ed25519_pubkey)`** — single canonical derivation in **`identity::on_chain_node_id_bytes`** / **`on_chain_node_id_hex`**. **`GET /identity`** uses the same rule. Do not use SHA256(x25519), libp2p multihash digests, or other hashes for registry/escrow `nodeId`.

**On-chain:** Polkadot Hub EVM — `SettlementEscrow`, `ProviderRegistry`, `IPriceOracle` (see `contracts/`). Tier A = TEE-verified (attestation flow writes proof on-chain); Tier B = best-effort.

**Off-chain:** Attestation service and routing/aggregators. **MVP TEE stub:** [`services/tee-attestation-stub/README.md`](./services/tee-attestation-stub/README.md) (`GET /v1/challenge`, `POST /v1/attest` → on-chain [`ProviderRegistry.setTEEProof`](./contracts/src/ProviderRegistry.sol)); stub `ADMIN_PRIVATE_KEY` must match registry `attestationService` when testing that path.

**Rust layout (high-signal files):**

- `src/server/inference.rs` — inference hot path; change carefully
- `src/server/mod.rs` — HTTP routes (new handlers registered here)
- `src/session.rs` — session lifecycle and pricing
- `src/receipts.rs` — signing and verification (consensus-critical)
- `src/identity.rs` — key management; TPM-related gates
- `src/registry.rs` — hub registry client is **stubbed**; safe to extend with real `ProviderRegistry` calls (see roadmap §1.3)
- `src/settlement/` — epoch loop; `evm.rs` sends escrow txs when `--features evm-settlement`

## When adding a new HTTP endpoint

1. Add handler under `src/server/`.
2. Register the route in `src/server/mod.rs`.
3. Add coverage: Rust integration and/or `tests-js/` for user-facing paths; update `tests-js/README.md` if you add scripts.

## Do not touch (unless explicitly asked)

- `Cargo.lock` — let Cargo update it
- `config/default.toml` pricing values — relied on by integration tests

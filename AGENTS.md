# AGENTS.md

## Build commands
- `cargo build --features mock-tpm`
- `cargo test --features mock-tpm`
- `cargo test --features tpm`

## Test commands (JS integration suite)
- `cd tests-js && yarn install && yarn tpm:suite`

## Local data directory
- ./dev-data/<node-name>/network/ - try to preserve libp2p peer identity
- ./dev-config/<node-name>.toml - config for multiple nodes

## Important constraints
- NEVER commit identity-secret.json (contains private keys)
- NEVER change receipt signing without updating tests/integration_test.rs
- settlement.rs is a stub — hub EVM wiring should land behind an explicit feature flag / reviewed integration
- identity.rs has a #[cfg(feature = "tpm")] gate — always test both features

## Architecture map

**Target (on-chain):** Polkadot Hub EVM (`pallet_revive`) — `SettlementEscrow`, `ProviderRegistry`, `IPriceOracle` (see `contracts/`). Tier A = TEE-verified (attestation service writes proof on-chain); Tier B = best-effort.

**Target (off-chain):** attestation service, aggregators (tier + price routing).

**Legacy / optional:** Unicity JSON-RPC when built with `--features unicity`.

- src/server/inference.rs — the hot path, touch carefully
- src/session.rs — session lifecycle, pricing lives here
- src/receipts.rs — signing/verification, consensus-critical
- src/receipts.rs unicity_request_id() — derives Unicity commitment ID from receipt (legacy)
- Unicity JSON-RPC: `registry.unicity_aggregator_url` as POST base URL (`--features unicity`); optional `registry.unicity_api_key` as `X-API-Key`
- Feature flag: --features unicity (enables async submit_commitment calls)
- Do NOT await Unicity submission in the inference hot path — fire-and-forget via tokio::spawn
- src/identity.rs — key management, TPM gate here
- src/registry.rs — STUB, safe to modify
- src/settlement.rs — STUB, safe to modify

## When adding a new endpoint
1. Add handler in src/server/
2. Register route in src/server/mod.rs
3. Add a JS test in tests-js/src/
4. Update tests-js/README.md

## Do not touch
- Cargo.lock (let cargo manage)
- config/default.toml pricing values (used by integration tests)

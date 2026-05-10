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
- settlement.rs is a stub — do not wire EVM calls without a new feature flag
- identity.rs has a #[cfg(feature = "tpm")] gate — always test both features

## Architecture map
- src/server/inference.rs — the hot path, touch carefully
- src/session.rs — session lifecycle, pricing lives here
- src/receipts.rs — signing/verification, consensus-critical
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

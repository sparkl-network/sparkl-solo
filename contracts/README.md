# Sparkl Hub EVM contracts

Solidity contracts for **Polkadot Hub EVM** (`pallet_revive`): provider registry, settlement escrow, and price oracles. Layout follows [Foundry](https://book.getfoundry.sh/).

For how these pieces fit the wider node and architecture, see the repo root [DEVELOPER.md](../DEVELOPER.md).

## Prerequisites

- [Foundry](https://book.getfoundry.sh/getting-started/installation) (`forge`, `anvil`, `cast`), **or** the official Docker image `ghcr.io/foundry-rs/foundry` with `/bin/bash` entrypoint overrides as needed.

## One-time setup

From this directory (`sparkl-solo/contracts/`):

```bash
forge install --no-git foundry-rs/forge-std
```

If `lib/forge-std` is already present, skip this step.

## Layout

| Path | Purpose |
|------|---------|
| `src/` | Production contracts (`ProviderRegistry`, `SettlementEscrow`, `DIAPriceOracle`, etc.) |
| `src/interfaces/` | `IPriceOracle`, `IProviderRegistry`, `IERC20`, `IDIAOracle` |
| `src/mocks/` | `MockOracle`, `MockERC20` (tests / local deploys) |
| `test/` | Forge tests (`*.t.sol`) |
| `script/` | `DeployLocal.s.sol` (Anvil), `DeploySparklBase.sol` (shared deployment), `DeployPaseo.s.sol` (Paseo Hub EVM testnet) |
| `deployments/` | Written by `DeployPaseo` (default `deployments/paseo.json`) |
| `foundry.toml` | Solidity `0.8.28`, `src`, `lib`, optimizer, filesystem allowlist for deployments JSON |

## Build

```bash
forge build
```

## Tests

Run the full suite:

```bash
forge test
```

Verbose traces (e.g. debugging a failure):

```bash
forge test -vvv
```

Using Docker (no local Foundry install):

```bash
docker run --rm -v "$PWD":/work -w /work --entrypoint forge ghcr.io/foundry-rs/foundry:latest test
```

Tests cover `ProviderRegistry`, `SettlementEscrow`, USDC→internal DOT conversion via **`getUsdcPerDot()`** / **`getDotForUsdc()`**, and mock ERC20 flows.

## Local chain: Anvil + deploy script

1. Start Anvil (default `http://127.0.0.1:8545`, chain id `31337`):

   ```bash
   anvil
   ```

2. In another terminal, from `contracts/`:

   ```bash
   forge script script/DeployLocal.s.sol:DeployLocal \
     --rpc-url http://127.0.0.1:8545 \
     --broadcast
   ```

   By default the script uses Anvil’s first test private key; override with `PRIVATE_KEY` in the environment if needed.

3. Console output lists deployed addresses: `MockOracle`, `MockERC20`, `ProviderRegistry`, `SettlementEscrow`.

Docker example (Anvil + script in one shot is possible but fragile; two terminals or `anvil` in background is clearer.)

## Paseo testnet (Hub EVM)

Deploy mocks plus `ProviderRegistry` and `SettlementEscrow` against a Paseo JSON-RPC endpoint. **`PRIVATE_KEY` is required** (hex, no committed keys). Overrides:

| Env | Meaning |
|-----|---------|
| `PRIVATE_KEY` | Deployer/signing key (required) |
| `ATTESTATION_SERVICE` | Optional; defaults to deployer (`ProviderRegistry` `setTEEProof` caller) |
| `DEPLOYMENTS_OUT` | Optional JSON path relative to `contracts/`; default `deployments/paseo.json` |

```bash
cd contracts

export PRIVATE_KEY=0x...      # funded account on Paseo Hub EVM
export PASEO_RPC=https://...  # Official Paseo / Polkadot Hub EVM revive JSON-RPC

forge script script/DeployPaseo.s.sol:DeployPaseo \
  --rpc-url "$PASEO_RPC" \
  --broadcast

# Verification: append when Hub EVM block explorer + Forge verifier are wired, e.g.:
#   --verify --etherscan-api-key YOUR_KEY ...
```

After broadcast, inspect `contracts/deployments/paseo.json` for addresses. Manual checklist for operators: see [DEVELOPER.md](../DEVELOPER.md) (**Deploy to Paseo**).

## Configuration notes

- **Oracle:** Primary spot quote is **USDC (6‑dec) smallest units per 1e18 internal DOT** (`IPriceOracle.getUsdcPerDot`). `SettlementEscrow.depositUsdcAsDot` credits internal DOT as `usdcAmount * 1e18 / usdcPerDot`. MVP mocks and deploy scripts baseline **`getUsdcPerDot() = 1_340_000`** (≈ **1.34 USD** per DOT at par USDC ≈ USD).
- **Native DOT** in escrow uses **10** Planck-style decimals on-chain vs **18** decimals for internal accounting; see `SettlementEscrow` helpers.

## Security review

Operational notes, reentrancy / access-control / oracle checklist, and how to run **Slither** (and optional **Mythril**): **[SECURITY.md](./SECURITY.md)**.

## Clean artifacts

```bash
forge clean
```

Removes `out/` and `cache/` (see `.gitignore`).

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

### ABI sync (Rust node + portal)

After changing contracts, copy the built artifacts so **off-chain consumers stay on the same ABI**:

- **Rust (`sparkl-solo`):** from `contracts/` after `forge build`, update `../abi/` from Forge output, for example:
  - `cp out/ProviderRegistry.sol/ProviderRegistry.json ../abi/ProviderRegistry.json`
  - `cp out/SettlementEscrow.sol/SettlementEscrow.json ../abi/SettlementEscrow.json`
  (Exact paths under `out/` match your contract file names.)

- **`sparkl-portal`:** copy the same JSON files into `sparkl-portal/lib/abi/` (or your sibling checkout) so the Next app and `sparkl-solo` never drift.

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
   anvil --host 0.0.0.0
   ```

2. In another terminal, from `contracts/`:

   ```bash
   forge script script/DeployLocal.s.sol:DeployLocal \
     --rpc-url http://127.0.0.1:8545 \
     --broadcast
   ```

   ```bash
   [⠊] Compiling...
No files changed, compilation skipped
Script ran successfully.

== Logs ==
  deployer 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
  MockOracle 0x5FbDB2315678afecb367f032d93F642f64180aa3
  MockERC20 USDC 0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
  ProviderRegistry 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9
  SettlementEscrow 0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9

## Setting up 1 EVM.

==========================

Chain 31337

Estimated gas price: 2.000000001 gwei

Estimated total gas used for script: 4723038

Estimated amount required: 0.009446076004723038 ETH

==========================

##### anvil-hardhat
✅  [Success] Hash: 0x35298b0e8b9567280fccfd162bd850bd05d87b276f993929e528b783f1b16653
Contract: ProviderRegistry
Contract Address: 0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
Block: 3
Paid: 0.000315221805230332 ETH (410839 gas * 0.767263588 gwei)


##### anvil-hardhat
✅  [Success] Hash: 0x7fe15c03a9b86566163aaebec51ed7b53ea3f951dfbe9b4be2594b3ed54053ec
Contract: MockERC20
Contract Address: 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9
Block: 3
Paid: 0.00116007185451248 ETH (1511960 gas * 0.767263588 gwei)


##### anvil-hardhat
✅  [Success] Hash: 0x364f9fdc93c1290f108d5a3e318a4556269c0e3d5705bbc6ac763c25ca2ae606
Contract: MockOracle
Contract Address: 0x5FbDB2315678afecb367f032d93F642f64180aa3
Block: 1
Paid: 0.000158953000158953 ETH (158953 gas * 1.000000001 gwei)


##### anvil-hardhat
✅  [Success] Hash: 0xd44538702e42e34b09b30b71aa0e5363313e881017e7e62c17ee112a94ac8b9c
Contract: SettlementEscrow
Contract Address: 0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9
Block: 4
Paid: 0.00101359963477302 ETH (1482630 gas * 0.683649754 gwei)


##### anvil-hardhat
✅  [Success] Hash: 0x9938fc4ab71245f0a3a087469ef036514a038d18855e1f3ca7d7970d91eb29de
Contract: MockOracle
Function: set(uint256)
Block: 2
Paid: 0.00005754648448948 ETH (65668 gas * 0.87632461 gwei)

✅ Sequence #1 on anvil-hardhat | Total Paid: 0.002705392779164265 ETH (3630050 gas * avg 0.818900308 gwei)
                                                                                           

==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

Transactions saved to: /home/derek/sparkl-solo/contracts/broadcast/DeployLocal.s.sol/31337/run-latest.json

Sensitive values saved to: /home/derek/sparkl-solo/contracts/cache/DeployLocal.s.sol/31337/run-latest.json
   ```

  ## sparkl-portal .env
  ### ProviderRegistry Contract Address
  ```env
  NEXT_PUBLIC_PROVIDER_REGISTRY_ADDRESS_<ENV>=0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
  ```
  ### SettlementEscrow Contract Address
  ```env
  NEXT_PUBLIC_SETTLEMENT_ESCROW_ADDRESS_<ENV>=0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9
  ```

   By default the script uses Anvil’s first test private key; override with `PRIVATE_KEY` in the environment if needed.

3. Console output lists deployed addresses: `MockOracle`, `MockERC20`, `ProviderRegistry`, `SettlementEscrow` (escrow uses **`nativeDotDecimals = 18`** so `depositDot` matches Anvil wei / ETH display).

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
- **Native DOT** in escrow: constructor **`nativeDotDecimals`** — **`10`** on Polkadot Asset Hub (Planck), **`18`** for **`DeployLocal`** on Anvil (wei). Internal balances always use **18** decimals per whole DOT; see `SettlementEscrow` helpers.

## Security review

Operational notes, reentrancy / access-control / oracle checklist, and how to run **Slither** (and optional **Mythril**): **[SECURITY.md](./SECURITY.md)**.

## Clean artifacts

```bash
forge clean
```

Removes `out/` and `cache/` (see `.gitignore`).

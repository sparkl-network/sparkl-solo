# Security notes (Hub EVM contracts)

This document captures **manual review targets**, **tooling baselines**, and **static-analysis summary** for [`src/`](./src/). It is not a formal audit.

## Tooling and baselines

| Step | Command / artifact |
|------|-------------------|
| Format | `forge fmt` (from `contracts/`) |
| Gas snapshot | `forge snapshot` → committed [`.gas-snapshot`](./.gas-snapshot) |
| Slither (local) | `python3 -m venv .venv-sa && . .venv-sa/bin/activate && pip install slither-analyzer && slither .` |
| Slither JSON (optional) | `slither . --json slither-report.json` (add `slither-report.json` to `.gitignore` if not committed) |

**Mythril:** Full `mythril` install failed in a clean Python 3.12 venv on this host (legacy `numpy` build). For ad-hoc runs use a container, e.g.:

```bash
docker run --rm -v "$PWD":/work -w /work mythril/myth myth analyze src/SettlementEscrow.sol
```

(Reality of images and Solidity version wiring may vary; treat as optional.)

**Last updated:** Slither **0.11.5** run on the repository state that includes the CEI tweak in `_depositUsdcAsDot` (oracle read and `credited` computed **before** `transferFrom`).

---

## 1. Reentrancy — `settle*` / `withdraw`

### `settlePartial` / `settleFull` → `_releaseSessionFunds`

- **No external calls.** State updates (`lockedInternal`, `totalLockedInternal`, `providerBalances`, `dotBalances`, `settled`) and `emit` only.
- **Assessment:** Not vulnerable to classic reentrancy via callee hooks.

### `withdrawDot` / `withdrawProviderDot`

- **Pattern:** *Checks → effects → interaction* for **balances**: `dotBalances` / `providerBalances` and `internalCirculating` are decremented **before** `msg.sender.call{value: native}("")`.
- **Slither** may still flag **reentrancy-events** (event after external call). That is **informational**: a reentrant call cannot double-withdraw the same internal units because accounting is already finalized.
- **Residual risk:** Malicious recipient could re-enter other **public** functions (e.g. `depositDot`, `openSession`). Callers should treat withdrawals as interactions with untrusted addresses. A **nonReentrant** guard could be added for defense-in-depth on the whole contract if product requirements allow the extra gas.

### `depositUsdcAsDot` → `_depositUsdcAsDot`

- **Hardening applied:** Staleness check (if enabled), **`getUsdcPerDot`** and **`credited`** computation occur **before** `usdc.transferFrom`, so a malicious ERC-20 hook cannot steer the oracle read used for **this** deposit’s credit (Hub USDC/precompile assumed non-malicious).
- **Slither** may still report **reentrancy-benign** (state writes after `transferFrom`). Credits are computed before the external call; re-entry cannot change `credited` for the same invocation. Remaining theoretical risk depends on assuming **standard** ERC-20 behavior for `transferFrom`.

### `receive()` / native `depositDot` / `openSession`

- `receive()` is empty (accept ETH). **`depositDot`** credits after `msg.value` intake (no external call mid-path). **`openSession`** with `msg.value` does not delegate-call out.

---

## 2. Access control

| Contract / surface | Intended actor | Enforcement |
|--------------------|----------------|-------------|
| `ProviderRegistry.transferOwnership`, `setAttestationService`, `setProviderFee` | **Owner** | `onlyOwner` |
| `ProviderRegistry.setTEEProof` | **`attestationService`** | `onlyAttestationService` |
| `ProviderRegistry.registerProvider`, `setProviderPayout`, `setProviderActive`, `setPricing` | **Calling provider** (`msg.sender` as provider) | Implicit (no admin override) |
| `SettlementEscrow` | **Anyone** for deposit / withdraw own balances; **session user** for settle; **session provider** for `recordUsage` | `msg.sender` checks; **no** single admin role on escrow |
| Price oracles | **Read-only** from escrow | Immutable `priceOracle` reference |

**Notes**

- Escrow is **not** “onlyOwner”; design is permissionless custody with per-function `msg.sender` rules. Registry **owner** is separate from **attestationService** (can be updated by owner).
- **TEE stub** off-chain must use a key whose address equals `attestationService` on the deployed registry.

---

## 3. Oracle fail-safes

| Concern | Mitigation in `SettlementEscrow` |
|---------|----------------------------------|
| **Zero price** | `getUsdcPerDot() == 0` → `BadAmount` before credit. |
| **Stale price** | Optional `maxOracleAgeSecs != type(uint256).max`: `priceUpdatedAt() == 0` or `block.timestamp > pu + max` → `OracleStale`. |
| **Slippage** | Optional `minDotInternalOut`: credited below floor → `Slippage`. |
| **Overflow** | `usdcAmount * 1e18` uses Solidity 0.8 checked math; pathological `usdcAmount` reverts. |
| **Rounding to zero** | `credited == 0` → `BadAmount`. |

**Caveats**

- Staleness uses **`block.timestamp`** and oracle-reported `priceUpdatedAt()`; validators can skew `block.timestamp` within protocol bounds — typical for DeFi oracles.
- **`DIAPriceOracle`** / live feeds: ensure feed keys and decimals match deployment assumptions; `PythPriceOracle` is a **placeholder** (`NotImplemented`).

---

## 4. Slither themes (high level)

Recurring **informational / low** items from a full-project run:

- **Solc** `^0.8.20` — compiler advisory list; project pins **0.8.28** in `foundry.toml` (good).
- **Low-level calls** on native withdraws — expected for ETH payout.
- **Timestamp** use in oracle staleness — documented above.
- **Mocks** (`MockOracle`, `MockERC20`) — not production; naming / immutables are test-only.

Run `slither .` after substantive changes and paste high-severity items into this file or a linked ticket.

---

## 5. Operational / deployment

- **Private keys:** never commit; use HSM/KMS for `attestationService` and deployers in production.
- **Registry `attestationService`:** rotate via `setAttestationService` only from **owner**; verify on-chain after deploy.
- **USDC / oracle addresses:** immutables in `SettlementEscrow` constructor — wrong wiring is not fixable without redeploy.

---

## References

- [Foundry book — gas snapshots](https://book.getfoundry.sh/forge/gas-snapshots)
- [Slither detectors](https://github.com/crytic/slither/wiki/Detector-Documentation)
- [Consensys Mythril](https://github.com/ConsenSys/mythril) (optional)

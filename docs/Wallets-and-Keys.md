# Wallets and keys

Who holds which keys in Sparkl, and which wallet signs which on-chain or off-chain action.

## Roles

| Role | Who | Primary goal |
|------|-----|----------------|
| **Operator** | Person or org running one or more inference nodes | Register nodes, manage lifecycle, receive settlement proceeds, sign TEE/PoR challenges |
| **Node (process)** | `sparkl-solo` binary on a machine | Serve inference, P2P, usage metering; may submit chain txs if configured with operator key |
| **End user (consumer)** | Wallet using the marketplace / escrow | Deposit funds, open sessions, settle or escape-hatch as session user |
| **Protocol / network** | Contract `owner`, `attestationService`, `settlementOperator` | Governance, TEE proof submission, batch settlement |

An **operator can run multiple nodes**. Each node has its own on-chain `nodeId` (`bytes32`, derived from libp2p PeerId) and its own registry row.

## Two planes: EVM vs node cryptography

### EVM (secp256k1) — Hub smart contracts

All Hub transactions use **Ethereum externally owned accounts (EOAs)**. **Libp2p keys cannot sign EVM transactions.**

### Node cryptography (Ed25519 / X25519) — protocol layer

- **Libp2p swarm key** — P2P identity and **canonical public node id** (`12D3Koo…`).
- **On-chain `nodeId`** — `bytes32` = `keccak256(libp2p PeerId multihash bytes)` (same rule as the portal).
- **Identity Ed25519** — `GET /identity` proofs, chunk receipts, attestation nonces (separate from libp2p today; may be unified later).
- **Derived X25519** — encrypt/decrypt inference payloads; `encryption_pubkey` on `registerNode`.

These never replace the operator EOA for `registerNode`, `recordUsage`, etc.

## On-chain identities per node

| Concept | On-chain | Set by |
|---------|----------|--------|
| **nodeId** | `bytes32` primary key in `ProviderRegistry` / `SettlementEscrow` | Derived from libp2p PeerId at registration |
| **nodeOperator(nodeId)** | `address` allowed to manage that node | `msg.sender` of `registerNode` |
| **payout** | `address` in `NodeInfo` (metadata + `isNodeActive` gate) | `registerNode` or `setNodePayout` by operator |

**Operator ≠ payout (optional).** Default: `payout == address(0)` → registry stores **`msg.sender`** as payout. The portal prefills payout from the connected wallet; the operator may set another address (treasury, multisig). Changing payout does **not** change who may sign `chillNode` / `recordUsage` — that stays **`nodeOperator`**.

## EVM wallets you actually use

### 1. Node operator EOA (per node on-chain; can be shared across nodes)

**On-chain name:** `ProviderRegistry.nodeOperator(nodeId)`.

**Used for:**

| Action | Contract | Who must sign |
|--------|----------|----------------|
| Register node | `ProviderRegistry.registerNode` | Caller becomes operator |
| Update payout, metadata, active flag | `setNodePayout`, `setNodeMetadata`, `setNodeActive` | `nodeOperator` |
| Chill / defunct | `chillNode`, `markDefunct` | `nodeOperator` |
| Rotate encryption key | `rotateEncryptionKey` | `nodeOperator` |
| Record usage (provider path) | `SettlementEscrow.recordUsage` | `nodeOperator(session.nodeId)` |
| Record usage (router path) | `SettlementEscrow.recordUsage` | `recordUsageRole` (sparkl-router hot key) |
| Protocol fee accrual | `SettlementEscrow` settle | 1% of gross `toProvider` → `protocolBalances` (treasury withdraws native DOT) |
| Withdraw earned balance | `SettlementEscrow.withdrawProviderDot` | `nodeOperator(nodeId)` — paid to **operator EOA**, not `payout` today |
| TEE PoR (off-chain) | Attestation stub | EIP-191 `personal_sign` by **same EOA** as `nodeOperator` |

**Where configured:**

| Surface | Config |
|---------|--------|
| Portal registration | Connected browser wallet (MetaMask, etc.) |
| Node auto-register / heartbeat / usage sync | `[settlement] evm_provider_wallet_private_key` in TOML or `SPARKLE__SETTLEMENT__EVM_PROVIDER_WALLET_PRIVATE_KEY` |

**Rule:** The private key in the node config must be the key for **`nodeOperator(nodeId)`** for that node. If it mismatches, `recordUsage` and registry calls revert with `NotNodeOperator` / `NotSessionProvider`.

There is **no separate on-chain “node EVM wallet”** — only **`nodeOperator`**. The TOML field `evm_provider_wallet_private_key` is the **operator hot key** loaded on the server.

### 2. Router `recordUsage` role EOA

**On-chain name:** `SettlementEscrow.recordUsageRole`.

**Used for:** Batched `recordUsage` txs when consumers use sparkl-router (metering from upstream `usage` JSON).

**Where configured:** `[settlement]` in [sparkl-router/config.example.toml](../../sparkl-router/config.example.toml). By default the router **generates** `data_dir/record-usage-key.json` at startup; optional `record_usage_private_key` overrides. **`registry_owner_private_key`** (registry owner) submits `setRecordUsage` when the on-chain role does not match. Local `launch-local.sh` / deploy sync sets the Anvil deployer key as owner; `DeployLocal` may still pre-assign a role address.

**Gas funding:** Withdraw from `protocolTreasury` via `withdrawProtocolDot` after session settles (1% protocol fee), then send native DOT to the recordUsage role EOA.

### 3. Protocol treasury EOA

**On-chain name:** `SettlementEscrow.protocolTreasury`.

**Used for:** Accrued protocol fee balance (`protocolBalances`); `withdrawProtocolDot` pays native DOT to the treasury wallet.

### 4. Settlement operator EOA (network-wide, not per node)

**On-chain name:** `SettlementEscrow.settlementOperator`.

**Used for:** `settleByOperatorPartial` / `settleByOperatorFull` — moves locked session funds to provider balance and user refund per rules.

**Where configured:** `settlement.evm_settlement_operator_wallet_private_key` on a settlement worker. Must match contract `settlementOperator`. Usually **separate** from the node operator key.

### 5. End user / consumer EOA

**Used for:** `deposit*`, `openSession`, user `settlePartial` / `settleFull`, `withdrawDot`.

Configured in the **user’s wallet** (portal `/user`, etc.) — not in node TOML.

### 4. Protocol admin wallets (infra)

| Role | Typical use |
|------|-------------|
| **Registry `owner`** | `setAttestationService`, `purgeDefunctNode` |
| **`attestationService`** | `setTEEProof(nodeId, hash)` |
| **Deployer** | `forge script` deploy |
| **Oracle updater** | `ModelPriceOracle`, rate oracles |

## Multi-node operator patterns

### Pattern A — one operator wallet, many nodes (common)

1. Connect wallet `0xOperator` in the portal.
2. Register each `nodeId` with `registerNode` (same `msg.sender`).
3. Set per-node `payout` (often the operator address or a shared treasury).
4. On each host, run sparkl-solo with the **same** `evm_provider_wallet_private_key`.

All nodes share **`nodeOperator`**; the chain distinguishes them by **`nodeId`**.

### Pattern B — one operator wallet per node

Each registration from a different EOA; each node TOML uses **that** node’s operator key.

### Pattern C — portal commercial register, node submits operational chain txs (MVP)

Operator **commercially registers** via portal (browser wallet = `nodeOperator`). The node process uses `evm_provider_wallet_private_key` for `recordUsage`, optional `setTEEProof` heartbeat, etc. — **not** for `registerNode` in MVP.

## Task matrix (quick reference)

| Task | Signer / key |
|------|----------------|
| Commercial register node (portal, MVP) | Operator browser EOA |
| `registerNode` from node startup | **Not used in MVP** — portal only |
| Probe / identity HTTP | Libp2p PeerId + optional Ed25519 proof |
| Encrypted inference | Node X25519 |
| Chunk receipts | Node Ed25519 |
| Heartbeat / `setTEEProof` (from node) | `evm_provider_wallet_private_key` |
| TEE challenge | Operator EOA `personal_sign` |
| `recordUsage` | `evm_provider_wallet_private_key` (= `nodeOperator`) |
| Operator settle | `evm_settlement_operator_wallet_private_key` |
| Open session / deposit | End user EOA |
| Withdraw provider earnings | `nodeOperator` via `withdrawProviderDot` |

## FAQ

### Must each node have its own EVM wallet?

**No on-chain requirement.** Multiple nodes may share one operator EOA. Each node must use a key that matches **its** `nodeOperator(nodeId)`.

### Can payout go to a different address than the operator?

**Yes** in registry metadata (`setNodePayout`). **`withdrawProviderDot`** today sends native DOT to **`nodeOperator`**, not `payout`.

### Can libp2p sign chain transactions?

**No.** Use the operator EOA for all contract calls.

## Related docs

- [sparkl-solo/docs/TRUST.md](../sparkl-solo/docs/TRUST.md) — PoR / PoA / PoU
- [sparkl-solo/DEVELOPER.md](../sparkl-solo/DEVELOPER.md) — creating operator keystores
- [sparkl-solo/contracts/SECURITY.md](../sparkl-solo/contracts/SECURITY.md) — contract roles
- [sparkl-portal/docs/DEVELOPER.md](../sparkl-portal/docs/DEVELOPER.md) — portal RPC and dev wallets

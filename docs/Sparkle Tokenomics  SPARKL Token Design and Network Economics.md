# Sparkle Tokenomics: SPARKL Token Design and Network Economics

**Version 0.1 — May 2026**

> **Engineering direction (Sparkl):** Settlement and provider registration are targeting **Polkadot Hub EVM** (`pallet_revive`), **DOT** + (later) **USDC** via precompile, with **Tier A** (TEE-verified) vs **Tier B** (best-effort) economics. Earlier drafts assumed different rails; pricing and treasury design should be read in that light. See `DEVELOPER.md` and `contracts/`.

***

## Abstract

This paper defines the economic architecture of the Sparkle network, covering the dual-currency payment system, SPARKL native token design, node operator economics, treasury mechanics, and long-term incentive alignment. Sparkle uses a **hybrid payment model**: inference is priced and settled in USDC (stable, predictable, frictionless for operators and consumers), while SPARKL functions as the network's economic security, governance, and value-accrual layer. This avoids the operator volatility risk of pure native-token payment networks while preserving the alignment incentives that make a native token worthwhile.

***

## 1. The Currency Decision: USDC vs. BTC vs. Native Token

### 1.1 Why Not BTC

Bitcoin is an inappropriate settlement currency for per-session inference billing:

- **Volatility**: an operator pricing inference at $0.10/M tokens would need to constantly reprice as BTC fluctuates 5–10% daily. Consumer costs become unpredictable
- **Settlement speed**: Bitcoin's 10-minute block time and 1-hour finality (6 confirmations) is incompatible with session-level settlement. Lightning Network adds operational complexity for what is already a multi-party protocol
- **Programmability**: the EVM escrow contracts, conditional releases, and dispute resolution mechanics that Sparkle's payment architecture requires cannot be expressed in Bitcoin Script
- **Developer friction**: the overwhelming majority of web3 developer tooling, SDK integrations, and DeFi infrastructure is built for EVM-compatible stablecoins, not BTC

BTC is an excellent treasury reserve asset — Sparkle's treasury should hold a BTC allocation for long-term value preservation. BTC is not appropriate as the primary inference payment rail.

### 1.2 Why Not Pure Native Token Payment

Networks that require inference to be paid in their native token (Nosana, early Akash, Bittensor) impose a conversion tax on every participant: consumers must buy the token to use the network; operators must sell the token to pay bills. Both incur exchange fees and price exposure. Several structural problems emerge:

- **Token price risk for operators**: a provider earning NOS or TAO for inference delivered today cannot predict what those earnings will be worth in USD when they pay their electricity bill tomorrow. This discourages participation from rational operators who cannot absorb currency risk
- **Demand is speculation-coupled**: when the native token price rises, compute appears artificially cheap in token terms; when it falls, compute appears expensive. Network usage becomes correlated with token speculation rather than genuine compute demand
- **Conversion friction for consumers**: a developer wanting to run 10 API calls must acquire a specific token, adding 2–3 steps to the onboarding funnel

Akash recognised this and introduced ACT (Akash Compute Token): a USD-pegged non-transferable credit minted by burning AKT, decoupling the payment experience from the speculative token. Render Network made the same move with Render Credits. Sparkle learns from these evolutions rather than repeating the early mistakes.

### 1.3 The Hybrid Model: USDC for Payment, SPARKL for Network Economics

Sparkle adopts a two-layer currency design:

**Layer 1 — USDC (inference payment rail)**: all inference is priced in USD, paid in native USDC on Base L2. Operators receive USDC. Consumers spend USDC. No token conversion required for either party. This is the operational layer — maximally frictionless, stable, and portable.

**Layer 2 — SPARKL (network security and value accrual)**: SPARKL is the network's economic backbone. It is required for node registration (stake), governance participation, and premium network features. SPARKL accrues value through a burn mechanism tied directly to USDC inference volume — every dollar of inference revenue burns a proportional quantity of SPARKL from circulating supply. This links SPARKL's scarcity to real network usage, not speculation.

This mirrors the most mature thinking in decentralised compute tokenomics: Akash's AKT/ACT split, Render's RENDER/Credits model, and the theoretical framework of the ClawCoin paper (arXiv:2604.19026), all converging on the same insight — stable settlement currency for operations, native token for protocol-layer alignment.

***

## 2. SPARKL Token Design

### 2.1 Supply Schedule

SPARKL follows a Bitcoin-inspired fixed supply with halving mechanics, as validated by Bittensor's architecture. Fixed supply eliminates the "rug via infinite inflation" failure mode that has destroyed several DePIN networks.

| Parameter | Value |
|---|---|
| Maximum supply | 210,000,000 SPARKL |
| Initial block/epoch reward | 2,100 SPARKL per epoch |
| Epoch duration | 10 minutes (matching inference epoch) |
| Halving trigger | Every 105,000 epochs (~2 years) |
| Year 1 daily emission | ~302,400 SPARKL/day |
| Year 3 daily emission (post-halving) | ~151,200 SPARKL/day |
| Year 5 daily emission (post-2nd halving) | ~75,600 SPARKL/day |
| Genesis allocation | 30% of max supply (63,000,000 SPARKL) |
| Mined/earned supply | 70% of max supply (147,000,000 SPARKL) |

The 10-minute epoch duration is not coincidental — it aligns with Sparkle's inference epoch settlement cycle. Epoch emissions and inference epoch settlement happen simultaneously, reducing on-chain transaction overhead.

### 2.2 Genesis Allocation

The 30% genesis allocation (63,000,000 SPARKL) is distributed at launch with the following vesting schedules:

| Allocation | % of Genesis | SPARKL Amount | Vesting |
|---|---|---|---|
| Core team | 20% | 12,600,000 | 12-month cliff, 36-month linear |
| Investors (seed + strategic) | 25% | 15,750,000 | 6-month cliff, 24-month linear |
| Foundation/Treasury | 30% | 18,900,000 | Controlled by governance, 48-month unlock |
| Ecosystem fund | 15% | 9,450,000 | Controlled by governance, deployable for grants |
| Public launch / liquidity | 10% | 6,300,000 | 20% unlocked at TGE, remainder 12-month linear |

**Total at TGE (Token Generation Event)**: approximately 6,300,000 SPARKL liquid (10% of genesis, 20% unlocked) — approximately 3% of max supply. This minimises initial dump risk while establishing exchange liquidity.

### 2.3 Earned Supply Distribution (Epoch Emissions)

Each epoch's 2,100 SPARKL emission is split between four beneficiaries:

| Recipient | % of Epoch Emission | SPARKL/Epoch | Rationale |
|---|---|---|---|
| **Node operators** | 70% | 1,470 SPARKL | Incentivises compute supply; rewards proportional to tokens delivered |
| **Treasury** | 15% | 315 SPARKL | Funds ongoing development, security audits, grants |
| **Staker rewards** | 10% | 210 SPARKL | Rewards SPARKL holders who stake to secure the network |
| **Burn reserve** | 5% | 105 SPARKL | Immediately burned — baseline deflationary pressure regardless of usage |

The 70/15/10/5 split is designed to heavily weight node operators in the growth phase, when supply-side acquisition is the primary constraint. This ratio is governable and expected to shift toward staker rewards as the network matures and operator supply becomes abundant relative to demand.

### 2.4 The Burn Mechanism

SPARKL's primary value-accrual mechanism is a **usage-driven burn** on top of USDC inference revenue. For every dollar of USDC settled through the network:

\[ \text{SPARKL burned} = \frac{\text{USD volume} \times r}{\text{SPARKL price in USD}} \]

where \(r\) is the **burn rate**, initially set at 0.5% of settled volume, governable up to 2%. This is expressed as: 0.5 cents of SPARKL value burned per dollar of inference revenue processed by the network.

**Why this mechanism works**: at $1M monthly inference volume (a modest target for even 50 active DGX Spark nodes at moderate utilisation), the burn rate removes SPARKL proportional to real economic activity. Unlike Akash's pre-buy-then-burn model, Sparkle's burn uses a small fraction of the settled USDC to market-buy and burn SPARKL — so the burn does not add friction to the consumer's payment flow. The consumer always pays USDC; the protocol autonomously executes the burn from the treasury's 5% platform fee allocation.

Concretely: from the 5% platform fee on each settled epoch:
- 2% → treasury (development, operations)
- 2% → staker yield (USDC-denominated, not SPARKL)
- 1% → automated SPARKL market buy and burn

This structure means that the higher Sparkle's inference volume, the faster SPARKL's circulating supply decreases, while operators and consumers experience no change in their USDC-denominated economics.

***

## 3. Node Operator Economics

### 3.1 Revenue Streams

A Sparkle node operator has three distinct revenue streams:

**Stream 1 — USDC inference earnings (primary)**

95% of gross inference revenue goes to the operator in USDC, settled at the end of each 10-minute epoch. This is stable, predictable, and immediately liquid. No token conversion, no price exposure.

| Configuration | Est. monthly USDC (moderate utilisation) |
|---|---|
| Node1 solo (128 GB, 70B models) | ~$850 |
| Node2 dual-DGX (256 GB, 405B models) | ~$2,100 |
| Node2 farm (3× DGX, mixed models) | ~$3,200 |

**Stream 2 — SPARKL epoch emissions (secondary)**

Operators earn SPARKL proportional to their share of total network token delivery in each epoch. If an operator delivers 5% of all tokens processed by the network in an epoch, they receive 5% of the 1,470 SPARKL allocated to operators.

This creates a long-term SPARKL accumulation benefit for early operators: when the network is small, each operator's share of epoch emissions is large. As the network grows, per-operator emissions decrease but the value of accumulated SPARKL may appreciate if usage-driven burns outpace new emissions.

**Stream 3 — Staking yield (optional)**

Operators who stake additional SPARKL beyond their registration minimum share in the 10% staker reward pool. Yield is denominated in SPARKL (inflationary early-phase yield) and partially in USDC (from the 2% staker allocation of platform fees).

### 3.2 Registration Stake

Every node operator must stake a minimum quantity of SPARKL to register on the network. The stake serves as a slashing bond — the cryptographic equivalent of a performance bond. Stake is slashed (burned, not redistributed) for:

- Confirmed attestation failure: provider's NRAS certificate is invalid or forged
- Confirmed dispute loss: on-chain dispute resolution determines provider over-claimed delivery
- Sustained unavailability: provider registered and earning SPARKL emissions but serving no actual inference (heartbeats present, inference sessions absent)

| Node type | Minimum stake | USD equivalent at launch price target |
|---|---|---|
| Node1 solo | 10,000 SPARKL | ~$200 |
| Node2 farm (up to 4 DGX) | 25,000 SPARKL | ~$500 |
| Node2 farm (5+ DGX) | 50,000 SPARKL | ~$1,000 |

Stakes are intentionally low at launch to maximise operator onboarding. They can be increased by governance as the network matures and the cost of attack grows.

**Unbonding period**: 7 days. An operator deregistering must wait 7 days before their stake is returned. This prevents registration-earn-deregister-repeat attacks that would drain epoch emissions without providing consistent network capacity.

### 3.3 Reputation Score and Emission Weighting

Raw token delivery count is not the only factor in epoch emission allocation. A reputation score multiplies each operator's raw share, incentivising quality over quantity:

\[ \text{operator share} = \frac{\text{tokens delivered} \times \text{reputation score}}{\sum_{\text{all operators}} (\text{tokens delivered} \times \text{reputation score})} \]

The reputation score is computed on-chain from a rolling 30-epoch window:

| Factor | Weight | Description |
|---|---|---|
| Uptime | 30% | Fraction of epochs with at least one active session or ready heartbeat |
| Dispute win rate | 25% | Fraction of raised disputes where operator was not at fault |
| Attestation freshness | 20% | Days since last valid NRAS certificate renewal (closer = higher) |
| Session completion rate | 15% | Fraction of opened sessions that completed without consumer timeout claim |
| Latency score | 10% | TTFT (time-to-first-token) vs. network median — lower is better |

A new operator starts with a neutral score of 1.0. Scores range from 0.5 (chronically failing provider) to 2.0 (top-tier provider), meaning a high-reputation operator earns up to 4× the epoch emissions of an equivalent-throughput low-reputation operator.

***

## 4. Treasury and Platform Fee Architecture

### 4.1 Fee Flows

Every settled inference epoch generates a 5% platform fee from the gross USDC value. This fee is the protocol's primary operating revenue — it funds development, security, grants, and ecosystem growth independently of SPARKL token price.

```
Gross USDC inference revenue (per epoch)
│
├── 95% → Node operator (USDC, direct)
│
└── 5% → Platform fee pool
        │
        ├── 2% → Development treasury (USDC)
        │       └── Pays: engineering, security audits, infrastructure
        │
        ├── 2% → Staker yield pool (USDC)
        │       └── Distributed pro-rata to SPARKL stakers each epoch
        │
        └── 1% → SPARKL burn execution
                └── Protocol market-buys SPARKL on DEX, burns it
```

### 4.2 Treasury Composition

The Sparkle treasury holds three asset classes, managed by on-chain governance:

**USDC operating reserve**: target 24 months of projected operating costs at current burn rate. This is the runway buffer — if all SPARKL price dropped to zero, the treasury's USDC reserve funds continued development. Initial target: $2M USDC.

**SPARKL ecosystem reserve**: the 30% Foundation genesis allocation (18,900,000 SPARKL). Used for ecosystem grants, liquidity incentives, partnerships, and operator bootstrap programs. Deployed only by governance vote with a 7-day timelock.

**BTC strategic reserve**: 10–20% of accumulated USDC treasury fees, periodically converted to BTC and held cold. BTC provides a hedge against USDC regulatory risk (Circle's centralised issuance) and long-term value preservation for the network's endowment.

### 4.3 Treasury Governance

The treasury is governed by SPARKL token holders through a time-locked on-chain governance module. Governance parameters:

| Action | Required quorum | Timelock after passage |
|---|---|---|
| Routine grant (<50,000 SPARKL equivalent) | 5% of staked supply | 24 hours |
| Treasury allocation (>$100K) | 15% of staked supply | 7 days |
| Tokenomics parameter change | 20% of staked supply | 14 days |
| Protocol upgrade | 25% of staked supply | 14 days |
| Emergency action | 10% of staked supply + multisig | 0 (immediate, 48hr review) |

In Phase 1 (pre-token-launch), governance is replaced by a 5-of-9 multisig held by the founding team and early investors. Governance transitions to on-chain SPARKL voting at the Phase 3 token launch.

### 4.4 Development Treasury Spending Framework

The development treasury (2% of platform fees + Foundation reserve) funds operations across four categories:

| Category | % of annual treasury | Purpose |
|---|---|---|
| Engineering | 45% | Core protocol development, node binary maintenance, SDK updates |
| Security | 20% | Smart contract audits, penetration testing, bug bounties |
| Ecosystem | 20% | Operator bootstrap grants, developer grants, hackathons |
| Operations | 15% | Infrastructure (coordinator nodes, bootstrap nodes), legal, compliance |

**Bug bounty programme**: funded from the Security allocation. Critical vulnerabilities (remote code execution on provider, attestation bypass): up to $100,000 USDC. High severity (dispute manipulation, epoch settlement fraud): up to $25,000 USDC. Medium: up to $5,000 USDC.

***

## 5. Consumer Economics

### 5.1 Payment Flow

Consumers interact entirely in USDC — no SPARKL purchase required for inference. The payment flow:

1. Consumer calls `api.sparkle.dev/v1/chat/completions` with a standard API key
2. The session escrow contract on Base locks the session's maximum USDC cost
3. Inference streams over QUIC; chunk receipts accumulate
4. At epoch close, the operator's settlement proof releases the actual USDC consumed
5. Unused escrow (if session ended early) is returned immediately

For agentic consumers (AI agents calling Sparkle autonomously), Unicity single-spend tokens replace manual USDC authorisation — the agent holds a pre-funded SPARKL or USDC balance and the SDK handles the entire payment flow without human intervention.

### 5.2 Volume Discounts

Consumers who stake SPARKL receive discounted inference rates. This creates organic SPARKL demand from high-volume users:

| SPARKL staked | Inference discount | Min. SPARKL required |
|---|---|---|
| None | 0% (standard rate) | 0 |
| Tier 1 | 5% | 5,000 SPARKL |
| Tier 2 | 10% | 20,000 SPARKL |
| Tier 3 | 15% | 100,000 SPARKL |
| Enterprise | 20% + SLA | Negotiated |

### 5.3 Prepay Credits

Consumers can prepay USDC for Sparkle Credits — non-transferable credits priced at a 5% discount to spot rates. Credits are held in the Unicity single-spend token system, redeemable against any provider. This mirrors Akash's ACT and Render's Credits models, providing:

- Predictable cost planning for consumers
- Upfront liquidity for Sparkle's treasury
- A natural mechanism for institutional prepayment without crypto custody complexity

***

## 6. Comparative Analysis

### 6.1 Tokenomics Model Comparison

| Network | Payment currency | Native token utility | Burn mechanism | Operator payment | Halving |
|---|---|---|---|---|---|
| **Sparkle** | USDC (stable) | Stake + governance + volume discount | Usage-driven (1% of fees) | 95% USDC + SPARKL emissions | Yes (supply-based, ~2yr) |
| **Akash** | ACT (USD-pegged credit) | AKT burned to mint ACT | Buy-and-burn at compute purchase | USD-equivalent ACT | No |
| **Render** | Render Credits (fiat-priced) | RENDER burned at job submission | Burn at job submission | RENDER (newly minted) | No |
| **Nosana** | NOS token | Staking + node operation | None (emission only) | NOS (token) | No |
| **Bittensor** | TAO emissions | Mining + validation rewards | Recycling mechanism | TAO (50% to miners) | Yes (supply-based, 21M cap) |
| **Darkbloom** | USD (Stripe) | None — no token | N/A | USD (95%) | N/A |

### 6.2 Key Design Decisions vs. Alternatives

**Why not SPARKL-only payments (like Nosana)?**
Nosana operators earn NOS for inference delivered. When NOS fell 70%+ from its peak, operators' real earnings halved despite delivering the same compute. Sparkle's USDC-primary model ensures operators always know what their compute is worth in real terms.

**Why not pure BME (like Akash/Render)?**
Akash's BME and Render's model require consumers to buy native tokens, then the protocol burns them. This is friction-efficient from the token's perspective but adds two conversion steps (fiat → AKT → ACT) for the consumer. Sparkle's model adds zero steps: consumers pay USDC directly, the protocol handles the SPARKL burn autonomously in the background.

**Why not Bitcoin-inspired halving with pure miner rewards (like Bittensor)?**
Bittensor's emission-only model means operators' SPARKL-equivalent earnings halve every two years regardless of USD denominated demand. An operator joining in Year 4 earns 4× less token for the same work as a Year 1 operator. Sparkle's USDC earnings are demand-driven, not emission-driven — a Year 4 operator earns the same USDC per token delivered as a Year 1 operator if demand is equal.

***

## 7. Launch Phases and Token Schedule

### Phase 1 — Network Bootstrap (Pre-Token)

- No SPARKL token exists
- Operators are paid 100% in USDC, no fee
- Platform fee is 0% — all revenue goes to operators
- Goal: prove the inference product works, acquire first 50 operators
- Duration: ~3 months

This is the Darkbloom model precisely: simple, trusted, USDC-only. Remove every possible friction to getting DGX Spark owners online.

### Phase 2 — SPARKL Private Launch

- SPARKL token generated; genesis allocation issued with vesting schedules
- Platform fee activates at 5% (USDC)
- Operators begin receiving SPARKL epoch emissions on top of USDC
- Stake requirement activates — operators who registered in Phase 1 have a 60-day grace period to acquire and stake minimum SPARKL
- SPARKL is not yet publicly tradeable — only earned by operators and issued to genesis holders
- Duration: ~3 months

### Phase 3 — Public Token Launch

- SPARKL lists on DEX (Uniswap v3 on Base, Raydium on Solana) with seed liquidity from the public launch allocation
- Usage-driven burn mechanism activates (1% of platform fees)
- Consumer staking discounts activate
- On-chain governance replaces multisig
- Duration: ongoing

### Phase 4 — EigenLayer Integration

- Sparkle registers as EigenLayer AVS
- Node operators can optionally restake ETH as additional collateral
- EigenLayer operators earn additional AVS rewards denominated in SPARKL
- SPARKL burn rate increases by governance vote as TVL grows
- Duration: ongoing

***

## 8. Risk Factors and Mitigations

### 8.1 Token Price Death Spiral

**Risk**: SPARKL price falls → operators lose interest in epoch emissions → fewer operators → worse service → less USDC volume → less burn pressure → lower price.

**Mitigation**: USDC earnings are entirely independent of SPARKL price. An operator earning $850/month USDC continues earning $850/month whether SPARKL is at $0.01 or $10. The SPARKL emission is a bonus, not the primary economic justification. Operators with rational economics — serving inference for USDC earnings — are not affected by SPARKL price.

### 8.2 Regulatory Risk (USDC Centralisation)

Circle, USDC's issuer, can freeze addresses and has done so under law enforcement orders. A frozen escrow contract would block settlements.

**Mitigation**: the EVM escrow contract accepts USDC as the default but the token address is a governance parameter. If Circle freezes the contract, governance can migrate to EURC, DAI, or another stablecoin. Sparkle's settlement layer is currency-agnostic at the contract level.

### 8.3 Emission Dilution (Early Operator vs. Late Operator Fairness)

Early operators accumulate SPARKL at a higher rate per unit of compute than late operators (larger share of smaller network).

**Mitigation**: this is by design and is the standard bootstrap incentive in every DePIN network. Late operators' USDC earnings scale with demand — if the network reaches $10M monthly volume, late operators earn far more USDC than early operators did, even if their SPARKL share per epoch is smaller. The reputation score's historical component also benefits operators who joined early and maintained quality service.

### 8.4 Stake Concentration (Governance Attack)

Large SPARKL holders could accumulate governance power and vote to redirect treasury funds to themselves.

**Mitigation**: 7–14 day timelocks on all major governance actions allow the community to observe and respond before execution. The multisig emergency mechanism allows the founding team to veto clearly malicious proposals during the network's early phase. Long-term, quadratic voting (one-person-one-vote weighted by square root of stake) can be adopted by governance to reduce whale dominance.

***

## 9. Summary Economics

At maturity (100 active DGX Spark nodes, 40% average utilisation, mixed model tiers):

| Metric | Estimate |
|---|---|
| Monthly network USDC volume | ~$2.5M |
| Monthly operator earnings (aggregate) | ~$2.4M USDC |
| Monthly platform fee to treasury | ~$50,000 USDC |
| Monthly SPARKL burn (1% of fees) | ~$500 equivalent SPARKL |
| Monthly SPARKL emissions to operators | ~8,820,000 SPARKL (Year 1 rate) |
| Annual emission to supply ratio | ~15% of circulating supply (Year 1) |
| Breakeven for solo Node1 operator | ~5–6 months hardware payback |

The network's economic proposition is straightforward: DGX Spark owners earn meaningful USDC income from hardware they already own, with SPARKL as an additional speculative upside. The USDC earnings require no token exposure, no price prediction, and no liquidity risk. SPARKL's value depends entirely on whether Sparkle's network usage grows — if it does, burn pressure creates genuine scarcity; if it does not, USDC earnings remain the operator's rational justification for participation regardless.

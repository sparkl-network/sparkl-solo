# Sparkle: Decentralised Private AI Inference for NVIDIA DGX Spark Owners

**Version 0.1 — May 2026**

***

## Abstract

Sparkle is a decentralised AI inference network that transforms idle NVIDIA DGX Spark personal supercomputers into privacy-preserving inference nodes, connecting their substantial compute capacity directly to developers, businesses, and autonomous AI agents. Inspired by Darkbloom's architecture for Apple Silicon, Sparkle extends the model to NVIDIA's GB10 Grace Blackwell platform, enabling the serving of frontier-class language models up to 200 billion parameters on a single node and up to 405 billion parameters across two interconnected units. The network achieves verifiable privacy through a four-layer trust model anchored in NVIDIA's Remote Attestation Service (NRAS) and hardware TPM2, uses Unicity's Sparse Merkle Tree aggregation for tamper-evident session receipts and provider discovery, and settles payments through a hybrid of Unicity single-spend tokens and EVM escrow contracts. Provider-to-consumer communication uses libp2p with QUIC transport for low-latency streaming and encrypted peer discovery, with an optional centralised coordinator for developer convenience. The result is an OpenAI-compatible inference API where operators earn 95% of revenue from compute they already own, users pay approximately 50% less than centralised providers, and neither party needs to trust the other beyond what hardware attestation and cryptographic proofs enforce.

***

## 1. Introduction

### 1.1 The Idle Compute Problem

The NVIDIA DGX Spark is a personal AI supercomputer based on the Grace Blackwell GB10 SoC, delivering 1 PFLOP of sparse FP4 AI compute and 128 GB of LPDDR5x unified memory at 273 GB/s bandwidth. At a retail price of approximately $4,699, it represents the most accessible path to serving frontier-class models locally. Yet most DGX Spark units sit idle for the majority of each day — during off-hours, weekends, and periods between active development sessions. The marginal cost of serving inference during idle periods is effectively zero beyond electricity (~$0.003–$0.004/hour at UK commercial rates), while the unit's capacity to serve models including Llama 3.3 70B, DeepSeek V3, and Qwen3 32B is fully realised.

Meanwhile, API inference from centralised cloud providers carries a three-layer markup: GPU manufacturers to hyperscalers, hyperscalers to inference API providers, and inference API providers to end users. Darkbloom, built by Eigen Labs, demonstrated that this markup can be collapsed by routing inference directly through idle personal hardware, pricing at 50% of leading competitors while maintaining operator margins above 90%. Darkbloom targets Apple Silicon Macs. Sparkle extends this model to DGX Spark hardware, enabling larger model serving, stronger hardware attestation infrastructure, and a fully decentralised peer-to-peer network topology.

### 1.2 Design Principles

Sparkle is designed around five principles:

- **Verifiable privacy**: the operator's hardware provably cannot observe user prompts or model responses, enforced through hardware attestation and end-to-end encryption, not contractual promises
- **Decentralisation**: no single coordinator is required; consumers can discover and connect to providers using fully trustless on-chain and aggregation-layer primitives
- **OpenAI compatibility**: the API surface is a drop-in replacement for existing applications, requiring only a `base_url` change
- **Operator economics**: providers retain 95% of inference revenue from hardware they already own, with no upfront cost to join beyond running an install script
- **Composability**: the architecture is designed to merge with Darkbloom and register as an EigenLayer Actively Validated Service, embedding Sparkle's inference capacity into the broader restaking economy

### 1.3 Relationship to Darkbloom

Darkbloom (`github.com/Layr-Labs/d-inference`) is an experimental prototype built by Eigen Labs, operating on Apple Silicon Macs via macOS-specific primitives: Apple Secure Enclave for key custody, `launchd` for service management, `PT_DENY_ATTACH` and Hardened Runtime for process isolation, and Apple's Managed Device Attestation for the trust chain. The network charges a 5% platform fee, routes through a Go-based coordinator running in a confidential VM, and uses NaCl Box (X25519 + XSalsa20-Poly1305) for end-to-end encryption.

Sparkle shares Darkbloom's encryption primitives (NaCl Box, wire-compatible with its Rust provider), billing philosophy, and coordinator architecture, but replaces every macOS-specific component with Linux/NVIDIA equivalents and adds a fully decentralised peer discovery and settlement layer that Darkbloom does not currently provide. The two systems are designed to merge: a common coordinator can route to both Apple Silicon and DGX Spark nodes once an `AttestationVerifier` interface abstracts the two hardware trust chains.

***

## 2. Hardware Foundation

### 2.1 DGX Spark Specifications

The DGX Spark is built on NVIDIA's GB10 Grace Blackwell Superchip, combining an ARM-based Grace CPU with a Blackwell GPU in a unified memory architecture.

| Component | Specification |
|---|---|
| SoC | NVIDIA GB10 Grace Blackwell |
| CPU | 20-core ARM (10× Cortex-X925 + 10× Cortex-A725) |
| GPU | Blackwell architecture, 1 PFLOP sparse FP4 |
| Unified Memory | 128 GB LPDDR5x |
| Memory Bandwidth | 273 GB/s |
| Max Model (single) | 200B parameters |
| Max Model (dual) | 405B parameters (two interconnected units) |
| Networking | 10 GbE + ConnectX-7 (up to 200 Gbps) |
| Storage | Up to 4 TB NVMe SSD |
| Operating System | Ubuntu 24.04 (DGX OS) |
| Idle Power | ~40W |
| Price | ~$4,699 (4TB Founders Edition) |

### 2.2 Inference Capabilities

The DGX Spark supports the full NVIDIA AI software stack natively: llama.cpp, vLLM, Ollama, TensorRT-LLM, and NVIDIA NIM containers. Benchmark decode rates for large models reach approximately 50 tokens/second on llama.cpp. The `llama-swap` tool enables dynamic model loading and unloading, allowing a single Sparkle node to serve multiple models without pre-loading all simultaneously. LiteLLM provides a unified OpenAI-compatible proxy across all backends.

The 128 GB unified memory is the defining capability advantage over GPU-based workstations: the RTX PRO 6000 Blackwell maxes at 96 GB VRAM, while the DGX Spark can load models that simply cannot fit elsewhere at this price point.

| Model | Parameters | Type | DGX Spark Status |
|---|---|---|---|
| Llama 3.3 70B | 70B dense | FP4/GGUF | Excellent — fits with headroom |
| DeepSeek V3 | 671B MoE (37B active) | FP4 MoE | Single node feasible |
| Qwen3 32B | 32B dense | FP8 | Fast decode |
| GPT-OSS 120B | 120B MoE | FP4 | ~50 tok/s decode |
| Llama 4 Scout | 109B MoE | FP8 | Single node |
| Dual-node models | 200–405B | FP4 | Two interconnected Sparks |

### 2.3 Security Hardware

The DGX Spark's security stack provides the hardware roots of trust that Sparkle's attestation model depends upon:

- **UEFI Secure Boot**: all firmware is cryptographically signed; boot chain integrity is enforced from power-on
- **Hardware TPM2**: Trusted Platform Module provides hardware-sealed key generation and storage, remote attestation capabilities, and measured boot
- **NVIDIA NRAS**: the NVIDIA Remote Attestation Service provides GPU-level attestation certificates, signing hardware reports that trace to NVIDIA's root CA
- **Self-Encrypting Drives**: NVMe SED support for storage encryption at rest

***

## 3. Trust Model and Privacy Architecture

### 3.1 The Fair-Inference Problem

The core technical challenge in any distributed inference network is the same problem Darkbloom identifies: the machine owner has root access and physical custody of the hardware. A naive implementation allows operators to intercept user prompts, log model responses, or substitute different models than advertised. Sparkle must make these attacks cryptographically impossible, not merely contractually prohibited.

The threat model Sparkle addresses:

- **Curious operator**: operator wants to read user prompts or responses
- **Malicious operator**: operator serves a degraded model, truncates responses, or lies about model identity
- **Network observer**: passive attacker monitoring traffic between consumer and provider
- **Dishonest consumer**: consumer denies receiving tokens they received to avoid payment
- **Dishonest provider**: provider claims to have delivered tokens it did not

### 3.2 Four-Layer Trust Architecture

**Layer 1: Hardware Attestation (NRAS + TPM2)**

On startup, the Sparkle provider binary generates an X25519 keypair inside the TPM2 hardware security module. The TPM2 produces a signed attestation report: a cryptographic statement that (a) this keypair was generated inside tamper-resistant hardware, (b) the measured boot chain matches known-good values, and (c) the software running is the expected Sparkle provider binary.

This attestation report is submitted to NVIDIA's Remote Attestation Service (NRAS), which verifies the GPU-level hardware state and counter-signs the attestation. The resulting certificate chains to NVIDIA's root CA — a trust anchor not controlled by Sparkle. The provider's X25519 public key is bound to this certificate: possession of the corresponding private key proves the holder is the attested hardware.

**Layer 2: End-to-End Encryption**

All inference payloads are encrypted using NaCl Box (X25519 + XSalsa20-Poly1305). The coordinator generates an ephemeral X25519 keypair per request (forward secrecy). Encryption uses the provider's TPM-bound public key; only the provider's TPM can decrypt. Since the TPM private key is hardware-sealed and non-exportable, no software running on the provider host — including the operator's own processes — can decrypt the payload.

The encryption flow:

```
1. Consumer → Coordinator: plaintext request (over TLS)
2. Coordinator → encrypts with provider's TPM-bound X25519 pubkey (ephemeral session key)
3. Coordinator → Provider: NaCl Box ciphertext (operator cannot decrypt)
4. Provider TPM → decrypts inside hardened process
5. Inference runs in-process (no subprocess, no IPC, no network socket)
6. Response → encrypted back to session key → Coordinator → Consumer
```

**Layer 3: Process Isolation**

The provider process runs under a hardened Linux security profile:

- `seccomp-bpf` filter: blocks system calls that could expose memory to external processes
- `AppArmor` mandatory access control profile: confines filesystem and network access to inference-required paths only
- `UEFI Secure Boot + TPM PCR binding`: the TPM keypair is only accessible when the measured boot chain matches the registered values — a patched binary cannot access the keys
- No subprocess or IPC path to the inference engine: inference runs in-process (via embedded Python/vLLM binding), eliminating the class of attacks where an operator intercepts the IPC channel

**Layer 4: Continuous Attestation Challenges**

The coordinator periodically issues nonce-signing challenges to active providers. The provider signs the nonce with its TPM-bound key and returns a signed response. A provider that has been physically modified, had its TPM replaced, or is routing through a proxy fails the challenge. Providers failing challenges are immediately removed from the routing pool and their on-chain stake is subject to slashing.

### 3.3 Trust Hierarchy

```
NVIDIA Root CA
    └── NRAS (NVIDIA Remote Attestation Service)
            └── DGX Spark TPM2 Certificate
                    └── Provider Node X25519 Keypair (NOISE libp2p identity)
                            └── Session Ephemeral Keypair (NaCl Box per-request)
                                    └── Chunk Receipt Ed25519 Signatures

Unicity BFT Consensus (1-second rounds)
    └── SMT Root (BFT-certified, immutable)
            └── Provider Token State (registration, heartbeats, capabilities)
                    └── Epoch Receipt Batch (inclusion proof per session)
                            └── Non-Deletion Proofs (dispute evidence)

EVM Escrow Contract (Base L2)
    └── Session Escrow Funds
            └── Epoch Settlement (proof-triggered release)
                    └── Dispute Resolution (optimistic + Unicity evidence)
```

***

## 4. Network Architecture

### 4.1 System Topology

Sparkle operates across three connection paths, supporting both developer convenience and maximum decentralisation:

```
┌─────────────────────────────────────────────────────────────────┐
│                      SPARKLE NETWORK                            │
│                                                                 │
│  ┌──────────────┐  ┌─────────────────┐  ┌─────────────────┐    │
│  │  Consumers   │  │   Coordinator   │  │  DGX Spark      │    │
│  │  (humans /   │  │   (optional)    │  │  Providers      │    │
│  │   AI agents) │  │   TypeScript    │  │  (Rust binary)  │    │
│  └──────┬───────┘  └────────┬────────┘  └────────┬────────┘    │
│         │                   │                    │              │
│         └──────── :30333 libp2p NOISE/DHT ────────┘             │
│         └──────── :9944 QUIC/WS inference stream ──┘            │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                TRUST & SETTLEMENT LAYER                  │   │
│  │                                                          │   │
│  │  Unicity Aggregation   ←→   EVM Escrow (Base)            │   │
│  │  (SMT registry,               (fund custody,             │   │
│  │   receipt proofs,              settlement,               │   │
│  │   single-spend tokens)         dispute resolution)       │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

**Path A — Coordinator-proxied** (MVP, works for all consumers): Consumer calls `api.sparkle.dev/v1/chat/completions`; the coordinator selects a provider, establishes a WebSocket tunnel, and streams the response. Simplest integration — identical to OpenAI SDK usage.

**Path B — Direct P2P over QUIC** (low-latency, privacy-maximising): Consumer discovers providers from the Unicity registry, performs a libp2p NOISE handshake on port 30333, verifies attestation, then opens a QUIC stream on port 9944 directly to the provider. No coordinator involvement.

**Path C — LAN/same-rack** (zero-latency): Two DGX Sparks on the same network discover each other via mDNS, connect directly over QUIC. Enables dual-node 405B parameter model serving without internet routing.

### 4.2 Port Assignments and Transport Stack

**Port 30333 — Peer Discovery and Handshake**

Port 30333 is the Substrate/Polkadot ecosystem convention for libp2p peer-to-peer networking. Sparkle adopts this convention for compatibility with the broader Polkadot tooling and operator expectations. The following protocols run on this port:

- **Kademlia DHT**: address resolution for known PeerIDs (not full provider discovery — that is handled by the Unicity registry)
- **mDNS**: local network discovery; two DGX Sparks on the same LAN discover each other without hitting the DHT
- **NOISE protocol handshake**: mutual authentication using each node's TPM-derived X25519 keypair; a successful handshake constitutes hardware attestation since the TPM private key is non-exportable
- **Identify protocol**: peers exchange supported protocols, capabilities, and software version
- **Attestation exchange**: after NOISE handshake, providers transmit their NRAS certificate; consumers verify it against the Unicity registry before routing any inference

**Port 9944 — Inference Streaming**

Port 9944 is the Substrate convention for WebSocket RPC. Sparkle adopts it for the inference data plane:

- QUIC preferred (UDP, encrypted headers, independent stream multiplexing, 0-RTT on resumed connections)
- WebSocket/TCP fallback for environments where UDP is blocked
- Each inference session is a QUIC stream (for concurrent sessions) or WebSocket connection (coordinator path)
- HTTP upgrade semantics allow the same port to serve both chain state queries (for operators running a Substrate node) and inference streams

**QUIC vs. WebSocket Privacy Properties**

QUIC provides superior privacy at the transport layer: QUIC encrypts packet headers (including packet numbers), supports ECH to hide destination hostnames, enables Connection ID rotation to break session correlation, and coalesces stream frames into datagrams to obscure per-token timing. WebSocket over TLS leaks frame sizes and timing that enable traffic analysis attacks even against encrypted payloads. For direct P2P sessions where maximum privacy is the goal, QUIC is the required transport. For coordinator-proxied sessions where infrastructure compatibility matters, WebSocket remains supported.

**Concurrent Session Handling**

QUIC multiplexes independent streams over a single UDP socket. A DGX Spark serving 16 concurrent inference sessions over QUIC uses one kernel socket with 16 independent streams — retransmits on one stream do not block others, congestion control is unified, and OS scheduler overhead is minimised. The equivalent WebSocket design uses 16 separate TCP connections competing for the NIC queue. At the scale of a DGX Spark's 128 GB unified memory capacity — potentially supporting 8–16 simultaneous large model sessions — QUIC's multiplexing advantage is material.

### 4.3 Provider Discovery: Three-Layer Architecture

**Layer 1: Unicity SMT Registry (Authoritative)**

Every provider registers as a Unicity token — a cryptographically addressed state object whose token ID is derived deterministically from their TPM public key. The registration state includes:

```json
{
  "peerId": "12D3KooW...",
  "multiaddrs": ["/ip4/1.2.3.4/udp/30333/quic-v1", "/ip4/1.2.3.4/tcp/9944/ws"],
  "models": ["llama-3.3-70b", "qwen3-32b"],
  "attestationHash": "0xabc...",
  "gpuMemoryGB": 128,
  "priceMicroUSDPerMToken": 200,
  "lastSeen": 1746878400000
}
```

State transitions — heartbeats every 30 seconds — update the token state. The Unicity aggregator inserts each transition into a Sparse Merkle Tree and certifies the root via BFT consensus with 1-second finality. Non-deletion proofs guarantee that a committed registration cannot be retroactively removed — a provider's history is permanently auditable.

Consumers query the aggregator API for a range scan: "return all active provider tokens where `models` contains `llama-3.3-70b` and `lastSeen` within 60 seconds". Each result carries an SMT inclusion proof — verifiable against the public BFT root without trusting the aggregator. This is the complete, verifiable, sybil-resistant provider directory. No DHT, no coordinator, and no central server is required for trustless discovery.

**Layer 2: libp2p Kademlia DHT (Address Resolution)**

Once a consumer has a provider's PeerID from the Unicity registry, the Kademlia DHT resolves the current multiaddr for that PeerID. This is the DHT's ideal use case: point lookup, not enumeration. Providers continuously refresh their DHT records automatically via libp2p's maintenance routines. Stale DHT records are a minor routing inconvenience — the canonical existence proof is the Unicity registry.

**Layer 3: Coordinator Cache (Millisecond Fast Path)**

The coordinator maintains an in-memory materialised view of the Unicity registry, refreshed via aggregator event subscriptions and periodic SMT scans. Consumer queries against `api.sparkle.dev/v1/providers` return in under 10 milliseconds from the cache. The cache is a performance optimisation — its staleness can always be verified against the Unicity registry directly.

A background reachability crawler probes registered providers periodically, marking unreachable nodes as stale in the cache without removing them from the Unicity registry. Stale providers are filtered from routing but retained for dispute resolution history.

***

## 5. Payment and Settlement Architecture

### 5.1 Design Constraints

Per-token inference billing creates unique payment constraints that standard payment systems do not handle well:

- **Micropayment granularity**: individual tokens cost fractions of a cent; on-chain transactions per token are economically irrational at any gas price
- **Streaming delivery**: payment corresponds to a continuous stream of tokens over 5–30 seconds, not a discrete completed delivery
- **Fair exchange**: neither party should be able to defect — the provider should not be paid for tokens not delivered; the consumer should not receive tokens without payment
- **Machine-speed operation**: AI agents calling the Sparkle API autonomously require zero-human-interaction payment flows

### 5.2 x402 for Payment Signalling

Sparkle uses the x402 protocol for payment authorisation — the HTTP `402 Payment Required` response tells the consumer exactly what to pay, where, and in what token, before service is rendered. x402 is built on EIP-3009 `transferWithAuthorization` — the consumer signs an off-chain payment authorisation (no gas, no on-chain transaction) that a facilitator broadcasts to settle USDC atomically on Base.

The x402 `PAYMENT-REQUIRED` header carries Sparkle-specific metadata alongside standard payment terms:

```http
HTTP/1.1 402 Payment Required
X-PAYMENT-REQUIRED: {
  "amount": "1500",
  "asset": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
  "payTo": "0x...",
  "maxTimeoutSeconds": 60,
  "sparkle": {
    "providerPeerId": "12D3KooW...",
    "attestationTx": "unicity://token/...",
    "model": "llama-3.3-70b",
    "sessionPubkey": "base64encodedX25519..."
  }
}
```

The consumer's SDK verifies the `attestationTx` field against the Unicity registry before signing the payment authorisation. If the provider's attestation is absent, stale, or revoked, the consumer refuses to sign — no service is attempted and no payment flows. Attestation validity is a **precondition on payment**, not a post-hoc audit.

### 5.3 x402 Limitations and Sparkle's Augmentations

x402 is payment-first: payment clears before service begins. For streaming inference, this creates a structural fair-exchange gap documented formally in the A402 paper (arXiv:2603.01179): a provider can receive payment and then crash mid-stream; x402 provides no protocol-level recourse. Conversely, a consumer could disconnect after receiving all tokens before payment settles.

Sparkle addresses this with three augmentations:

**Signed Chunk Receipts**: the consumer signs a cryptographic acknowledgement every 50 tokens (or 2 seconds, whichever is sooner):

```
ChunkReceipt {
    session_id:    UUID,
    provider_id:   PeerID,
    seq:           u64,        // monotonic
    token_count:   u32,        // cumulative tokens acknowledged
    content_hash:  [u8; 32],   // SHA-256 of ciphertext chunk batch
    timestamp:     u64,        // unix milliseconds
    consumer_sig:  [u8; 64],   // Ed25519 over above fields
}
```

These receipts are signed with the consumer's session key — the same key established during the NOISE handshake and tied to their Unicity single-spend payment token. A provider cannot claim more tokens were delivered than the consumer acknowledged; a consumer cannot deny receiving tokens they signed for.

**EVM Escrow Contract**: rather than paying the full session value upfront and relying on provider honesty for refunds, the consumer's x402 authorisation locks funds in a Base escrow contract. Funds release in response to verifiable epoch settlement proofs. If the provider crashes mid-session and the escrow timeout triggers (60 seconds with no settlement progress), the contract refunds the consumer proportionally based on the last acknowledged chunk receipt.

**Unicity Single-Spend Tokens**: payment commitments are issued as Unicity single-spend tokens — redeemable exactly once, provably. This prevents consumers from double-spending a payment authorisation across two providers simultaneously and gives providers a cryptographic proof of the payment commitment independent of any EVM state.

### 5.4 Epoch Batch Settlement

Individual per-token on-chain transactions are replaced by epoch batches. Every 10 minutes, each provider:

1. Collects all closed session summaries for the epoch
2. Merkle-hashes all chunk receipts into an epoch root
3. Submits the epoch batch as a Unicity state transition — receives an inclusion proof with BFT-certified timestamp and a non-deletion guarantee
4. Calls `settleEpoch(epochId, receiptsRoot, unicityProof, totalTokens, amount)` on the EVM escrow contract
5. The contract verifies the Unicity inclusion proof structure, confirms the provider's attestation is active, and releases the epoch's escrowed funds

Under normal operation, this is **one EVM transaction per provider per 10 minutes** — negligible gas cost regardless of how many inference sessions occurred within the epoch.

### 5.5 Dispute Resolution

Disputes arise when a provider claims more tokens were delivered than the consumer acknowledged, or when a consumer denies receiving tokens the provider can prove were sent. The resolution mechanism is optimistic and on-chain:

**Normal path** (automatic, no dispute): session completes, consumer sends final signed receipt, provider includes it in epoch settlement. EVM contract releases funds. Typical case — no dispute mechanism invoked.

**Provider crash mid-stream**: consumer detects missing chunks (gap in signed sequence numbers). After the session timeout, the consumer submits their last valid chunk receipt to the EVM contract as a `claimPartialRefund`. The contract verifies the consumer's Ed25519 signature on the receipt against the provider's attested session key, computes the fraction of tokens delivered, and refunds the remainder. No arbitrator required.

**Disputed delivery (contested)**: either party raises a dispute by submitting their Unicity inclusion proof to the EVM contract. The contract verifies both proofs against the public Unicity BFT root (available via oracle), identifies the inconsistency (provider's claimed delivery vs. consumer's last acknowledged receipt), and resolves automatically. Disputes are typically resolved within one Ethereum block on Base (approximately 2 seconds).

**Fraudulent dispute** (bad-faith claim by either party): the disputing party's staked tokens are slashed. Providers must stake to register; consumers making repeated fraudulent dispute claims have their API keys suspended.

### 5.6 Pricing Model

Sparkle prices at 50% of the cheapest major competitor for each model tier, following Darkbloom's established pricing philosophy. DGX Spark's ability to serve larger models (200B+ parameters) enables a premium tier not available on Apple Silicon hardware.

| Model Tier | Input ($/M tokens) | Output ($/M tokens) | Benchmark |
|---|---|---|---|
| Small dense (≤32B) | $0.03 | $0.15 | 50% of OpenRouter |
| Large dense (33B–70B) | $0.10 | $0.78 | 50% of OpenRouter |
| Large MoE (70B–200B) | $0.13 | $1.04 | 50% of OpenRouter |
| XL MoE (200B–405B, dual-node) | $0.20 | $1.80 | Premium tier — not available elsewhere at this price |

Platform fee: 5% (consistent with Darkbloom's actual fee structure as confirmed in source code). Provider receives 95% of gross inference revenue.

***

## 6. Component Architecture

### 6.1 sparkle-provider (Rust binary, DGX Spark)

The provider agent is a statically-linked ARM64 (`aarch64-unknown-linux-gnu`) binary with no runtime dependencies. Installation:

```bash
curl -fsSL https://api.sparkle.dev/install.sh | bash
```

The install script downloads the binary, generates a TPM2 keypair via `tpm2-tools`, registers with the Sparkle network (submitting the NRAS attestation report), writes `/etc/systemd/system/sparkle-provider.service`, and enables the service. Total install time: under 2 minutes.

**Core modules:**

- `attestation.rs`: TPM2 keypair generation, NRAS certificate submission, periodic challenge-response signing
- `crypto.rs`: NaCl Box keypair management, request decryption, response encryption (wire-compatible with Darkbloom's Rust provider)
- `tunnel.rs`: WebSocket/QUIC client connecting outbound to coordinator; libp2p swarm for P2P mode
- `inference.rs`: proxy to local llama-swap → vLLM/llama.cpp/Ollama/NIM backends
- `scheduling.rs`: bounded async queue (`tokio::sync::mpsc` + `Semaphore`), concurrency management, backpressure signalling
- `receipts.rs`: chunk receipt generation, checkpoint signing, epoch batch assembly
- `settlement.rs`: Unicity state transition SDK calls, IPFS pinning, EVM `settleEpoch` submission
- `store.rs`: `sled` embedded KV store for in-flight session persistence and crash recovery

**Key Cargo dependencies:**

```toml
libp2p       = { version = "0.55", features = ["tokio","quic","tcp","noise","yamux","kad","mdns","identify"] }
tpm2-tss     = "0.7"
crypto_box   = "0.9"      # NaCl Box — wire-compatible with Darkbloom
ed25519-dalek = "2"
axum         = "0.8"      # WebSocket/HTTP server on :9944
sled         = "0.34"
subxt        = "0.37"
tokio        = { version = "1", features = ["full"] }
tokio-tungstenite = "0.26"
reqwest      = "0.12"
```

### 6.2 sparkle-coordinator (TypeScript / Hono)

The coordinator is an optional centralised gateway providing the developer convenience path. It is not required for the network to function — it can be run by Sparkle Labs, by large operators, or by any third party.

**Core responsibilities:**

- OpenAI-compatible HTTP API (`/v1/chat/completions`, `/v1/models`, streaming SSE)
- Provider registry cache backed by Unicity aggregator subscriptions
- Request routing: selects provider by model availability, measured latency, load, and pricing
- WebSocket/QUIC tunnel management for providers behind NAT
- NRAS attestation verification for registered providers
- x402 payment facilitation: receives EIP-3009 signed authorisations, broadcasts to Base
- Redis wait queue for burst handling (10-second bounded queue, `BLPOP` pattern)
- NATS pub/sub for job dispatch to provider tunnels
- Reachability crawler: probes registered providers, marks stale in cache

**Key packages:**

```json
{
  "hono": "^4",
  "surrealdb": "^1",
  "ioredis": "^5",
  "nats": "^2",
  "@unicity/state-transition-sdk": "^1",
  "viem": "^2",
  "zod": "^3"
}
```

### 6.3 Unicity Integration Layer

The Unicity State Transition SDK is used at three points in the Sparkle stack:

**Provider registration** (on startup, TypeScript coordinator or Rust via FFI):
```typescript
const uc = new UnicityClient(aggregatorUrl);
const token = await uc.mintToken({
  tokenId: deriveTokenId(tpmPublicKey),
  state: { peerId, multiaddrs, models, attestationHash, gpuMemoryGB, price },
  ownerProof: tpmSignature,
});
```

**Epoch settlement** (per provider, every 10 minutes):
```typescript
const epochCommitment = await uc.submitStateTransition({
  tokenId: providerTokenId,
  transitionData: { epochId, receiptsRoot, totalTokens, amountMicroUSD },
  ownerProof: providerSignature,
});
// epochCommitment.inclusionProof → submitted to EVM escrow contract
```

**Payment single-spend token** (per session, consumer SDK):
```typescript
const paymentToken = await uc.createToken({
  value: sessionMaxAmount,
  recipient: providerPeerId,
  validUntil: Date.now() + 60_000,
});
// Token is single-spend — Unicity SMT prevents double-use
```

### 6.4 EVM Escrow Contract (Base L2)

The escrow contract is a minimal Solidity contract deployed on Base. Its interface:

```solidity
interface ISparkleEscrow {
    // Consumer opens escrow at session start
    function openSession(bytes32 sessionId, address provider, uint256 maxAmount) external;

    // Provider settles epoch with Unicity inclusion proof
    function settleEpoch(
        bytes32 epochId,
        bytes32 receiptsRoot,
        bytes calldata unicityInclusionProof,
        uint256 totalTokens,
        uint256 amount
    ) external;

    // Consumer claims partial refund after provider crash
    function claimPartialRefund(
        bytes32 sessionId,
        ChunkReceipt calldata lastReceipt,
        bytes calldata consumerSig
    ) external;

    // Either party raises a dispute with Unicity non-deletion proof
    function raiseDispute(
        bytes32 sessionId,
        ChunkReceipt[] calldata receipts,
        bytes calldata unicityNonDeletionProof
    ) external;
}
```

Unicity inclusion proof verification in Solidity uses the `sparsemerkleverifier` pattern: reconstruct the root from the leaf and sibling hashes, compare against the BFT-certified root fetched via oracle. The oracle call is the only centralisation point in the settlement path — a Chainlink or UMA-based oracle providing the latest Unicity BFT root satisfies this dependency.

### 6.5 sparkle-sdk (TypeScript, consumer-facing)

Drop-in OpenAI SDK replacement:

```typescript
import { Sparkle } from '@sparkle/sdk';

// Coordinator path (simple)
const client = new Sparkle({
  baseUrl: 'https://api.sparkle.dev',
  apiKey: 'sk-sparkle-...',
});

// Direct P2P path (trustless)
const client = new Sparkle({
  mode: 'p2p',
  registryUrl: 'https://aggregator.unicity.network',
  wallet: privateKey,
});

// Identical to OpenAI SDK from here
const stream = await client.chat.completions.create({
  model: 'llama-3.3-70b',
  messages: [{ role: 'user', content: 'Hello' }],
  stream: true,
});
```

In P2P mode, the SDK autonomously queries the Unicity registry, verifies inclusion proofs, performs NOISE handshakes, manages chunk receipt signing, and handles payment commitment using Unicity single-spend tokens — zero human interaction required for AI agent use cases.

### 6.6 sparkle-dashboard (Next.js)

Operator web UI providing:

- Real-time earnings (USD + SPARKL token), token throughput, concurrent sessions
- Attestation status: NRAS certificate validity, TPM PCR values, next renewal deadline
- Model management: loaded models, VRAM allocation, llama-swap queue depth
- Unicity registry status: token state, last heartbeat, inclusion proof verification
- Epoch settlement history: receipts root → Unicity inclusion proof → EVM transaction chain
- Network topology: active libp2p peers, DHT routing table, connection quality metrics

***

## 7. Operator Economics

### 7.1 Cost Structure

The DGX Spark's idle power consumption of approximately 40W sets the floor for operator costs. At UK commercial electricity rates (~£0.25/kWh ≈ $0.032/kWh), idle electricity costs approximately $0.013/hour. Under inference load (~150–200W estimated), this rises to approximately $0.006–0.008 per additional hour. These are the only marginal costs for an operator who already owns the hardware.

### 7.2 Revenue Model

At 40% average utilisation (a conservative estimate for a DGX Spark primarily used for personal development with Sparkle running in the background):

| Metric | Conservative | Moderate | Optimistic |
|---|---|---|---|
| Utilisation | 20% | 40% | 70% |
| Avg output tok/s | 30 | 40 | 50 |
| Model tier | Large dense | Large MoE | XL MoE |
| Gross revenue/hr | ~$0.40 | ~$1.20 | ~$3.50 |
| Electricity cost/hr | $0.015 | $0.020 | $0.025 |
| Net margin | ~96% | ~98% | ~99% |
| Monthly net (720hr) | ~$275 | ~$850 | ~$2,480 |

Hardware payback at moderate utilisation: approximately 5–6 months. At optimistic utilisation serving large MoE models: approximately 2 months. These projections assume the Sparkle network achieves sufficient demand routing — early operators will see lower utilisation; the economics improve as the network grows.

### 7.3 Dual-Node Pairing

Two DGX Spark units connected via their ConnectX-7 NICs can register as a single 405B-capable endpoint in the Sparkle network. This tier is not available from any other consumer-grade hardware and commands premium pricing ($0.20 input / $1.80 output per million tokens). A dual-node operator achieves roughly 2.5× the revenue of a single-node operator while sharing the discovery and attestation overhead of a single registration.

***

## 8. Competitive Landscape

### 8.1 Comparable Networks

| Platform | Hardware | Billing | Routing | Streaming | Privacy | Chain |
|---|---|---|---|---|---|---|
| **Darkbloom** | Apple Silicon | Stripe + internal ledger | Centralised coordinator | ✅ SSE | HW attestation (Apple SE) | None |
| **Sparkle** | NVIDIA DGX Spark | x402 + Unicity + EVM escrow | Coordinator + P2P DHT | ✅ QUIC streams | HW attestation (NRAS + TPM2) | None (Unicity + Base) |
| **Akash** | Any GPU (container) | Escrow drip (per second) | Reverse auction SDL | ❌ Container workloads | TEE (roadmap) | Cosmos |
| **Nosana** | Any GPU | Per-job NOS token | Smart contract queue | ❌ Batch jobs | Encryption only | Solana |
| **Bittensor** | Any GPU | TAO emissions (per epoch) | Validator scoring | ❌ Async | None | Custom L1 |
| **Vast.ai** | Any GPU (hosted) | Hourly rate | Centralised matching | ✅ | Contractual only | None |
| **io.net** | Cluster GPU | Per-cluster-hour | Ray graph topology | ❌ Training/batch | Contractual | Solana |

### 8.2 Differentiation

Sparkle's differentiation is the intersection of three properties that no other network currently provides simultaneously:

1. **Hardware-verified privacy at the inference level**: not contractual, not TEE-in-progress — provable through NRAS + TPM2 attestation with a public verification path
2. **200B+ parameter model serving from consumer hardware**: enabled by DGX Spark's 128 GB unified memory; no other consumer-grade device at this price point matches this capability
3. **Fully decentralised trustless discovery**: the Unicity registry provides complete, verifiable, sybil-resistant provider enumeration with cryptographic proofs — no central coordinator required for discovery or dispute resolution

***

## 9. Path to EigenLayer AVS

Eigen Labs, the builders of Darkbloom and EigenLayer, have explicitly positioned AI inference as a supported Actively Validated Service use case. Two live examples already exist: Kite AI (EigenLayer restaking for AI inference verification) and Ritual's Infernet protocol (smart contracts natively calling AI models through EigenLayer-secured oracles).

Sparkle is designed as a natural EigenLayer AVS candidate. The migration path:

1. **Phase 1–2** (current design): Unicity for proof generation, Base EVM for fund custody, platform-operated coordinator
2. **Phase 3**: Register Sparkle as an EigenLayer AVS. DGX Spark operators become EigenLayer operators, restaking ETH as economic collateral for their attestation claims
3. **Phase 4**: Replace the Base EVM escrow oracle dependency with EigenLayer's operator set as the BFT-certified Unicity root relayer — eliminating the oracle centralisation point entirely

At Phase 3, the trust model upgrades from "cryptographic proof + hardware bond" to "cryptographic proof + hardware bond + $16B+ restaked ETH economic security". Consumer-side verification remains identical — the Unicity inclusion proofs are the same; only the economic backstop grows.

This trajectory also enables the natural merger with Darkbloom: a unified EigenLayer AVS serving inference from both Apple Silicon and NVIDIA DGX Spark nodes, with hardware-specific attestation backends abstracted behind a common `VerifyAttestation(report) → NodeCertificate` interface in the coordinator.

***

## 10. Implementation Roadmap

### Phase 1 — Core MVP (Weeks 1–8)

- `sparkle-provider` Rust binary: TPM2 keygen, NaCl Box E2E, WebSocket coordinator tunnel, llama-swap proxy, systemd service, one-line install
- `sparkle-coordinator` TypeScript/Hono: OpenAI-compatible gateway, provider registry cache, request routing, WebSocket tunnels, Stripe billing (mirroring Darkbloom's approach for rapid MVP)
- `sparkle-dashboard` Next.js: operator earnings, model status, basic attestation display
- Testnet: coordinator + 5 DGX Spark nodes, internal testing

### Phase 2 — Privacy and Decentralisation (Weeks 9–16)

- Full NRAS attestation integration (replace stub with live NRAS verification)
- Chunk receipt generation and Ed25519 signing in provider
- Unicity State Transition SDK integration: provider registration, heartbeats, epoch batch submission
- EVM escrow contract deployment on Base testnet: openSession, settleEpoch, claimPartialRefund
- x402 payment flow replacing Stripe for crypto-native consumers
- Unicity single-spend tokens for payment commitments

### Phase 3 — P2P Network (Weeks 17–24)

- libp2p swarm in provider binary: Kademlia DHT, mDNS, NOISE handshake on :30333
- QUIC transport on :9944 with streaming chunk protocol
- P2P mode in `sparkle-sdk`: trustless discovery, direct NOISE handshake, QUIC inference
- Dispute resolution: `raiseDispute` and `resolveDispute` on EVM contract with Unicity non-deletion proofs
- Dual-node pairing: two DGX Sparks register as a single 405B endpoint

### Phase 4 — EigenLayer AVS (Weeks 25–36)

- Operator stake registration on EigenLayer
- AVS smart contract: slashing conditions for attestation failure or confirmed misbehaviour
- Merge coordinator with Darkbloom: unified routing for Apple Silicon + DGX Spark providers
- Unicity BFT root relayer via EigenLayer operator set (eliminates oracle dependency)
- Public mainnet launch

***

## 11. Security Considerations

### 11.1 Known Limitations

**Residual hardware attack**: physical memory probing of the LPDDR5x chips soldered into the GB10 SoC package can theoretically expose decrypted inference data in DRAM. This is the same residual threat accepted by Apple's Private Cloud Compute and by every hardware TEE. Mitigation: memory encryption via TPM-sealed keys reduces the exposure window; inference data is overwritten immediately after use.

**NRAS dependency**: Sparkle's attestation root relies on NVIDIA's Remote Attestation Service remaining available and honest. NVIDIA becomes a trusted third party in the attestation chain. Mitigation: the Unicity registry stores attestation certificate hashes — if NRAS becomes unavailable, existing certificates remain verifiable against stored hashes until expiry; the network degrades gracefully rather than failing.

**Coordinator centralisation (Path A)**: the coordinator-proxied path requires trusting the coordinator with routing metadata (which provider, which model, session duration). The coordinator sees ciphertext it cannot decrypt but can perform traffic analysis. Mitigation: the direct P2P path (Path B) eliminates this; the coordinator is explicitly optional.

**Unicity aggregator availability**: the provider registry depends on Unicity aggregators being available. Unicity's BFT consensus with multiple aggregator nodes provides fault tolerance; the coordinator cache provides availability during aggregator downtime.

### 11.2 Security Assumptions

The Sparkle security model rests on four assumptions:

1. NVIDIA's TPM2 implementation is tamper-resistant and NRAS is honest
2. NaCl Box (X25519 + XSalsa20-Poly1305) is computationally secure
3. Unicity's BFT consensus is honest (requires >2/3 of aggregators to be non-Byzantine)
4. Base L2 settlement is final within Ethereum's economic security bounds

None of these assumptions require trusting Sparkle Labs, the coordinator operator, or any individual provider.

***

## 12. Conclusion

Sparkle addresses a clear market inefficiency: tens of thousands of NVIDIA DGX Spark units represent substantial frontier AI inference capacity that sits idle outside active development hours, while developers and AI agents pay three-layer markups to centralised cloud inference providers. By connecting this idle capacity directly to demand — with hardware-verified privacy, decentralised peer discovery via Unicity's SMT aggregation layer, and fair payment settlement through epoch batching and EVM escrow — Sparkle achieves simultaneous benefits for both sides of the market.

For operators, Sparkle converts idle hardware into a revenue stream with near-100% gross margins and no marginal cost beyond electricity. For consumers, Sparkle delivers 50% cost reduction against centralised providers with stronger privacy guarantees than any cloud inference API offers. For the broader AI ecosystem, Sparkle demonstrates that hardware-verified decentralised inference is achievable today — not as a roadmap item, but as a deployable system built on production-grade components: NVIDIA's existing attestation infrastructure, Unicity's battle-tested aggregation layer, and Darkbloom's open-source NaCl Box provider primitives.

The architecture is explicitly designed to grow: Darkbloom merger in Phase 3, EigenLayer AVS registration in Phase 4. Sparkle is not a standalone product — it is the NVIDIA hardware pillar of a unified decentralised inference network that, once mature, routes requests across the full spectrum of personal AI supercomputing capacity regardless of underlying silicon.

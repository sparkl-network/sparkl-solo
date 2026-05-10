No research needed — this is a direct engineering specification. Here's a complete, cursor-ready implementation brief.

Sparkl Node1 — Prototype Implementation Spec
Project Bootstrap

bash
cargo new Sparkl-node1 --bin
cd Sparkl-node1
Target: aarch64-unknown-linux-gnu (DGX Spark, Ubuntu 24.04 DGX OS)
Cross-compile target for dev: x86_64-unknown-linux-gnu (same binary, different arch)

Cargo.toml

text
[package]
name    = "Sparkl-node1"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "sparkl-node1"
path = "src/main.rs"

[dependencies]
# Async runtime
tokio            = { version = "1", features = ["full"] }
tokio-stream     = "0.1"

# HTTP / WebSocket server
axum             = { version = "0.8", features = ["ws"] }
axum-extra       = { version = "0.10", features = ["typed-header"] }
tower            = "0.5"
tower-http       = { version = "0.6", features = ["cors", "trace"] }

# HTTP client (proxy to llama-swap / vLLM)
reqwest          = { version = "0.12", features = ["json", "stream"] }

# libp2p
libp2p           = { version = "0.55", features = [
    "tokio", "quic", "tcp", "noise", "yamux",
    "kad", "mdns", "identify", "ping"
]}

# Crypto
crypto_box       = "0.9"
ed25519-dalek    = { version = "2", features = ["rand_core"] }
x25519-dalek     = "2"
rand             = "0.8"
sha2             = "0.10"
hex              = "0.4"
base64           = "0.22"
zeroize          = "1"

# TPM2 (optional — feature-gated for dev without TPM hardware)
tss-esapi        = { version = "0.11", optional = true }

# Embedded storage
sled             = "0.34"

# Serialisation
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"

# Config
config           = "0.14"
toml             = "0.8"

# Logging / tracing
tracing          = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Error handling
anyhow           = "1"
thiserror        = "2"

# Utilities
uuid             = { version = "1", features = ["v4"] }
chrono           = { version = "0.4", features = ["serde"] }
dashmap          = "6"
bytes            = "1"
futures          = "0.3"

# HTTP (Unicity + EVM settlement calls)
url              = "2"

[features]
default  = ["tpm"]
tpm      = ["tss-esapi"]
mock-tpm = []   # dev mode: software-emulated TPM, no hardware required
Directory Structure

text
sparkl-node1/
├── Cargo.toml
├── config/
│   └── default.toml          ← default config, shipped with binary
├── src/
│   ├── main.rs               ← entrypoint, wires all modules together
│   ├── config.rs             ← Config struct, load from file + env
│   ├── identity.rs           ← keypair generation, TPM2 or software fallback
│   ├── attestation.rs        ← NRAS registration, challenge-response handler
│   ├── crypto.rs             ← NaCl Box encrypt/decrypt, session key management
│   ├── network/
│   │   ├── mod.rs            ← libp2p Swarm setup, event loop
│   │   ├── behaviour.rs      ← ComposedBehaviour (Kad + mDNS + Identify + Ping)
│   │   └── discovery.rs      ← DHT advertisement, peer event handling
│   ├── server/
│   │   ├── mod.rs            ← axum router setup, shared state
│   │   ├── inference.rs      ← POST /v1/chat/completions handler
│   │   ├── models.rs         ← GET /v1/models handler
│   │   ├── health.rs         ← GET /health, GET /status
│   │   └── middleware.rs     ← auth, logging, error formatting
│   ├── proxy.rs              ← reqwest proxy to llama-swap/vLLM backend
│   ├── session.rs            ← session lifecycle, ChunkReceipt, state machine
│   ├── receipts.rs           ← chunk receipt generation, Ed25519 signing
│   ├── store.rs              ← sled wrappers, session persistence
│   ├── registry.rs           ← Unicity token registration, heartbeat loop
│   ├── settlement.rs         ← epoch batch assembly, EVM settlement call
│   └── error.rs              ← SparklError enum
├── install.sh                ← one-line install script
└── tests/
    ├── integration_test.rs
    └── fixtures/
Module Specifications

config.rs

rust
// Config struct — loaded from default.toml, overlaid with env vars
// Env prefix: SPARKLE_
// e.g. SPARKLE_BACKEND_URL=http://localhost:8000

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub node:        NodeConfig,
    pub network:     NetworkConfig,
    pub backend:     BackendConfig,
    pub attestation: AttestationConfig,
    pub registry:    RegistryConfig,
    pub settlement:  SettlementConfig,
    pub pricing:     PricingConfig,
}

pub struct NodeConfig {
    pub name:         String,        // human-readable, shown in dashboard
    pub data_dir:     PathBuf,       // ~/.sparkl/
    pub log_level:    String,        // "info" default
    pub mode:         NodeMode,      // Solo | Farm (node2)
}

pub struct NetworkConfig {
    pub listen_addrs:  Vec<String>,  // ["/ip4/0.0.0.0/udp/30333/quic-v1", "/ip4/0.0.0.0/tcp/30333"]
    pub inference_port: u16,         // 9944
    pub external_ip:   Option<String>, // if behind NAT, announce this
    pub bootstrap_peers: Vec<String>,  // hardcoded coordinator bootstrap multiaddrs
}

pub struct BackendConfig {
    pub url:           String,       // "http://127.0.0.1:8000" (llama-swap)
    pub health_path:   String,       // "/health"
    pub models_path:   String,       // "/v1/models"
    pub timeout_secs:  u64,          // 120
}

pub struct AttestationConfig {
    pub nras_url:      String,       // "https://nras.attestation.nvidia.com"
    pub nras_enabled:  bool,         // false in mock-tpm feature
    pub cert_ttl_days: u64,          // 7 — refresh before expiry
}

pub struct RegistryConfig {
    pub unicity_aggregator_url: String,  // "https://aggregator.unicity.network"
    pub heartbeat_secs:         u64,     // 30
    pub enabled:                bool,    // false in dev/local mode
}

pub struct SettlementConfig {
    pub epoch_secs:         u64,         // 600 (10 minutes)
    pub evm_rpc_url:        String,      // Base L2 RPC
    pub escrow_contract:    String,      // 0x...
    pub enabled:            bool,        // false in dev/local mode
}

pub struct PricingConfig {
    pub micro_usd_per_m_input_tokens:  u64,   // 100 = $0.10/M
    pub micro_usd_per_m_output_tokens: u64,   // 780 = $0.78/M
}
config/default.toml:

text
[node]
name        = "sparkl-node"
data_dir    = "~/.sparkl"
log_level   = "info"
mode        = "solo"

[network]
listen_addrs   = ["/ip4/0.0.0.0/udp/30333/quic-v1", "/ip4/0.0.0.0/tcp/30333/ws"]
inference_port = 9944
bootstrap_peers = [
    "/dns4/bootstrap.sparkl.dev/tcp/30333/p2p/12D3KooW..."
]

[backend]
url          = "http://127.0.0.1:8000"
health_path  = "/health"
models_path  = "/v1/models"
timeout_secs = 120

[attestation]
nras_url     = "https://nras.attestation.nvidia.com"
nras_enabled = false     # set true on DGX with real TPM
cert_ttl_days = 7

[registry]
unicity_aggregator_url = "https://aggregator.unicity.network"
heartbeat_secs         = 30
enabled                = false   # set true for network participation

[settlement]
epoch_secs      = 600
evm_rpc_url     = "https://mainnet.base.org"
escrow_contract = "0x0000000000000000000000000000000000000000"
enabled         = false   # set true for billing

[pricing]
micro_usd_per_m_input_tokens  = 100
micro_usd_per_m_output_tokens = 780
identity.rs

rust
// Manages the node's cryptographic identity.
// In TPM mode: generates and seals X25519 keypair in TPM2, never touches memory.
// In mock-tpm mode: generates software keypair, persists to ~/.sparkl/identity.json
//
// The keypair is used for:
//   1. libp2p PeerId derivation (NOISE identity)
//   2. NaCl Box decryption (inference request decryption)
//   3. Ed25519 signing for chunk receipts
//   4. Attestation challenge response signing

pub struct NodeIdentity {
    pub peer_id:        PeerId,           // libp2p identity (derived from X25519 pubkey)
    pub x25519_pubkey:  [u8; 32],         // public half — shared with registry
    pub ed25519_pubkey: [u8; 32],         // receipt signing pubkey
}

// Functions to implement:
pub async fn load_or_generate(config: &Config) -> Result<NodeIdentity>
// → checks ~/.sparkl/identity.json (mock) or TPM handle (tpm feature)
// → generates fresh keys if none exist
// → persists public keys to ~/.sparkl/identity.json (public data only)

pub async fn sign_challenge(nonce: &[u8; 32]) -> Result<[u8; 64]>
// → signs with TPM-bound key (tpm feature) or software Ed25519 (mock)
// → used by attestation challenge-response loop

pub async fn decrypt_request(ciphertext: &[u8], ephemeral_pubkey: &[u8; 32]) -> Result<Vec<u8>>
// → NaCl Box open: uses node's X25519 private key + consumer's ephemeral pubkey
// → returns plaintext inference request JSON
attestation.rs

rust
// Handles NRAS registration and ongoing challenge-response.
//
// On startup:
//   1. Collect TPM2 attestation report (PCR values + TPM quote)
//   2. POST to NRAS: { tpm_report, node_pubkey, software_hash }
//   3. Receive NRAS certificate (X.509, signed by NVIDIA root CA)
//   4. Store cert in ~/.sparkl/nras_cert.pem
//   5. Submit cert hash to Unicity registry (done via registry.rs)
//
// Ongoing (every cert_ttl_days - 1 days):
//   - Renew cert before expiry
//   - Re-submit cert hash to Unicity via heartbeat
//
// In mock-tpm mode:
//   - Generates a self-signed "mock attestation cert"
//   - Logs warning: "MOCK ATTESTATION — not valid for mainnet"
//   - All other flows identical

pub struct AttestationState {
    pub cert_pem:       String,
    pub cert_hash:      [u8; 32],
    pub issued_at:      DateTime<Utc>,
    pub expires_at:     DateTime<Utc>,
    pub nras_enabled:   bool,
}

pub async fn register_with_nras(identity: &NodeIdentity, config: &AttestationConfig)
    -> Result<AttestationState>

pub async fn run_renewal_loop(state: Arc<Mutex<AttestationState>>, config: AttestationConfig)
// → spawned as background tokio task
// → sleeps until 24h before expiry, then renews
crypto.rs

rust
// NaCl Box session crypto.
// Consumers encrypt requests using the node's X25519 pubkey + their ephemeral key.
// Wire format (from Darkbloom spec, maintained for compatibility):
//
//   [32 bytes: consumer ephemeral X25519 pubkey]
//   [24 bytes: nonce]
//   [N bytes:  ciphertext (XSalsa20-Poly1305)]
//
// Response encryption:
//   Each chunk encrypted with same session key, incrementing nonce.
//   Allows consumer to decrypt chunks as they arrive without waiting for stream end.

pub struct SessionCrypto {
    shared_secret:  SharedSecret,    // X25519 DH result, stored as crypto_box::SalsaBox
    nonce_counter:  AtomicU64,       // increments per chunk, prevents nonce reuse
}

impl SessionCrypto {
    pub fn from_request_header(header: &[u8], node_identity: &NodeIdentity) -> Result<Self>
    // → extracts consumer ephemeral pubkey from first 32 bytes
    // → performs X25519 DH → derives SalsaBox

    pub fn decrypt_request(&self, ciphertext: &[u8]) -> Result<Vec<u8>>

    pub fn encrypt_chunk(&self, plaintext: &[u8]) -> Result<Vec<u8>>
    // → uses auto-incrementing nonce, thread-safe via AtomicU64
}
network/mod.rs + network/behaviour.rs

rust
// libp2p swarm setup.
//
// Protocols enabled:
//   - QUIC transport (primary, :30333/udp)
//   - TCP transport (fallback, :30333/tcp)
//   - NOISE authentication (using node's identity keypair)
//   - Yamux multiplexing (over TCP only — QUIC has native streams)
//   - Kademlia DHT (address resolution for known PeerIDs)
//   - mDNS (LAN peer discovery — finds other Sparkl nodes on same network)
//   - Identify (exchange capabilities on connect)
//   - Ping (liveness checks)

#[derive(NetworkBehaviour)]
pub struct SparklNetworkBehaviour {
    pub kademlia:  kad::Behaviour<kad::store::MemoryStore>,
    pub mdns:      mdns::tokio::Behaviour,
    pub identify:  identify::Behaviour,
    pub ping:      ping::Behaviour,
}

// Swarm event loop — runs as background tokio task.
// On mDNS discovery: adds peer to Kademlia routing table.
// On Identify response: logs peer capabilities, updates known peers.
// On Ping timeout: removes peer from routing table.
// Exposes channel for querying known peers from other modules.

pub async fn start_swarm(
    identity: &NodeIdentity,
    config: &NetworkConfig,
) -> Result<(Swarm<SparklNetworkBehaviour>, mpsc::Sender<SwarmCommand>)>
server/inference.rs

rust
// POST /v1/chat/completions — the core inference handler.
//
// Request flow:
//
//   1. Receive POST with body:
//      {
//        "encrypted": true,              // if present, body is NaCl Box ciphertext
//        "epk": "<base64 X25519 pubkey>",// consumer's ephemeral pubkey
//        "ciphertext": "<base64>"        // encrypted OpenAI request JSON
//      }
//      OR standard OpenAI JSON (unencrypted, for dev/local mode)
//
//   2. If encrypted:
//      - Derive SessionCrypto from epk + node identity
//      - Decrypt ciphertext → plaintext OpenAI request JSON
//
//   3. Validate request (model exists in backend, context within limits)
//
//   4. Open session in SessionManager:
//      - Assign session_id (UUID v4)
//      - Record start time, model, consumer identity
//      - Lock escrow check (if billing enabled)
//
//   5. Forward to proxy.rs → reqwest stream to llama-swap :8000
//
//   6. Stream response back to consumer as SSE:
//      For each chunk from backend:
//        a. Encrypt chunk (if session is encrypted)
//        b. Append chunk to session receipt buffer
//        c. Every 50 tokens OR 2 seconds: generate ChunkReceipt, sign it
//        d. Embed signed receipt in SSE `data:` field alongside chunk
//           (consumer can strip it; it's base64 appended to the JSON line)
//        e. Send SSE chunk to consumer
//
//   7. On stream complete:
//      - Send final SSE: data: [DONE]
//      - Close session, record total tokens, final receipt
//      - Hand session to settlement module
//
//   8. On consumer disconnect mid-stream:
//      - Record last acknowledged receipt
//      - Mark session as "consumer_disconnected"
//      - Settlement uses last acknowledged receipt for partial claim
//
//   9. On backend timeout / error:
//      - Send SSE error event to consumer
//      - Mark session as "provider_error"
//      - Do not bill for incomplete session

// SSE chunk format (encrypted):
// data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[...],"sparkl":{"seq":42,"receipt":"<base64 ChunkReceipt>"}}
//
// The "sparkl" field is ignored by standard OpenAI SDK clients.
session.rs

rust
// Session state machine.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionState {
    Opening,
    Active,
    Completed,
    ConsumerDisconnected,   // consumer dropped mid-stream
    ProviderError,          // backend failed mid-stream
    Disputed,               // consumer raised a dispute
    Settled,                // epoch settlement confirmed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id:               Uuid,
    pub consumer_pubkey:  Option<[u8; 32]>,   // None if unencrypted (dev mode)
    pub model:            String,
    pub state:            SessionState,
    pub started_at:       DateTime<Utc>,
    pub ended_at:         Option<DateTime<Utc>>,
    pub tokens_input:     u64,
    pub tokens_output:    u64,
    pub receipts:         Vec<ChunkReceipt>,   // in-memory during session
    pub last_receipt_seq: u64,
    pub amount_micro_usd: u64,                 // computed at session end
}

// SessionManager: DashMap<Uuid, Arc<Mutex<Session>>>
// Global shared state, Arc'd into axum AppState
pub struct SessionManager {
    sessions:  DashMap<Uuid, Arc<Mutex<Session>>>,
    store:     Arc<Store>,   // sled persistence for crash recovery
}

impl SessionManager {
    pub fn open(&self, model: &str, consumer_pubkey: Option<[u8; 32]>) -> Uuid
    pub fn record_chunk(&self, id: Uuid, tokens: u32, content_hash: [u8; 32])
    pub fn close(&self, id: Uuid, state: SessionState)
    pub fn pending_settlement(&self) -> Vec<Session>   // sessions awaiting epoch
    pub fn recover_from_store(&self) -> Result<()>     // on startup: load incomplete sessions
}
receipts.rs

rust
// ChunkReceipt generation and signing.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkReceipt {
    pub session_id:    Uuid,
    pub provider_id:   String,          // libp2p PeerId as string
    pub seq:           u64,             // monotonic checkpoint index
    pub token_count:   u64,             // cumulative output tokens at this checkpoint
    pub content_hash:  [u8; 32],        // SHA-256 of all ciphertext chunks in this window
    pub timestamp_ms:  u64,
    pub provider_sig:  [u8; 64],        // Ed25519 signature over above fields (canonical JSON)
}

// Called by inference handler every RECEIPT_INTERVAL_TOKENS (50) or RECEIPT_INTERVAL_MS (2000)
pub fn generate_receipt(
    session: &Session,
    identity: &NodeIdentity,
    chunk_window_hash: [u8; 32],
) -> Result<ChunkReceipt>

// Serialise receipt for embedding in SSE stream
pub fn encode_receipt_for_sse(receipt: &ChunkReceipt) -> String  // base64(json)

// Verify a consumer-signed receipt (used in dispute resolution)
pub fn verify_consumer_receipt(
    receipt: &ChunkReceipt,
    consumer_pubkey: &[u8; 32],
    consumer_sig: &[u8; 64],
) -> bool
store.rs

rust
// sled-backed persistence. Two trees:
//
//   "sessions" tree:  session_id (bytes) → Session JSON
//   "receipts" tree:  session_id+seq     → ChunkReceipt JSON
//   "epochs" tree:    epoch_id           → EpochBatch JSON
//   "identity" tree:  "node_identity"    → NodeIdentity public data JSON
//
// Purpose:
//   - Crash recovery: reload in-flight sessions on restart
//   - Dispute evidence: receipts persisted for 7 days post-settlement
//   - Epoch audit log

pub struct Store {
    db: sled::Db,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self>
    pub fn save_session(&self, session: &Session) -> Result<()>
    pub fn load_session(&self, id: Uuid) -> Result<Option<Session>>
    pub fn save_receipt(&self, receipt: &ChunkReceipt) -> Result<()>
    pub fn receipts_for_session(&self, id: Uuid) -> Result<Vec<ChunkReceipt>>
    pub fn save_epoch(&self, epoch: &EpochBatch) -> Result<()>
    pub fn prune_old_sessions(&self, older_than: Duration) -> Result<u64>
    // → called on startup: removes sessions settled >7 days ago
}
proxy.rs

rust
// reqwest-based proxy to the inference backend (llama-swap / vLLM / Ollama).
//
// Translates between Sparkl's decrypted request format and the backend's
// OpenAI-compatible API. Returns a streaming response as a tokio Stream of Bytes.
//
// Handles:
//   - Connection pooling (reqwest::Client is reused across requests)
//   - Backend health check on startup (retries 5× with backoff)
//   - Model availability validation (GET /v1/models → check requested model exists)
//   - Timeout: config.backend.timeout_secs
//   - Token counting: parses "usage" field from final SSE chunk
//   - Error translation: 503 from backend → SparklError::BackendUnavailable

pub struct BackendProxy {
    client:   reqwest::Client,
    base_url: Url,
}

impl BackendProxy {
    pub async fn check_health(&self) -> Result<()>
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>>
    pub async fn stream_completion(
        &self,
        request: serde_json::Value,
    ) -> Result<impl Stream<Item = Result<Bytes>>>
}
registry.rs

rust
// Unicity token registration and heartbeat.
// When registry.enabled = false: no-op (dev/local mode).
//
// On startup:
//   1. Derive Unicity token ID from node's X25519 pubkey (SHA-256 hash)
//   2. Build ProviderState { peer_id, multiaddrs, models, attestation_hash, pricing, ... }
//   3. POST to Unicity aggregator: mint or update token
//   4. Store returned inclusion proof in ~/.sparkl/unicity_proof.json
//
// Heartbeat loop (every heartbeat_secs):
//   1. Fetch current model list from backend proxy
//   2. Build fresh ProviderState
//   3. POST state transition to Unicity aggregator
//   4. Update stored inclusion proof
//
// Inclusion proof is embedded in the attestation field of x402 PAYMENT-REQUIRED responses.

#[derive(Serialize, Deserialize)]
pub struct ProviderState {
    pub peer_id:            String,
    pub multiaddrs:         Vec<String>,
    pub models:             Vec<String>,
    pub attestation_hash:   String,       // hex-encoded NRAS cert hash
    pub gpu_memory_gb:      u32,          // 128
    pub price_input_m:      u64,          // micro_usd_per_m_input_tokens
    pub price_output_m:     u64,
    pub node_version:       String,       // "1"
    pub last_seen_ms:       u64,
}

pub async fn register(identity: &NodeIdentity, state: &ProviderState, config: &RegistryConfig)
    -> Result<InclusionProof>

pub async fn run_heartbeat_loop(
    identity: Arc<NodeIdentity>,
    proxy:    Arc<BackendProxy>,
    config:   RegistryConfig,
)  // spawned as background tokio task
settlement.rs

rust
// Epoch batch settlement.
// When settlement.enabled = false: logs earnings summary only (dev mode).
//
// Epoch loop (every epoch_secs = 600):
//   1. Collect all Settled sessions from SessionManager since last epoch
//   2. Build EpochBatch:
//        { epoch_id, provider_id, sessions: Vec<SessionSummary>,
//          total_tokens_input, total_tokens_output, total_micro_usd,
//          receipts_root: MerkleRoot }
//   3. Submit EpochBatch as Unicity state transition → get inclusion proof
//   4. Call EVM escrow contract: settleEpoch(epoch_id, receipts_root, unicity_proof, amount)
//   5. Store epoch + tx_hash in store
//   6. Log: "Epoch settled: $X.XX USDC, N sessions, tx: 0x..."
//
// MerkleRoot: SHA-256 binary Merkle tree over all ChunkReceipt hashes in the epoch.
// Leaf = SHA-256(canonical_json(receipt))

#[derive(Serialize, Deserialize)]
pub struct EpochBatch {
    pub epoch_id:              u64,
    pub provider_peer_id:      String,
    pub session_count:         u32,
    pub total_tokens_output:   u64,
    pub total_micro_usd:       u64,
    pub receipts_root:         [u8; 32],   // Merkle root
    pub started_at:            DateTime<Utc>,
    pub ended_at:              DateTime<Utc>,
}

pub async fn run_epoch_loop(
    sessions:   Arc<SessionManager>,
    store:      Arc<Store>,
    identity:   Arc<NodeIdentity>,
    config:     SettlementConfig,
)  // spawned as background tokio task
server/health.rs

rust
// GET /health
// → 200 { "status": "ok", "version": "0.1.0" }  (always — liveness probe)

// GET /status
// → 200 {
//     "peer_id":          "12D3KooW...",
//     "uptime_secs":      12345,
//     "attestation":      { "valid": true, "expires_at": "2026-05-17T..." },
//     "registry":         { "registered": true, "last_heartbeat": "...", "token_id": "0x..." },
//     "backend":          { "healthy": true, "models": ["llama-3.3-70b"] },
//     "sessions_active":  3,
//     "sessions_total":   142,
//     "tokens_served":    1234567,
//     "usdc_earned":      "12.34",
//     "sparkl_earned":    "142.0",
//     "peers_known":      18,
//     "settlement":       { "last_epoch": 12, "last_tx": "0x...", "pending_usdc": "1.23" }
//   }
// → Used by sparkl-dashboard
main.rs

rust
// Startup sequence — this is the wiring. Each module starts up in order:
//
//  1. Load config (file + env overlay)
//  2. Initialise tracing subscriber (json format for systemd journal)
//  3. Open sled store
//  4. Load or generate node identity (TPM or mock)
//  5. Start attestation (NRAS or mock cert)
//  6. Start libp2p swarm → background task
//  7. Start backend proxy health check (retry loop, fail-fast if backend unreachable)
//  8. Start Unicity registry → background task (if enabled)
//  9. Start settlement epoch loop → background task (if enabled)
// 10. Recover incomplete sessions from store
// 11. Start axum server on :9944
// 12. Log: "sparkl-node1 ready — PeerId: 12D3KooW... — listening on :9944"
//
// Graceful shutdown on SIGTERM / SIGINT:
//  - Stop accepting new sessions
//  - Wait for active sessions to complete (max 30s)
//  - Run final partial epoch settlement for completed sessions
//  - Flush sled store
//  - Exit 0
Feature Flags for Development

text
cargo run                          # mock-tpm, registry disabled, settlement disabled
cargo run --features tpm           # real TPM2 (requires tss2 libs on DGX)
SPARKLE_REGISTRY_ENABLED=true      # register on Unicity (testnet)
SPARKLE_SETTLEMENT_ENABLED=true    # submit to Base Sepolia testnet
SPARKLE_ATTESTATION_NRAS_ENABLED=true  # real NRAS (requires NVIDIA hardware)
This lets you build and test the entire node — inference proxying, chunk receipts, session management, libp2p networking — on a laptop without a TPM or DGX hardware. The mock-tpm feature generates a software keypair that behaves identically to the TPM path for all protocol logic.

install.sh (one-liner install)

bash
#!/usr/bin/env bash
set -euo pipefail

ARCH=$(uname -m)  # aarch64 or x86_64
VERSION="0.1.0"
BINARY_URL="https://releases.sparkl.dev/${VERSION}/sparkl-node1-${ARCH}"
DATA_DIR="$HOME/.sparkl"

echo "Installing sparkl-node1 ${VERSION}..."

mkdir -p "$DATA_DIR"
curl -fsSL "$BINARY_URL" -o /usr/local/bin/sparkl-node1
chmod +x /usr/local/bin/sparkl-node1

# Write default config if none exists
[ -f "$DATA_DIR/config.toml" ] || sparkl-node1 --print-default-config > "$DATA_DIR/config.toml"

# Write systemd unit
cat > /etc/systemd/system/sparkl-node1.service <<EOF
[Unit]
Description=Sparkl Node1 — DGX Spark Inference Provider
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$USER
ExecStart=/usr/local/bin/sparkl-node1 --config $DATA_DIR/config.toml
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
WatchdogSec=60

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now sparkl-node1

echo ""
echo "sparkl-node1 installed and running."
echo "Check status:  sparkl-node1 status"
echo "View logs:     journalctl -u sparkl-node1 -f"
echo "Dashboard:     http://localhost:9944/status"
Cursor Agent Prompt (start here)

Paste this as the initial prompt in a new Cursor project after creating the repo structure:

Implement the sparkl-node1 Rust binary according to the module specifications in this project. Start with config.rs and identity.rs (mock-tpm feature only — no tss-esapi yet), then store.rs, then proxy.rs, then session.rs and receipts.rs, then server/ (health first, then inference handler), then network/ (libp2p swarm), then registry.rs and settlement.rs as stubs that log-only when disabled.

All modules must compile under cargo build --features mock-tpm. Do not implement TPM2 hardware bindings yet. The inference handler must correctly proxy streaming SSE responses from a local llama-swap or Ollama instance at http://127.0.0.1:11434 (Ollama default) or http://127.0.0.1:8000 (llama-swap default).

Write integration tests in tests/integration_test.rs that spin up a mock backend (axum test server returning fake SSE), send a plaintext (unencrypted) inference request, and verify chunks arrive with valid ChunkReceipts embedded.

---

## Runtime test note

For the active `sparkl-solo` repo, use `tests-js/` for recurring runtime checks:

```bash
cd tests-js
yarn install
yarn status
yarn attestation
yarn encrypted
yarn tpm:suite
```
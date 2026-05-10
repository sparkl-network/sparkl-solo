use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use chrono::Utc;
use crypto_box::aead::Aead;
use crypto_box::{PublicKey as CryptoPublicKey, SalsaBox, SecretKey};
use futures::stream;
use rand::RngCore;
use reqwest::Client;
use serde_json::{json, Value};
use serial_test::serial;
use sparkl_solo::config::{
    AttestationConfig, BackendConfig, Config, NetworkConfig, NodeConfig, NodeMode, PricingConfig,
    RegistryConfig, SettlementConfig,
};
use sparkl_solo::identity::{self, NodeIdentity};
use sparkl_solo::network::{self, SwarmCommand};
use sparkl_solo::proxy::BackendProxy;
use sparkl_solo::receipts::{ChunkReceipt, UnicityProof};
use sparkl_solo::server::{self, AppState};
use sparkl_solo::session::SessionManager;
use sparkl_solo::store::Store;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::sleep;

async fn backend_health() -> Json<Value> {
    Json(json!({"ok": true}))
}

async fn backend_models() -> Json<Value> {
    Json(json!({"data": [{"id": "mock-model"}]}))
}

async fn backend_chat() -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>>
{
    let chunks = vec![
        Ok(Event::default().data(r#"{"id":"a","object":"chat.completion.chunk","choices":[{"delta":{"content":"hello"}}]}"#)),
        Ok(Event::default().data(r#"{"id":"a","object":"chat.completion.chunk","choices":[{"delta":{"content":" world"}}]}"#)),
        Ok(Event::default().data("[DONE]")),
    ];
    Sse::new(stream::iter(chunks))
}

async fn spawn(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr
}

fn reserve_tcp_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    listener.local_addr().expect("local addr").port()
}

fn test_config(backend_addr: SocketAddr, temp_dir: &TempDir) -> Config {
    Config {
        node: NodeConfig {
            name: "test-node".to_string(),
            data_dir: temp_dir.path().join("data"),
            log_level: "info".to_string(),
            mode: NodeMode::Solo,
            receipt_cadence_tokens: 1,
            include_models: vec![],
            exclude_models: vec![],
        },
        network: NetworkConfig {
            listen_addrs: vec![],
            inference_port: 0,
            external_ip: None,
            public_addrs: vec![],
            bootstrap_peers: vec![],
            public_addr: vec![],
            expose_status_detail: false,
            allow_non_globals_in_dht: true,
        },
        backend: BackendConfig {
            url: format!("http://{}", backend_addr),
            health_path: "/health".to_string(),
            models_path: "/v1/models".to_string(),
            timeout_secs: 20,
        },
        attestation: AttestationConfig {
            nras_url: "https://example.com".to_string(),
            nras_enabled: false,
            cert_ttl_days: 7,
        },
        registry: RegistryConfig {
            unicity_aggregator_url: "https://example.com".to_string(),
            heartbeat_secs: 30,
            enabled: false,
        },
        settlement: SettlementConfig {
            epoch_secs: 600,
            evm_rpc_url: "https://example.com".to_string(),
            escrow_contract: "0x0".to_string(),
            enabled: false,
        },
        pricing: PricingConfig {
            micro_usd_per_m_input_tokens: 100,
            micro_usd_per_m_output_tokens: 780,
        },
    }
}

async fn assert_receipts_present(resp: reqwest::Response) {
    assert!(resp.status().is_success());
    let body = resp.text().await.expect("body");
    let mut verified = 0usize;
    let mut last_seq: Option<u64> = None;

    for line in body.lines().filter(|l| l.starts_with("data: ")) {
        let payload = line.trim_start_matches("data: ").trim();
        if payload == "[DONE]" {
            continue;
        }
        let chunk: Value = serde_json::from_str(payload).expect("chunk json");
        let receipt_b64 = chunk
            .get("sparkl")
            .and_then(|s| s.get("receipt"))
            .and_then(Value::as_str)
            .expect("receipt exists");

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(receipt_b64)
            .expect("decode receipt");
        let receipt: ChunkReceipt = serde_json::from_slice(&decoded).expect("receipt json");
        assert!(receipt.seq > 0);
        assert_eq!(receipt.session_id.to_string().len(), 36);
        if let Some(prev) = last_seq {
            assert_eq!(
                receipt.seq,
                prev + 1,
                "receipt sequence must increment by one"
            );
        }
        last_seq = Some(receipt.seq);
        verified += 1;
    }

    assert!(
        verified >= 2,
        "expected at least two receipt-bearing chunks"
    );
}

#[tokio::test]
#[serial]
async fn status_exposes_minimal_public_fields() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let cfg = test_config(backend_addr, &temp_dir);

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity,
        proxy,
        sessions,
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let client = Client::new();
    let resp = client
        .get(format!("http://{}/status", node_addr))
        .send()
        .await
        .expect("send");
    assert!(resp.status().is_success());
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("ok"));
    assert!(body.get("ready").and_then(Value::as_bool).is_some());
    assert!(
        body.get("peer_id").is_none(),
        "public /status must not expose peer details"
    );
    assert!(
        body.get("identity").is_none(),
        "public /status must not expose identity keys"
    );

    let detail_resp = client
        .get(format!("http://{}/status/detail", node_addr))
        .send()
        .await
        .expect("send");
    assert_eq!(detail_resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
#[serial]
async fn status_detail_is_available_when_enabled() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let mut cfg = test_config(backend_addr, &temp_dir);
    cfg.network.expose_status_detail = true;

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity,
        proxy,
        sessions,
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let client = Client::new();
    let resp = client
        .get(format!("http://{}/status/detail", node_addr))
        .send()
        .await
        .expect("send");
    assert!(resp.status().is_success());
    let body: Value = resp.json().await.expect("json body");
    assert!(body.get("peer_id").and_then(Value::as_str).is_some());
    assert!(body.get("identity").is_some());
}

#[tokio::test]
#[serial]
async fn returns_stored_unicity_proof_for_receipt() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let cfg = test_config(backend_addr, &temp_dir);

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity,
        proxy,
        sessions: sessions.clone(),
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let session_id = sessions.open("mock-model", None);
    let stored_proof = UnicityProof {
        request_id: "req-123".to_string(),
        state_id: "state-123".to_string(),
        proof_hex: "deadbeef".to_string(),
        anchored_at_ms: 123456789,
    };
    sessions
        .save_unicity_proof(session_id, 7, &stored_proof)
        .expect("save proof");

    let client = Client::new();
    let resp = client
        .get(format!(
            "http://{}/receipts/proof/{}/{}",
            node_addr, session_id, 7
        ))
        .send()
        .await
        .expect("send");
    assert!(resp.status().is_success());

    let body: Value = resp.json().await.expect("json body");
    let expected_session_id = session_id.to_string();
    assert_eq!(
        body.get("proof_hex").and_then(Value::as_str),
        Some("deadbeef")
    );
    assert_eq!(
        body.get("request_id").and_then(Value::as_str),
        Some("req-123")
    );
    assert_eq!(
        body.get("state_id").and_then(Value::as_str),
        Some("state-123")
    );
    assert_eq!(
        body.get("anchored_at_ms").and_then(Value::as_u64),
        Some(123456789)
    );
    assert_eq!(
        body.get("session_id").and_then(Value::as_str),
        Some(expected_session_id.as_str())
    );
    assert_eq!(body.get("seq").and_then(Value::as_u64), Some(7));
}

#[tokio::test]
#[serial]
async fn lists_models_via_node_endpoint() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let cfg = test_config(backend_addr, &temp_dir);

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity,
        proxy,
        sessions,
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let client = Client::new();
    let resp = client
        .get(format!("http://{}/v1/models", node_addr))
        .send()
        .await
        .expect("send");
    assert!(resp.status().is_success());

    let body: Value = resp.json().await.expect("json body");
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .expect("data array");
    assert!(!data.is_empty());
    assert_eq!(
        data[0].get("id").and_then(Value::as_str),
        Some("mock-model")
    );
}

#[tokio::test]
#[serial]
async fn rejects_unknown_model_before_stream() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let cfg = test_config(backend_addr, &temp_dir);

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity,
        proxy,
        sessions,
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/v1/chat/completions", node_addr))
        .json(&json!({
            "model": "does-not-exist",
            "messages": [{"role":"user","content":"ping"}],
            "stream": true
        }))
        .send()
        .await
        .expect("send");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(
        body.get("type").and_then(Value::as_str),
        Some("model_not_found")
    );
}

#[tokio::test]
#[serial]
async fn proxies_plaintext_and_embeds_receipts() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let cfg = test_config(backend_addr, &temp_dir);

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity,
        proxy,
        sessions,
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/v1/chat/completions", node_addr))
        .json(&json!({
            "model": "mock-model",
            "messages": [{"role":"user","content":"ping"}],
            "stream": true
        }))
        .send()
        .await
        .expect("send");
    assert_receipts_present(resp).await;
}

#[tokio::test]
#[serial]
async fn proxies_encrypted_and_embeds_receipts() {
    let backend = Router::new()
        .route("/health", get(backend_health))
        .route("/v1/models", get(backend_models))
        .route("/v1/chat/completions", post(backend_chat));
    let backend_addr = spawn(backend).await;

    let temp_dir = TempDir::new().expect("tempdir");
    let cfg = test_config(backend_addr, &temp_dir);

    let identity = identity::load_or_generate(&cfg).await.expect("identity");
    let store = Arc::new(Store::open(&cfg.node.data_dir).expect("store"));
    let sessions = Arc::new(SessionManager::new(store));
    let proxy = Arc::new(BackendProxy::new(&cfg.backend).expect("proxy"));

    let app_state = AppState {
        config: cfg.clone(),
        identity: identity.clone(),
        proxy,
        sessions,
        swarm_cmd: None,
        started_at: Utc::now(),
    };
    let node_addr = spawn(server::router(app_state)).await;

    let plaintext = json!({
        "model": "mock-model",
        "messages": [{"role":"user","content":"ping"}],
        "stream": true
    });
    let plaintext_bytes = serde_json::to_vec(&plaintext).expect("serialize plaintext");

    let mut secret_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret_bytes);
    let ephemeral_secret = SecretKey::from(secret_bytes);
    let ephemeral_public = ephemeral_secret.public_key().to_bytes();
    let node_public = CryptoPublicKey::from(identity.x25519_pubkey);
    let box_cipher = SalsaBox::new(&node_public, &ephemeral_secret);

    let mut nonce_bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = crypto_box::Nonce::from_slice(&nonce_bytes);
    let encrypted = box_cipher
        .encrypt(nonce, plaintext_bytes.as_ref())
        .expect("encrypt");
    let mut wire_ciphertext = nonce_bytes.to_vec();
    wire_ciphertext.extend_from_slice(&encrypted);

    let request_body = json!({
        "encrypted": true,
        "epk": base64::engine::general_purpose::STANDARD.encode(ephemeral_public),
        "ciphertext": base64::engine::general_purpose::STANDARD.encode(wire_ciphertext)
    });

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/v1/chat/completions", node_addr))
        .json(&request_body)
        .send()
        .await
        .expect("send");

    assert_receipts_present(resp).await;
}

#[tokio::test]
#[serial]
async fn two_nodes_discover_each_other_with_separate_configs() {
    let temp_dir = TempDir::new().expect("tempdir");
    let port_1 = reserve_tcp_port();
    let port_2 = reserve_tcp_port();
    let data_1 = temp_dir.path().join("node1-data");
    let data_2 = temp_dir.path().join("node2-data");

    let cfg1_path = temp_dir.path().join("node1.toml");
    let cfg2_path = temp_dir.path().join("node2.toml");

    let cfg1_toml = format!(
        r#"[node]
name = "node-1"
data_dir = "{}"
log_level = "info"
mode = "solo"

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/{port_1}"]
inference_port = 9944
bootstrap_peers = []

[backend]
url = "http://127.0.0.1:11434"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 30

[attestation]
nras_url = "https://example.com"
nras_enabled = false
cert_ttl_days = 7

[registry]
unicity_aggregator_url = "https://example.com"
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
evm_rpc_url = "https://example.com"
escrow_contract = "0x0"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
"#,
        data_1.display()
    );
    std::fs::write(&cfg1_path, cfg1_toml).expect("write node1 config");
    let cfg1 = sparkl_solo::config::load(Some(&cfg1_path)).expect("load node1 config");
    let id1 = NodeIdentity {
        peer_id: "test-node-1".to_string(),
        x25519_pubkey: [1u8; 32],
        ed25519_pubkey: [2u8; 32],
    };
    let (swarm1, swarm1_cmd) = network::start_swarm(&id1, &cfg1.network, &cfg1.node.data_dir)
        .await
        .expect("start swarm1");

    let cfg2_toml = format!(
        r#"[node]
name = "node-2"
data_dir = "{}"
log_level = "info"
mode = "solo"

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/{port_2}"]
inference_port = 9945
bootstrap_peers = ["/ip4/127.0.0.1/tcp/{port_1}/p2p/{}"]

[backend]
url = "http://127.0.0.1:11434"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 30

[attestation]
nras_url = "https://example.com"
nras_enabled = false
cert_ttl_days = 7

[registry]
unicity_aggregator_url = "https://example.com"
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
evm_rpc_url = "https://example.com"
escrow_contract = "0x0"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
"#,
        data_2.display(),
        swarm1.peer_id
    );
    std::fs::write(&cfg2_path, cfg2_toml).expect("write node2 config");
    let cfg2 = sparkl_solo::config::load(Some(&cfg2_path)).expect("load node2 config");
    let id2 = NodeIdentity {
        peer_id: "test-node-2".to_string(),
        x25519_pubkey: [3u8; 32],
        ed25519_pubkey: [4u8; 32],
    };
    let (swarm2, swarm2_cmd) = network::start_swarm(&id2, &cfg2.network, &cfg2.node.data_dir)
        .await
        .expect("start swarm2");

    let mut node1_sees_node2 = false;
    let mut node2_sees_node1 = false;

    for _ in 0..30 {
        let (tx1, rx1) = oneshot::channel();
        swarm1_cmd
            .send(SwarmCommand::GetKnownPeers(tx1))
            .await
            .expect("query swarm1");
        let peers1 = rx1.await.expect("receive peers1");

        let (tx2, rx2) = oneshot::channel();
        swarm2_cmd
            .send(SwarmCommand::GetKnownPeers(tx2))
            .await
            .expect("query swarm2");
        let peers2 = rx2.await.expect("receive peers2");

        node1_sees_node2 = peers1.iter().any(|p| p == &swarm2.peer_id);
        node2_sees_node1 = peers2.iter().any(|p| p == &swarm1.peer_id);

        if node1_sees_node2 && node2_sees_node1 {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    assert!(
        node1_sees_node2,
        "node1 never discovered node2; node2 peer id={}",
        swarm2.peer_id
    );
    assert!(
        node2_sees_node1,
        "node2 never discovered node1; node1 peer id={}",
        swarm1.peer_id
    );
}

#[tokio::test]
#[serial]
async fn recovers_active_sessions_after_restart() {
    let temp_dir = TempDir::new().expect("tempdir");
    let store = Arc::new(Store::open(temp_dir.path()).expect("store"));

    let sessions_before = SessionManager::new(store.clone());
    let session_id = sessions_before.open("mock-model", None);
    sessions_before.record_chunk(session_id, 5, [0u8; 32], 1_000_000);

    let sessions_after = SessionManager::new(store);
    sessions_after.recover_from_store().expect("recover");

    assert_eq!(sessions_after.active_count(), 1);
    let recovered = sessions_after.get(session_id).expect("session recovered");
    assert_eq!(recovered.model, "mock-model");
    assert_eq!(recovered.tokens_output, 5);
}

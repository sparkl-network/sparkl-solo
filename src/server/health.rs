use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

use crate::identity;
use crate::network::SwarmCommand;

use super::AppState;

pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn status(State(state): State<AppState>) -> Json<Value> {
    let ready = state.proxy.check_health().await.is_ok();
    Json(json!({
        "status": "ok",
        "ready": ready
    }))
}

pub async fn status_detail(State(state): State<AppState>) -> impl IntoResponse {
    if !state.config.network.expose_status_detail {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (peers_known, peers) = get_peers_snapshot(&state).await;
    let cert_type = identity::attestation_cert_type().unwrap_or("mock-software");
    let attestation = if state.config.attestation.nras_enabled {
        json!({
            "mode": "nras",
            "valid": null,
            "status": "not_yet_verified",
            "expires_at": null,
            "cert_type": cert_type
        })
    } else {
        json!({
            "mode": if cert_type == "swtpm" { "tpm-dev" } else { "mock" },
            "valid": true,
            "status": cert_type,
            "expires_at": null,
            "cert_type": cert_type
        })
    };
    Json(json!({
        "peer_id": state.identity.peer_id,
        "identity": {
            "pubkey": hex::encode(state.identity.x25519_pubkey),
            "x25519_pubkey": hex::encode(state.identity.x25519_pubkey),
            "ed25519_pubkey": hex::encode(state.identity.ed25519_pubkey),
        },
        "uptime_secs": (Utc::now() - state.started_at).num_seconds().max(0),
        "attestation": attestation,
        "registry": {
            "registered": state.config.registry.enabled
        },
        "backend": {
            "url": state.config.backend.url
        },
        "sessions_active": state.sessions.active_count(),
        "peers_known": peers_known,
        "peers": peers,
        "settlement": {
            "enabled": state.config.settlement.enabled
        }
    }))
    .into_response()
}

async fn get_peers_snapshot(state: &AppState) -> (usize, Vec<String>) {
    let Some(swarm_cmd) = state.swarm_cmd.clone() else {
        return (0, Vec::new());
    };

    let (tx, rx) = oneshot::channel();
    if swarm_cmd
        .send(SwarmCommand::GetKnownPeers(tx))
        .await
        .is_err()
    {
        return (0, Vec::new());
    }

    match timeout(Duration::from_millis(500), rx).await {
        Ok(Ok(peers)) => (peers.len(), peers),
        _ => (0, Vec::new()),
    }
}

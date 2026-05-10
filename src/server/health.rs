use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

use crate::network::SwarmCommand;

use super::AppState;

pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn status(State(state): State<AppState>) -> Json<Value> {
    let (peers_known, peers) = get_peers_snapshot(&state).await;
    Json(json!({
        "peer_id": state.identity.peer_id,
        "identity": {
            "pubkey": hex::encode(state.identity.x25519_pubkey),
            "x25519_pubkey": hex::encode(state.identity.x25519_pubkey),
            "ed25519_pubkey": hex::encode(state.identity.ed25519_pubkey),
        },
        "uptime_secs": (Utc::now() - state.started_at).num_seconds().max(0),
        "attestation": {
            "valid": !state.config.attestation.nras_enabled,
            "expires_at": null
        },
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

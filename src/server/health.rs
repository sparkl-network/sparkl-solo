use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};

use super::AppState;

pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn status(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "peer_id": state.identity.peer_id,
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
        "settlement": {
            "enabled": state.config.settlement.enabled
        }
    }))
}

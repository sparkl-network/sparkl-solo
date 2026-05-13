use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::identity::NodeIdentity;
use crate::network::SwarmCommand;
use crate::proxy::BackendProxy;
use crate::session::SessionManager;

pub mod attestation;
pub mod health;
pub mod identity;
pub mod inference;
pub mod models;
pub mod receipts;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub identity: NodeIdentity,
    pub proxy: Arc<BackendProxy>,
    pub sessions: Arc<SessionManager>,
    pub swarm_cmd: Option<mpsc::Sender<SwarmCommand>>,
    pub started_at: DateTime<Utc>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/status", get(health::status))
        .route("/status/detail", get(health::status_detail))
        .route("/identity", get(identity::identity))
        .route("/attestation/challenge", post(attestation::challenge))
        .route("/receipts/verify", post(receipts::verify))
        .route("/receipts/proof/{session_id}/{seq}", get(receipts::proof))
        .route("/v1/models", get(models::list_models))
        .route("/v1/chat/completions", post(inference::chat_completions))
        .with_state(state)
}

pub fn is_model_allowed(state: &AppState, model_id: &str) -> bool {
    state.config.node.is_model_allowed(model_id)
}

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::identity::NodeIdentity;
use crate::proxy::BackendProxy;
use crate::session::SessionManager;

pub mod health;
pub mod inference;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub identity: NodeIdentity,
    pub proxy: Arc<BackendProxy>,
    pub sessions: Arc<SessionManager>,
    pub started_at: DateTime<Utc>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/status", get(health::status))
        .route("/v1/chat/completions", post(inference::chat_completions))
        .with_state(state)
}

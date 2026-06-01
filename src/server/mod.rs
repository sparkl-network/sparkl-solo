use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use crate::attestation::NrasRuntimeState;
use crate::config::Config;
use crate::identity::NodeIdentity;
use crate::network::SwarmCommand;
use crate::proxy::BackendProxy;
use crate::session::SessionManager;

pub mod attestation;
pub mod auth;
pub mod health;
pub mod identity;
pub mod inference;
pub mod models;
pub mod receipts;
pub mod settlement;
pub mod tee;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub identity: NodeIdentity,
    pub proxy: Arc<BackendProxy>,
    pub sessions: Arc<SessionManager>,
    pub swarm_cmd: Option<mpsc::Sender<SwarmCommand>>,
    pub started_at: DateTime<Utc>,
    pub nras_state: Arc<RwLock<NrasRuntimeState>>,
}

pub fn router(state: AppState) -> Router {
    let settlement_auth = state.config.settlement.enabled;

    let mut openai = Router::new()
        .route("/v1/models", get(models::list_models))
        .route("/v1/chat/completions", post(inference::chat_completions));

    if settlement_auth {
        openai = openai.route_layer(from_fn_with_state(
            state.clone(),
            auth::require_session_bearer,
        ));
    }

    Router::new()
        .route("/health", get(health::health))
        .route("/status", get(health::status))
        .route("/status/detail", get(health::status_detail))
        .route("/identity", get(identity::identity))
        .route("/attestation/challenge", post(attestation::challenge))
        .route("/receipts/verify", post(receipts::verify))
        .route("/receipts/proof/{session_id}/{seq}", get(receipts::proof))
        .merge(openai)
        .route("/tee/verify", post(tee::verify_quote))
        .route("/settlement/deposit-dot", post(settlement::deposit_dot))
        .route("/settlement/deposit-usdc", post(settlement::deposit_usdc_as_dot))
        .route("/settlement/withdraw-dot", post(settlement::withdraw_dot))
        .route("/settlement/withdraw-provider", post(settlement::withdraw_provider))
        .with_state(state)
}

pub fn is_model_allowed(state: &AppState, model_id: &str) -> bool {
    state.config.node.is_model_allowed(model_id)
}

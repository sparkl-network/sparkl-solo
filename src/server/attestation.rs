use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::identity;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct ChallengeRequest {
    pub nonce: String,
}

pub async fn challenge(
    State(state): State<AppState>,
    Json(req): Json<ChallengeRequest>,
) -> Response {
    let nonce_bytes = match hex::decode(&req.nonce) {
        Ok(v) if v.len() == 32 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "nonce must be 32-byte hex string" })),
            )
                .into_response()
        }
    };
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&nonce_bytes);

    let signature = match identity::sign_challenge(&nonce).await {
        Ok(sig) => sig,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("signing failed: {err}") })),
            )
                .into_response()
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "provider_id": state.identity.peer_id,
            "nonce": req.nonce,
            "signature": hex::encode(signature),
            "attestation": {
                "cert_type": "mock-software"
            }
        })),
    )
        .into_response()
}

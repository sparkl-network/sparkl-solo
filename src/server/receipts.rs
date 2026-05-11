use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use crate::receipts::{self, ChunkReceipt};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct VerifyReceiptRequest {
    pub receipt: String,
    pub provider_pubkey: String,
}

pub async fn verify(
    State(_state): State<AppState>,
    Json(req): Json<VerifyReceiptRequest>,
) -> Response {
    let receipt_bytes = match base64::engine::general_purpose::STANDARD.decode(&req.receipt) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "valid": false, "reason": "invalid_receipt_base64" })),
            )
                .into_response()
        }
    };
    let receipt: ChunkReceipt = match serde_json::from_slice(&receipt_bytes) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "valid": false, "reason": "invalid_receipt_json" })),
            )
                .into_response()
        }
    };

    let provider_key_vec = match hex::decode(&req.provider_pubkey) {
        Ok(v) if v.len() == 32 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "valid": false, "reason": "invalid_provider_pubkey" })),
            )
                .into_response()
        }
    };
    let mut provider_pubkey = [0u8; 32];
    provider_pubkey.copy_from_slice(&provider_key_vec);

    let valid = receipts::verify_provider_receipt(&receipt, &provider_pubkey);
    let reason = if valid {
        "signature_ok"
    } else {
        "signature_invalid"
    };
    (
        StatusCode::OK,
        Json(json!({ "valid": valid, "reason": reason })),
    )
        .into_response()
}

pub async fn proof(
    Path((session_id, seq)): Path<(String, u64)>,
    State(state): State<AppState>,
) -> Response {
    let session_uuid = match uuid::Uuid::parse_str(&session_id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_session_id" })),
            )
                .into_response()
        }
    };

    match state.sessions.get_unicity_proof(session_uuid, seq) {
        Ok(Some(proof)) => (
            StatusCode::OK,
            Json(json!({
                "session_id": session_id,
                "seq": seq,
                "proof_hex": proof.proof_hex,
                "request_id": if proof.request_id.is_empty() { serde_json::Value::Null } else { json!(proof.request_id) },
                "state_id": if proof.state_id.is_empty() { serde_json::Value::Null } else { json!(proof.state_id) },
                "anchored_at_ms": if proof.anchored_at_ms == 0 { serde_json::Value::Null } else { json!(proof.anchored_at_ms) }
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "proof_not_found" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed_to_load_proof: {err}") })),
        )
            .into_response(),
    }
}

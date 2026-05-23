use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::Json as JsonRequest;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing::info;

use crate::tee_verification;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct VerifyQuoteRequest {
    pub quote: String,
    pub vendor: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyQuoteResponse {
    pub valid: bool,
    pub canonical_hash: Option<String>,
    pub vendor: String,
    pub error: Option<String>,
}

/// Verify a TEE quote and return its canonical hash.
///
/// This endpoint is used by consumers to validate a provider's
/// TEE quote before trusting receipt proofs. It performs
/// vendor-specific verification and returns the canonical
/// 32-byte SHA-256 hash of the verified quote.
pub async fn verify_quote(
    _state: State<AppState>,
    JsonRequest(req): JsonRequest<VerifyQuoteRequest>,
) -> (StatusCode, Json<VerifyQuoteResponse>) {
    let vendor: tee_verification::TeeVendor =
        match tee_verification::TeeVendor::from_str(&req.vendor) {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(VerifyQuoteResponse {
                        valid: false,
                        canonical_hash: None,
                        vendor: req.vendor,
                        error: Some("invalid vendor string".to_string()),
                    }),
                )
            }
        };

    // Decode the base64-encoded quote
    let quote_bytes = match base64::engine::general_purpose::STANDARD.decode(&req.quote) {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(VerifyQuoteResponse {
                    valid: false,
                    canonical_hash: None,
                    vendor: req.vendor,
                    error: Some("quote is not valid base64".to_string()),
                }),
            )
        }
    };

    let tee_quote = tee_verification::TeeQuote {
        vendor,
        quote_bytes,
        quote_b64: req.quote.clone(),
        extended_report: None,
        cert_chain: None,
    };

    // Verify with a zero nonce (nonce verification is done separately via NRAS)
    let zero_nonce = [0u8; 32];
    match tee_verification::verify_quote(&tee_quote, &zero_nonce).await {
        Ok(result) => {
            if result.verified {
                info!(
                    vendor = %result.vendor,
                    hash = %hex::encode(result.quote_hash),
                    "TEE quote verified"
                );
                (
                    StatusCode::OK,
                    Json(VerifyQuoteResponse {
                        valid: true,
                        canonical_hash: Some(hex::encode(result.quote_hash)),
                        vendor: req.vendor,
                        error: None,
                    }),
                )
            } else {
                info!(
                    vendor = %result.vendor,
                    error = ?result.error,
                    "TEE quote verification failed"
                );
                (
                    StatusCode::BAD_REQUEST,
                    Json(VerifyQuoteResponse {
                        valid: false,
                        canonical_hash: Some(hex::encode(result.quote_hash)),
                        vendor: req.vendor,
                        error: result.error,
                    }),
                )
            }
        }
        Err(err) => {
            info!(%err, vendor = %vendor, "TEE quote verification error");
            (
                StatusCode::BAD_REQUEST,
                Json(VerifyQuoteResponse {
                    valid: false,
                    canonical_hash: None,
                    vendor: req.vendor,
                    error: Some(err.to_string()),
                }),
            )
        }
    }
}

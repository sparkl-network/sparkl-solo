use std::convert::Infallible;

use anyhow::{anyhow, Context};
use async_stream::stream;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use futures::StreamExt;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::identity;
use crate::receipts::{encode_receipt_for_sse, generate_receipt_with_tee, hash_chunk, provider_identity};
use crate::session::SessionState;

use super::AppState;

pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<Value>,
) -> Response {
    let (backend_request, consumer_epk) = match decrypt_request_if_needed(request).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid request: {err}") })),
            )
                .into_response();
        }
    };

    let model = match backend_request.get("model").and_then(|m| m.as_str()) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "missing required field: model" })),
            )
                .into_response();
        }
    };

    let available_models = match state.proxy.list_models().await {
        Ok(models) => models,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("backend model listing failed: {err}") })),
            )
                .into_response();
        }
    };
    let available_models = available_models
        .into_iter()
        .filter(|m| super::is_model_allowed(&state, &m.id))
        .collect::<Vec<_>>();
    if !available_models.iter().any(|m| m.id == model) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!("model not found: {model}"),
                "type": "model_not_found"
            })),
        )
            .into_response();
    }

    let proxy = state.proxy.clone();
    let sessions = state.sessions.clone();
    let receipt_cadence = state.config.node.receipt_cadence_tokens.max(1);
    let output_price_per_m = state.config.pricing.micro_usd_per_m_output_tokens;

    let event_stream = stream! {
        // Determine TEE quote hash for this session.
        // When session_security_tier is TeeVerified, attempt to generate a TEE quote.
        let tee_quote_hash = if state.config.node.session_security_tier
            == crate::session::SecurityTier::TeeVerified
        {
            match crate::tee_verification::generate_quote(crate::tee_verification::TeeVendor::Mock).await {
                Ok(quote) => {
                    info!(vendor = %quote.vendor, "TEE quote generated for session");
                    Some(quote.canonical_hash())
                }
                Err(err) => {
                    warn!(%err, "TEE quote generation failed; falling back to best-effort");
                    None
                }
            }
        } else {
            None
        };

        // Attempt to open an on-chain session (graceful degradation if unset).
        // Only available when `evm-settlement` feature is enabled.
        let evm_session_id: Option<u64> = {
            #[cfg(feature = "evm-settlement")]
            {
                if state.config.settlement.enabled {
                    let escrow_addr = state.config.settlement.escrow_contract.clone();
                    let rpc_url = state.config.registry.effective_evm_rpc_url(&state.config.settlement).to_string();
                    let pk = state.config.settlement.evm_provider_wallet_private_key.clone();
                    let min_deposit = state.config.settlement.session_min_deposit;
                    let tier = state.config.node.session_security_tier;

                    match crate::identity::on_chain_node_id_from_identity(&state.identity) {
                        node_id => {
                            match crate::settlement::evm::open_session_on_chain(
                                &escrow_addr,
                                &rpc_url,
                                &pk,
                                node_id,
                                tier,
                                min_deposit,
                            ).await {
                                Ok(id) => {
                                    if id.is_some() {
                                        info!(evm_session_id = ?id, "on-chain session opened");
                                    }
                                    id
                                }
                                Err(err) => {
                                    warn!(%err, "open_session_on_chain failed; session will not be linked to escrow");
                                    None
                                }
                            }
                        }
                    }
                } else {
                    None
                }
            }
            #[cfg(not(feature = "evm-settlement"))]
            {
                None
            }
        };

        let session_id = if let Some(ref hash) = tee_quote_hash {
            sessions.open_with_tee(
                &model,
                consumer_epk,
                state.config.node.session_security_tier,
                Some(*hash),
                evm_session_id,
            )
        } else {
            sessions.open(
                &model,
                consumer_epk,
                state.config.node.session_security_tier,
                evm_session_id,
            )
        };

        let identity = match provider_identity() {
            Ok(i) => i,
            Err(err) => {
                sessions.close(session_id, SessionState::ProviderError);
                yield Ok::<Event, Infallible>(
                    Event::default().data(json!({"error": format!("identity unavailable: {err}")}).to_string())
                );
                return;
            }
        };

        let mut backend = match proxy.stream_completion(backend_request).await {
            Ok(s) => s,
            Err(err) => {
                sessions.close(session_id, SessionState::ProviderError);
                yield Ok::<Event, Infallible>(
                    Event::default().data(json!({"error": format!("backend unavailable: {err}")}).to_string())
                );
                return;
            }
        };

        while let Some(next) = backend.next().await {
            match next {
                Ok(bytes) => {
                    let body = String::from_utf8_lossy(&bytes);
                    for line in body.lines() {
                        if !line.starts_with("data: ") {
                            continue;
                        }
                        let payload = line.trim_start_matches("data: ").trim();
                        if payload == "[DONE]" {
                            sessions.close(session_id, SessionState::Completed);
                            log_session_completion(&sessions, session_id, &model);
                            yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
                            return;
                        }

                        let mut chunk: Value = match serde_json::from_str(payload) {
                            Ok(v) => v,
                            Err(_) => {
                                warn!("dropping non-json chunk from backend");
                                continue;
                            }
                        };
                        let content_hash = hash_chunk(payload.as_bytes());
                        sessions.record_chunk(session_id, 1, content_hash, output_price_per_m);

                        if let Some(session) = sessions.get(session_id) {
                            if session.tokens_output % receipt_cadence == 0 {
                                let receipt = match generate_receipt_with_tee(&session, &identity, content_hash, session.tee_quote_hash) {
                                    Ok(r) => r,
                                    Err(err) => {
                                        warn!(%err, "receipt generation failed");
                                        yield Ok::<Event, Infallible>(Event::default().data(chunk.to_string()));
                                        continue;
                                    }
                                };
                                sessions.add_receipt(session_id, receipt.clone());
                                chunk["sparkl"] = json!({
                                    "seq": receipt.seq,
                                    "receipt": encode_receipt_for_sse(&receipt),
                                });
                            }
                        }
                        yield Ok::<Event, Infallible>(Event::default().data(chunk.to_string()));
                    }
                }
                Err(err) => {
                    sessions.close(session_id, SessionState::ProviderError);
                    yield Ok::<Event, Infallible>(
                        Event::default().data(json!({"error": err.to_string()}).to_string())
                    );
                    return;
                }
            }
        }

        sessions.close(session_id, SessionState::Completed);
        log_session_completion(&sessions, session_id, &model);
        yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
    };

    Sse::new(event_stream).into_response()
}

async fn decrypt_request_if_needed(request: Value) -> anyhow::Result<(Value, Option<[u8; 32]>)> {
    let encrypted = request
        .get("encrypted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !encrypted {
        return Ok((request, None));
    }

    let epk_b64 = request
        .get("epk")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing epk for encrypted request"))?;
    let ciphertext_b64 = request
        .get("ciphertext")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing ciphertext for encrypted request"))?;

    let epk_vec = base64::engine::general_purpose::STANDARD
        .decode(epk_b64)
        .context("epk is not valid base64")?;
    if epk_vec.len() != 32 {
        return Err(anyhow!("epk must decode to exactly 32 bytes"));
    }
    let mut epk = [0u8; 32];
    epk.copy_from_slice(&epk_vec);

    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(ciphertext_b64)
        .context("ciphertext is not valid base64")?;
    let plaintext = identity::decrypt_request(&ciphertext, &epk).await?;
    let parsed =
        serde_json::from_slice::<Value>(&plaintext).context("decrypted body is not valid json")?;
    Ok((parsed, Some(epk)))
}

fn log_session_completion(
    sessions: &crate::session::SessionManager,
    session_id: uuid::Uuid,
    model: &str,
) {
    if let Some(session) = sessions.get(session_id) {
        let duration_ms = session
            .ended_at
            .map(|end| {
                end.signed_duration_since(session.started_at)
                    .num_milliseconds()
                    .max(0)
            })
            .unwrap_or(0);
        info!(
            session_id = %session_id,
            model = %model,
            tokens_output = session.tokens_output,
            receipts = session.receipts.len(),
            amount_micro_usd = session.amount_micro_usd,
            duration_ms,
            "session completed"
        );
    }
}

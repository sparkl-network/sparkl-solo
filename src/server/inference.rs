use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use async_stream::stream;
use axum::extract::{Extension, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use futures::StreamExt;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::capacity::{max_queue_depth, AcquireError};
use crate::identity;
use crate::receipts::{encode_receipt_for_sse, generate_receipt_with_tee, hash_chunk, provider_identity};
use crate::session::SessionState;

use super::auth::AuthenticatedEvmSession;
use super::AppState;

struct StreamCleanup {
    sessions: Arc<crate::session::SessionManager>,
    session_id: Uuid,
    finished: Arc<AtomicBool>,
}

impl Drop for StreamCleanup {
    fn drop(&mut self) {
        if !self.finished.load(Ordering::Relaxed) {
            self.sessions
                .close(self.session_id, SessionState::ConsumerDisconnected);
        }
    }
}

pub async fn chat_completions(
    State(state): State<AppState>,
    bearer_session: Option<Extension<AuthenticatedEvmSession>>,
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

    let published = match crate::models::build_catalog(
        &state.proxy,
        &state.config.models,
        &state.config.node,
        &state.admission,
    )
    .await
    {
        Ok(models) => models,
        Err(err) => {
            return provider_unavailable_response(&format!("backend model listing failed: {err}"));
        }
    };
    let model_entry = match published.iter().find(|m| m.id == model) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("model not found: {model}"),
                    "type": "model_not_found"
                })),
            )
                .into_response();
        }
    };

    let concurrency = model_entry.concurrency;
    let max_queue = max_queue_depth(concurrency, state.config.capacity.queue_depth_ratio);
    let wait_timeout = Duration::from_secs(state.config.capacity.queue_wait_timeout_secs.max(1));

    let admission_guard = match state
        .admission
        .acquire(&model, concurrency, max_queue, wait_timeout)
        .await
    {
        Ok(guard) => guard,
        Err(err) => return capacity_exhausted_response(err, concurrency),
    };

    let proxy = state.proxy.clone();
    let sessions = state.sessions.clone();
    let receipt_cadence = state.config.node.receipt_cadence_tokens.max(1);
    let config = state.config.clone();
    let _identity_state = state.identity.clone();

    let finished = Arc::new(AtomicBool::new(false));
    let cleanup_finished = Arc::clone(&finished);

    let event_stream = stream! {
        let _admission_guard = admission_guard;
        let tee_quote_hash = if config.node.session_security_tier
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

        let evm_session_id: Option<u64> = if let Some(Extension(auth)) = bearer_session {
            info!(
                evm_session_id = auth.session_id,
                user = %auth.user,
                "using consumer-authenticated on-chain session"
            );
            Some(auth.session_id)
        } else {
            #[cfg(feature = "evm-settlement")]
            {
                if config.settlement.enabled {
                    let escrow_addr = config.settlement.escrow_contract.clone();
                    let rpc_url = config
                        .registry
                        .effective_evm_rpc_url(&config.settlement)
                        .to_string();
                    let pk = config.settlement.evm_provider_wallet_private_key.clone();
                    let min_deposit = config.settlement.session_min_deposit;
                    let tier = config.node.session_security_tier;
                    let node_id = crate::identity::on_chain_node_id_from_identity(&_identity_state);

                    match crate::settlement::evm::open_session_on_chain(
                        &escrow_addr,
                        &rpc_url,
                        &pk,
                        node_id,
                        &model,
                        tier,
                        min_deposit,
                    )
                    .await
                    {
                        Ok(id) => {
                            if id.is_some() {
                                info!(evm_session_id = ?id, "on-chain session opened by provider");
                            }
                            id
                        }
                        Err(err) => {
                            warn!(%err, "open_session_on_chain failed; session will not be linked to escrow");
                            None
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
                config.node.session_security_tier,
                Some(*hash),
                evm_session_id,
            )
        } else {
            sessions.open(
                &model,
                consumer_epk,
                config.node.session_security_tier,
                evm_session_id,
            )
        };

        let _cleanup = StreamCleanup {
            sessions: sessions.clone(),
            session_id,
            finished: cleanup_finished,
        };

        let identity = match provider_identity() {
            Ok(i) => i,
            Err(err) => {
                finished.store(true, Ordering::Relaxed);
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
                finished.store(true, Ordering::Relaxed);
                sessions.close(session_id, SessionState::ProviderError);
                yield Ok::<Event, Infallible>(
                    Event::default().data(json!({
                        "error": format!("backend unavailable: {err}"),
                        "type": "provider_unavailable"
                    }).to_string())
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
                            if let Some(usage_line) =
                                final_stream_usage_line(&sessions, session_id, &model)
                            {
                                yield Ok::<Event, Infallible>(Event::default().data(usage_line));
                            }
                            finished.store(true, Ordering::Relaxed);
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
                        sessions.record_chunk(session_id, 1, content_hash);

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
                    finished.store(true, Ordering::Relaxed);
                    sessions.close(session_id, SessionState::ProviderError);
                    yield Ok::<Event, Infallible>(
                        Event::default().data(json!({"error": err.to_string()}).to_string())
                    );
                    return;
                }
            }
        }

        if let Some(usage_line) = final_stream_usage_line(&sessions, session_id, &model) {
            yield Ok::<Event, Infallible>(Event::default().data(usage_line));
        }
        finished.store(true, Ordering::Relaxed);
        sessions.close(session_id, SessionState::Completed);
        log_session_completion(&sessions, session_id, &model);
        yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
    };

    Sse::new(event_stream).into_response()
}

fn provider_unavailable_response(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": message,
            "type": "provider_unavailable",
        })),
    )
        .into_response()
}

fn capacity_exhausted_response(err: AcquireError, concurrency: u32) -> Response {
    let (active, queued, retry_after) = match err {
        AcquireError::QueueFull {
            active,
            queued, ..
        } => (active, queued, 15),
        AcquireError::WaitTimeout {
            active,
            queued, ..
        } => (active, queued, 30),
    };
    let body = json!({
        "error": "model at capacity",
        "type": "capacity_exhausted",
        "retry_after": retry_after,
        "active_requests": active,
        "concurrency": concurrency,
        "queued_requests": queued,
    });
    let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    if let Ok(val) = HeaderValue::from_str(&retry_after.to_string()) {
        response.headers_mut().insert("retry-after", val);
    }
    response
}

/// Final OpenAI-style `usage` chunk for router on-chain metering (sparkl-router parses this).
fn final_stream_usage_line(
    sessions: &crate::session::SessionManager,
    session_id: uuid::Uuid,
    model: &str,
) -> Option<String> {
    let session = sessions.get(session_id)?;
    if session.tokens_output == 0 {
        return None;
    }
    Some(
        json!({
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": session.tokens_input,
                "completion_tokens": session.tokens_output,
                "total_tokens": session.tokens_input.saturating_add(session.tokens_output),
            }
        })
        .to_string(),
    )
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
    let plaintext = if let Some(v) = request
        .get("encryption_key_version")
        .and_then(|x| x.as_u64())
    {
        identity::decrypt_request_versioned(&ciphertext, &epk, v as u32)?
    } else {
        identity::decrypt_request(&ciphertext, &epk).await?
    };
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

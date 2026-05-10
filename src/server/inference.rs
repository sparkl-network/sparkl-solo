use std::convert::Infallible;

use anyhow::{anyhow, Context};
use async_stream::stream;
use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::Json;
use base64::Engine;
use futures::StreamExt;
use serde_json::{json, Value};
use tracing::warn;

use crate::identity;
use crate::receipts::{encode_receipt_for_sse, generate_receipt, hash_chunk, provider_identity};
use crate::session::SessionState;

use super::AppState;

pub async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<Value>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let parsed_request = decrypt_request_if_needed(request).await;
    let proxy = state.proxy.clone();
    let sessions = state.sessions.clone();

    let event_stream = stream! {
        let (backend_request, consumer_epk) = match parsed_request {
            Ok(v) => v,
            Err(err) => {
                yield Ok(Event::default().data(json!({"error": format!("invalid request: {err}")}).to_string()));
                return;
            }
        };

        let model = backend_request
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown-model")
            .to_string();
        let session_id = sessions.open(&model, consumer_epk);

        let identity = match provider_identity() {
            Ok(i) => i,
            Err(err) => {
                sessions.close(session_id, SessionState::ProviderError);
                yield Ok(Event::default().data(json!({"error": format!("identity unavailable: {err}")}).to_string()));
                return;
            }
        };

        let mut backend = match proxy.stream_completion(backend_request).await {
            Ok(s) => s,
            Err(err) => {
                sessions.close(session_id, SessionState::ProviderError);
                yield Ok(Event::default().data(json!({"error": format!("backend unavailable: {err}")}).to_string()));
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
                            yield Ok(Event::default().data("[DONE]"));
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
                            let receipt = match generate_receipt(&session, &identity, content_hash) {
                                Ok(r) => r,
                                Err(err) => {
                                    warn!(%err, "receipt generation failed");
                                    continue;
                                }
                            };
                            sessions.add_receipt(session_id, receipt.clone());
                            chunk["sparkl"] = json!({
                                "seq": receipt.seq,
                                "receipt": encode_receipt_for_sse(&receipt),
                            });
                            yield Ok(Event::default().data(chunk.to_string()));
                        }
                    }
                }
                Err(err) => {
                    sessions.close(session_id, SessionState::ProviderError);
                    yield Ok(Event::default().data(json!({"error": err.to_string()}).to_string()));
                    return;
                }
            }
        }

        sessions.close(session_id, SessionState::Completed);
        yield Ok(Event::default().data("[DONE]"));
    };

    Sse::new(event_stream)
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

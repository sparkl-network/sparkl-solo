//! Forward router `request` frames to the local inference HTTP server.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::warn;
use uuid::Uuid;

use crate::config::{RegistryConfig, SettlementConfig};
use crate::identity::NodeIdentity;
use crate::router_client::activate::handle_activate_request;
use crate::router_client::frames::{NodeToRouterFrame, RouterToNodeFrame};

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

pub struct ForwardContext {
    pub http: Client,
    pub local_base: String,
    pub ws_tx: Arc<Mutex<WsSink>>,
    pub identity: NodeIdentity,
    pub settlement: SettlementConfig,
    pub registry: RegistryConfig,
}

pub async fn handle_router_frame(ctx: &ForwardContext, text: &str) {
    let frame = match RouterToNodeFrame::parse(text) {
        Ok(f) => f,
        Err(e) => {
            warn!(%e, "invalid router frame JSON");
            return;
        }
    };

    match frame {
        RouterToNodeFrame::Ping => {
            let _ = send_frame(ctx, &NodeToRouterFrame::Pong).await;
        }
        RouterToNodeFrame::Request {
            rid,
            method,
            path,
            headers,
            body,
        } => {
            let ctx = ctx.clone_for_task();
            tokio::spawn(async move {
                if let Err(e) = forward_http_request(&ctx, rid, &method, &path, headers, body).await
                {
                    warn!(%e, %rid, "forward request failed");
                    let _ = send_frame(
                        &ctx,
                        &NodeToRouterFrame::Error {
                            rid,
                            code: 502,
                            message: e.to_string(),
                        },
                    )
                    .await;
                }
            });
        }
        RouterToNodeFrame::ActivateRequest {
            rid,
            session_id,
            signature,
            block_number,
            message,
        } => {
            let ctx = ctx.clone_for_task();
            tokio::spawn(async move {
                let resp = handle_activate_request(
                    rid,
                    &session_id,
                    &signature,
                    block_number,
                    message,
                    &ctx.identity,
                    &ctx.settlement,
                    &ctx.registry,
                )
                .await;
                let _ = send_frame(&ctx, &resp).await;
            });
        }
        RouterToNodeFrame::Challenge { .. } | RouterToNodeFrame::Ready { .. } => {}
    }
}

impl ForwardContext {
    fn clone_for_task(&self) -> Self {
        Self {
            http: self.http.clone(),
            local_base: self.local_base.clone(),
            ws_tx: Arc::clone(&self.ws_tx),
            identity: self.identity.clone(),
            settlement: self.settlement.clone(),
            registry: self.registry.clone(),
        }
    }
}

async fn send_frame(ctx: &ForwardContext, frame: &NodeToRouterFrame) -> Result<()> {
    let json = frame.to_json().context("serialize frame")?;
    let mut guard = ctx.ws_tx.lock().await;
    guard
        .send(Message::Text(json.into()))
        .await
        .context("websocket send")?;
    Ok(())
}

async fn forward_http_request(
    ctx: &ForwardContext,
    rid: Uuid,
    method: &str,
    path: &str,
    headers: Value,
    body: Option<String>,
) -> Result<()> {
    let method: Method = method
        .parse()
        .with_context(|| format!("invalid HTTP method: {method}"))?;
    let url = format!(
        "{}{}",
        ctx.local_base.trim_end_matches('/'),
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    );

    let mut req = ctx.http.request(method.clone(), &url);
    if let Some(hm) = build_header_map(&headers)? {
        req = req.headers(hm);
    }
    if let Some(b) = body {
        req = req.body(b);
    }

    let resp = req
        .send()
        .await
        .context("local inference HTTP request")?;
    let status = resp.status();
    let is_sse = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_sse {
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("read SSE chunk")?;
            let data = String::from_utf8_lossy(&bytes).into_owned();
            send_frame(
                ctx,
                &NodeToRouterFrame::Chunk {
                    rid,
                    data,
                },
            )
            .await?;
        }
        send_frame(
            ctx,
            &NodeToRouterFrame::End {
                rid,
                status: status.as_u16(),
            },
        )
        .await?;
    } else {
        let bytes = resp.bytes().await.context("read response body")?;
        if !bytes.is_empty() {
            send_frame(
                ctx,
                &NodeToRouterFrame::Chunk {
                    rid,
                    data: String::from_utf8_lossy(&bytes).into_owned(),
                },
            )
            .await?;
        }
        send_frame(
            ctx,
            &NodeToRouterFrame::End {
                rid,
                status: status.as_u16(),
            },
        )
        .await?;
    }

    Ok(())
}

fn build_header_map(headers: &Value) -> Result<Option<HeaderMap>> {
    let Some(map) = headers.as_object() else {
        return Ok(None);
    };
    let mut hm = HeaderMap::new();
    for (k, v) in map {
        let Some(s) = v.as_str() else { continue };
        if k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case("content-length") {
            continue;
        }
        let name = HeaderName::try_from(k.as_str())
            .with_context(|| format!("invalid header name: {k}"))?;
        let value =
            HeaderValue::from_str(s).with_context(|| format!("invalid header value for {k}"))?;
        hm.insert(name, value);
    }
    if hm.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hm))
    }
}

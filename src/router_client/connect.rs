//! WSS connect + challenge/auth handshake with sparkl-router.
//! Uses tokio-tungstenite directly (no axum types) to avoid protocol issues.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::info;

use crate::config::{RegistryConfig, SettlementConfig};
use crate::identity::{self, NodeIdentity};
use crate::router_client::challenge::connect_challenge_payload;
use crate::router_client::forward::{ForwardContext, handle_router_frame};
use crate::router_client::frames::{NodeToRouterFrame, RouterToNodeFrame};

pub async fn run_connected_session(
    ws_url: &str,
    identity: &NodeIdentity,
    moniker: Option<&str>,
    http: Client,
    local_base: String,
    settlement: SettlementConfig,
    registry: RegistryConfig,
) -> Result<()> {

    // Use tokio-tungstenite::connect_async directly — no axum WebSocketUpgrade
    let (ws, _resp) = connect_async(ws_url)
        .await
        .with_context(|| format!("websocket connect to {ws_url}"))?;


    // Split the stream so we can send and receive independently.
    // This is needed because ForwardContext expects a SplitSink for ws_tx.
    let (mut sink, mut stream) = ws.split();

    // Wait for router's Challenge frame
    let challenge_text = recv_text(&mut stream).await.context("waiting for challenge")?;

    let challenge: RouterToNodeFrame =
        serde_json::from_str(&challenge_text).context("parse challenge frame")?;

    let (nonce_hex, block) = match &challenge {
        RouterToNodeFrame::Challenge { nonce, block } => (nonce.clone(), *block),
        other => anyhow::bail!("expected challenge from router, got: {:?}", other),
    };

    // Decode nonce and build payload
    let mut nonce_bytes = [0u8; 32];
    let decoded = hex::decode(nonce_hex.trim()).context("decode challenge nonce")?;
    let copy_len = decoded.len().min(32);
    nonce_bytes[..copy_len].copy_from_slice(&decoded[..copy_len]);

    let payload = connect_challenge_payload(&nonce_bytes, block);

    // Sign the challenge
    let sig = identity::sign_bytes(&payload).context("sign connect challenge")?;
    let node_id = identity::on_chain_node_id_hex_from_peer_id(&identity.peer_id)
        .context("get on-chain node ID")?;
    let ed_pk = hex::encode(identity.ed25519_pubkey);

    let auth = NodeToRouterFrame::Auth {
        node_id,
        signature: hex::encode(&sig),
        ed25519_pubkey: Some(ed_pk),
        moniker: moniker.map(str::to_string),
    };

    sink.send(Message::Text(auth.to_json().context("serialize auth")?))
        .await
        .with_context(|| "send auth frame to router")?;

    // Wait for Ready frame
    let ready_text = recv_text(&mut stream).await.context("waiting for ready")?;

    let ready: RouterToNodeFrame =
        serde_json::from_str(&ready_text).context("parse ready frame")?;

    match &ready {
        RouterToNodeFrame::Ready { router_url } => {
            info!(%router_url, peer_id = %identity.peer_id, "router tunnel ready");
        }
        other => anyhow::bail!("expected ready from router, got: {:?}", other),
    }

    // Create forward context for HTTP->WS bridging
    let forward_ctx = ForwardContext {
        http,
        local_base,
        ws_tx: Arc::new(Mutex::new(sink)), // Send half
        identity: identity.clone(),
        settlement,
        registry,
    };


    // Main event loop — read frames from router
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                handle_router_frame(&forward_ctx, &t).await;
            }
            Ok(Message::Ping(p)) => {
                let mut guard = forward_ctx.ws_tx.lock().await;
                let _ = guard.send(Message::Pong(p)).await;
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {} // Pong etc. — ignore
        }
    }

    Ok(())
}

/// Wait for a Text message from the WebSocket stream.
async fn recv_text(
    stream: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
) -> Result<String> {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => return Ok(t.to_string()),
            Ok(Message::Close(c)) => {
                anyhow::bail!("websocket closed before text frame: {:?}", c);
            }
            Err(e) => {
                return Err(e.into());
            }
            _ => continue, // Skip Ping/Pong/etc.
        }
    }
    anyhow::bail!("websocket stream ended without text frame");
}

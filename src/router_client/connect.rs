//! WSS connect + challenge/auth handshake with sparkl-router.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::info;

use crate::config::{RegistryConfig, SettlementConfig};
use crate::identity::{self, NodeIdentity};
use crate::router_client::challenge::connect_challenge_payload;
use crate::router_client::frames::{NodeToRouterFrame, RouterToNodeFrame};
use crate::router_client::forward::{ForwardContext, handle_router_frame};
use reqwest::Client;

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

pub async fn run_connected_session(
    ws_url: &str,
    identity: &NodeIdentity,
    http: Client,
    local_base: String,
    settlement: SettlementConfig,
    registry: RegistryConfig,
) -> Result<()> {
    let (ws, _) = connect_async(ws_url)
        .await
        .with_context(|| format!("websocket connect to {ws_url}"))?;

    let (sink, mut stream) = ws.split();
    let forward_ctx = ForwardContext {
        http,
        local_base,
        ws_tx: Arc::new(Mutex::new(sink)),
        identity: identity.clone(),
        settlement,
        registry,
    };

    let challenge_text = recv_text(&mut stream).await?;
    let challenge: RouterToNodeFrame =
        serde_json::from_str(&challenge_text).context("parse challenge frame")?;
    let (nonce_hex, block) = match challenge {
        RouterToNodeFrame::Challenge { nonce, block } => (nonce, block),
        _ => anyhow::bail!("expected challenge frame from router"),
    };

    let mut nonce = [0u8; 32];
    let decoded = hex::decode(nonce_hex.trim()).context("decode challenge nonce")?;
    let copy_len = decoded.len().min(32);
    nonce[..copy_len].copy_from_slice(&decoded[..copy_len]);

    let payload = connect_challenge_payload(&nonce, block);
    let sig = identity::sign_bytes(&payload).context("sign connect challenge")?;
    let node_id = identity::on_chain_node_id_hex_from_peer_id(&identity.peer_id)?;
    let ed_pk = hex::encode(identity.ed25519_pubkey);

    let auth = NodeToRouterFrame::Auth {
        node_id,
        signature: hex::encode(sig),
        ed25519_pubkey: Some(ed_pk),
    };
    {
        let mut guard = forward_ctx.ws_tx.lock().await;
        guard
            .send(Message::Text(auth.to_json()?.into()))
            .await
            .context("send auth frame")?;
    }

    let ready_text = recv_text(&mut stream).await?;
    let ready: RouterToNodeFrame =
        serde_json::from_str(&ready_text).context("parse ready frame")?;
    match ready {
        RouterToNodeFrame::Ready { router_url } => {
            info!(%router_url, peer_id = %identity.peer_id, "router tunnel ready");
        }
        _ => anyhow::bail!("expected ready frame from router"),
    }

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
            _ => {}
        }
    }

    Ok(())
}

async fn recv_text(stream: &mut futures_util::stream::SplitStream<WsStream>) -> Result<String> {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => return Ok(t.to_string()),
            Ok(Message::Close(_)) => anyhow::bail!("websocket closed before text frame"),
            Err(e) => return Err(e.into()),
            _ => continue,
        }
    }
    anyhow::bail!("websocket closed")
}

use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::identity::{current_identity, sign_bytes, NodeIdentity};
use crate::session::Session;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkReceipt {
    pub session_id: Uuid,
    pub provider_id: String,
    pub seq: u64,
    pub token_count: u64,
    pub content_hash: [u8; 32],
    pub timestamp_ms: u64,
    pub provider_sig: Vec<u8>,
}

pub fn generate_receipt(
    session: &Session,
    identity: &NodeIdentity,
    chunk_window_hash: [u8; 32],
) -> Result<ChunkReceipt> {
    let mut receipt = ChunkReceipt {
        session_id: session.id,
        provider_id: identity.peer_id.clone(),
        seq: session.last_receipt_seq + 1,
        token_count: session.tokens_output,
        content_hash: chunk_window_hash,
        timestamp_ms: Utc::now().timestamp_millis() as u64,
        provider_sig: Vec::new(),
    };
    let canonical = canonical_payload(&receipt)?;
    receipt.provider_sig = sign_bytes(&canonical)?.to_vec();
    Ok(receipt)
}

pub fn encode_receipt_for_sse(receipt: &ChunkReceipt) -> String {
    let json = serde_json::to_vec(receipt).unwrap_or_default();
    base64::engine::general_purpose::STANDARD.encode(json)
}

pub fn verify_consumer_receipt(
    receipt: &ChunkReceipt,
    consumer_pubkey: &[u8; 32],
    consumer_sig: &[u8; 64],
) -> bool {
    let vk = match VerifyingKey::from_bytes(consumer_pubkey) {
        Ok(vk) => vk,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(consumer_sig);
    let payload = match canonical_payload(receipt) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    vk.verify(&payload, &sig).is_ok()
}

pub fn verify_provider_receipt(receipt: &ChunkReceipt, provider_pubkey: &[u8; 32]) -> bool {
    let vk = match VerifyingKey::from_bytes(provider_pubkey) {
        Ok(vk) => vk,
        Err(_) => return false,
    };
    let sig_bytes: [u8; 64] = match receipt.provider_sig.as_slice().try_into() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(&sig_bytes);
    let payload = match canonical_payload(receipt) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    vk.verify(&payload, &sig).is_ok()
}

pub fn hash_chunk(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub fn unicity_request_id(receipt: &ChunkReceipt) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(receipt.session_id.as_bytes());
    hasher.update(receipt.seq.to_le_bytes());
    hasher.update(receipt.provider_id.as_bytes());
    hasher.finalize().into()
}

#[cfg(feature = "unicity")]
const UNICITY_GATEWAY_TESTNETS: [&str; 2] = [
    "https://gateway-test.unicity.network/",
    "https://goggregator-test.unicity.network/",
];

#[cfg(feature = "unicity")]
pub async fn submit_commitment(receipt: &ChunkReceipt) -> Result<String> {
    let request_id = hex::encode(unicity_request_id(receipt));
    let payload =
        hex::encode(serde_json::to_vec(receipt).context("failed to serialize receipt payload")?);
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "submit_commitment",
        "params": {
            "requestId": request_id,
            "payload": payload,
        },
        "id": 1
    });

    post_to_unicity_gateways(&payload, "submit_commitment").await?;

    // Proof materialization may lag slightly behind acceptance; retry briefly.
    let mut last_err = None;
    for _ in 0..3 {
        match get_inclusion_proof_hex(&request_id).await {
            Ok(proof_hex) => return Ok(proof_hex),
            Err(err) => last_err = Some(err),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("failed to fetch inclusion proof")))
}

#[cfg(feature = "unicity")]
async fn get_inclusion_proof_hex(request_id: &str) -> Result<String> {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "get_inclusion_proof",
        "params": {
            "requestId": request_id,
            "stateId": request_id
        },
        "id": 2
    });
    let body = post_to_unicity_gateways(&payload, "get_inclusion_proof").await?;
    let value: serde_json::Value =
        serde_json::from_str(&body).context("invalid JSON response from get_inclusion_proof")?;
    if let Some(error) = value.get("error") {
        anyhow::bail!("Unicity get_inclusion_proof RPC error: {error}");
    }
    let result = value
        .get("result")
        .cloned()
        .context("Unicity get_inclusion_proof missing result field")?;
    let result_bytes = serde_json::to_vec(&result).context("failed to encode inclusion proof")?;
    Ok(hex::encode(result_bytes))
}

#[cfg(feature = "unicity")]
async fn post_to_unicity_gateways(payload: &serde_json::Value, method: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let mut last_error = String::new();

    for gateway in UNICITY_GATEWAY_TESTNETS {
        let resp = client
            .post(gateway)
            .header("content-type", "application/json")
            .json(payload)
            .send()
            .await;

        match resp {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    return Ok(body);
                }
                last_error = format!("gateway {gateway} returned HTTP {status}: {body}");
            }
            Err(err) => {
                last_error = format!("gateway {gateway} request failed: {err}");
            }
        }
    }

    anyhow::bail!("Unicity {method} failed on all gateways: {last_error}")
}

pub fn provider_identity() -> Result<NodeIdentity> {
    current_identity().context("provider identity unavailable")
}

fn canonical_payload(receipt: &ChunkReceipt) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Payload<'a> {
        session_id: Uuid,
        provider_id: &'a str,
        seq: u64,
        token_count: u64,
        content_hash: [u8; 32],
        timestamp_ms: u64,
    }

    let payload = Payload {
        session_id: receipt.session_id,
        provider_id: &receipt.provider_id,
        seq: receipt.seq,
        token_count: receipt.token_count,
        content_hash: receipt.content_hash,
        timestamp_ms: receipt.timestamp_ms,
    };
    Ok(serde_json::to_vec(&payload)?)
}

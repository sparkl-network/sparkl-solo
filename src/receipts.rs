use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(feature = "unicity")]
use tracing::info;
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

#[cfg(feature = "unicity")]
#[derive(Debug, Clone)]
struct RequestId(String);

#[cfg(feature = "unicity")]
impl RequestId {
    fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "unicity")]
#[derive(Debug, Clone)]
struct StateId(String);

#[cfg(feature = "unicity")]
impl StateId {
    fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "unicity")]
#[derive(Debug)]
enum InclusionProofQueryError {
    InvalidParams(anyhow::Error),
    Other(anyhow::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnicityProof {
    pub request_id: String,
    pub state_id: String,
    pub proof_hex: String,
    pub anchored_at_ms: u64,
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
pub async fn submit_commitment(receipt: &ChunkReceipt, gateway_url: &str) -> Result<UnicityProof> {
    let request_id = RequestId(hex::encode(unicity_request_id(receipt)));
    let state_id = derive_state_id(receipt, &request_id);
    let payload_hex = hex::encode(
        canonical_payload(receipt).context("failed to serialize canonical receipt payload")?,
    );
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "submit_commitment",
        "params": {
            "requestId": request_id.as_str(),
            "payload": payload_hex,
        },
        "id": 1
    });

    post_to_unicity_gateway(gateway_url, &body, "submit_commitment").await?;

    // Proof materialization may lag slightly behind acceptance; retry briefly.
    let mut last_err = None;
    for _ in 0..3 {
        match get_inclusion_proof_hex(gateway_url, &request_id, &state_id).await {
            Ok((proof_hex, query_shape)) => {
                info!(
                    request_id = request_id.as_str(),
                    state_id = state_id.as_str(),
                    query_shape,
                    "unicity inclusion proof query succeeded"
                );
                return Ok(UnicityProof {
                    request_id: request_id.0.clone(),
                    state_id: state_id.0.clone(),
                    proof_hex,
                    anchored_at_ms: Utc::now().timestamp_millis() as u64,
                });
            }
            Err(err) => last_err = Some(err),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("failed to fetch inclusion proof")))
}

#[cfg(feature = "unicity")]
async fn get_inclusion_proof_hex(
    gateway_url: &str,
    request_id: &RequestId,
    state_id: &StateId,
) -> Result<(String, &'static str)> {
    // Official aggregator docs show `stateId` for get_inclusion_proof.
    let state_id_params = serde_json::json!({
        "stateId": state_id.as_str()
    });
    match get_inclusion_proof_with_params(gateway_url, state_id_params).await {
        Ok(proof_hex) => return Ok((proof_hex, "stateId")),
        Err(InclusionProofQueryError::InvalidParams(_)) => {
            let request_id_params = serde_json::json!({
                "requestId": request_id.as_str()
            });
            let proof_hex =
                match get_inclusion_proof_with_params(gateway_url, request_id_params).await {
                    Ok(proof_hex) => proof_hex,
                    Err(InclusionProofQueryError::InvalidParams(err))
                    | Err(InclusionProofQueryError::Other(err)) => return Err(err),
                };
            return Ok((proof_hex, "requestId"));
        }
        Err(InclusionProofQueryError::Other(err)) => return Err(err),
    }
}

#[cfg(feature = "unicity")]
async fn get_inclusion_proof_with_params(
    gateway_url: &str,
    params: serde_json::Value,
) -> std::result::Result<String, InclusionProofQueryError> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "get_inclusion_proof",
        "params": params,
        "id": 2
    });
    let body = post_to_unicity_gateway(gateway_url, &body, "get_inclusion_proof")
        .await
        .map_err(InclusionProofQueryError::Other)?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|err| {
        InclusionProofQueryError::Other(anyhow::anyhow!(
            "invalid JSON response from get_inclusion_proof: {err}"
        ))
    })?;
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(serde_json::Value::as_i64);
        let anyhow_err =
            anyhow::anyhow!("Unicity get_inclusion_proof RPC error code={code:?}: {error}");
        if code == Some(-32602) {
            return Err(InclusionProofQueryError::InvalidParams(anyhow_err));
        }
        return Err(InclusionProofQueryError::Other(anyhow_err));
    }
    let result = value.get("result").cloned().ok_or_else(|| {
        InclusionProofQueryError::Other(anyhow::anyhow!(
            "Unicity get_inclusion_proof missing result field"
        ))
    })?;
    let result_bytes = serde_json::to_vec(&result).map_err(|err| {
        InclusionProofQueryError::Other(anyhow::anyhow!("failed to encode inclusion proof: {err}"))
    })?;
    Ok(hex::encode(result_bytes))
}

#[cfg(feature = "unicity")]
fn derive_state_id(_receipt: &ChunkReceipt, request_id: &RequestId) -> StateId {
    // For now we mirror request_id until live schema confirms a distinct derivation algorithm.
    StateId(request_id.0.clone())
}

#[cfg(feature = "unicity")]
async fn post_to_unicity_gateway(
    url: &str,
    payload: &serde_json::Value,
    method: &str,
) -> Result<String> {
    let resp = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .json(payload)
        .send()
        .await
        .with_context(|| format!("Unicity {method} request to {url} failed"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    anyhow::ensure!(
        status.is_success(),
        "Unicity {method} at {url} returned HTTP {status}: {}",
        &body[..body.len().min(200)]
    );
    Ok(body)
}

pub fn provider_identity() -> Result<NodeIdentity> {
    current_identity().context("provider identity unavailable")
}

pub(crate) fn canonical_payload(receipt: &ChunkReceipt) -> Result<Vec<u8>> {
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

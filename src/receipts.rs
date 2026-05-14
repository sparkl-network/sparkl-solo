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

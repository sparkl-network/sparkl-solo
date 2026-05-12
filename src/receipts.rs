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
pub async fn submit_commitment(
    receipt: &ChunkReceipt,
    gateway_url: &str,
    api_key: Option<&str>,
) -> Result<UnicityProof> {
    let gateway_url = gateway_url.trim();
    anyhow::ensure!(
        !gateway_url.is_empty(),
        "registry.unicity_aggregator_url is empty; set it to your Unicity JSON-RPC gateway base URL"
    );

    let request_id = RequestId(hex::encode(unicity_request_id(receipt)));
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

    let submit_raw =
        post_to_unicity_gateway(gateway_url, &body, "submit_commitment", api_key).await?;
    let submit_value: serde_json::Value =
        serde_json::from_str(&submit_raw).context("invalid JSON from submit_commitment")?;

    let state_id = match submit_value.get("error") {
        None => derive_state_id(receipt, &request_id),
        Some(err) => {
            let code = err.get("code").and_then(serde_json::Value::as_i64);
            if code != Some(-32601) {
                anyhow::bail!("Unicity submit_commitment RPC error code={code:?}: {err}");
            }

            let ed = crate::identity::ed25519_secret_bytes().context("identity for anchor")?;
            let built = crate::unicity_cert::build_certification_request(receipt, &ed)
                .context("encode certification_request")?;
            let cert_body = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "certification_request",
                "params": built.request_hex,
                "id": 3_i64,
            });

            let cert_raw = post_to_unicity_gateway(
                gateway_url,
                &cert_body,
                "certification_request",
                api_key,
            )
            .await?;
            let cert_val: serde_json::Value =
                serde_json::from_str(&cert_raw).context("invalid JSON from certification_request")?;

            if let Some(cerr) = cert_val.get("error") {
                anyhow::bail!("Unicity certification_request RPC error: {cerr}");
            }

            let status = cert_val
                .get("result")
                .and_then(|r| r.get("status"))
                .and_then(serde_json::Value::as_str);
            match status {
                Some("SUCCESS") | Some("STATE_ID_EXISTS") => (),
                other => anyhow::bail!(
                    "Unicity certification_request unexpected result {:?}: {}",
                    other,
                    &cert_raw[..cert_raw.len().min(200)]
                ),
            }

            StateId(hex::encode(built.state_id_raw))
        }
    };
    let mut last_err = None;
    for _ in 0..3 {
        match get_inclusion_proof_hex(gateway_url, &request_id, &state_id, api_key).await {
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
    api_key: Option<&str>,
) -> Result<(String, &'static str)> {
    // Official aggregator docs show `stateId` for get_inclusion_proof.
    let state_id_params = serde_json::json!({
        "stateId": state_id.as_str()
    });
    match get_inclusion_proof_with_params(gateway_url, state_id_params, api_key).await {
        Ok(proof_hex) => return Ok((proof_hex, "stateId")),
        Err(InclusionProofQueryError::InvalidParams(_)) => {
            let request_id_params = serde_json::json!({
                "requestId": request_id.as_str()
            });
            let proof_hex =
                match get_inclusion_proof_with_params(gateway_url, request_id_params, api_key).await {
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
    api_key: Option<&str>,
) -> std::result::Result<String, InclusionProofQueryError> {
    const METHODS: [&str; 2] = ["get_inclusion_proof", "get_inclusion_proof.v2"];

    let mut last_method_nf: Option<anyhow::Error> = None;
    for (i, rpc_method) in METHODS.iter().enumerate() {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": rpc_method,
            "params": params,
            "id": 2
        });
        let raw = match post_to_unicity_gateway(gateway_url, &body, rpc_method, api_key).await {
            Ok(s) => s,
            Err(err) => return Err(InclusionProofQueryError::Other(err)),
        };
        let value: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
            InclusionProofQueryError::Other(anyhow::anyhow!(
                "invalid JSON response from {rpc_method}: {err}"
            ))
        })?;
        if let Some(error) = value.get("error") {
            let code = error.get("code").and_then(serde_json::Value::as_i64);
            if code == Some(-32601) && i + 1 < METHODS.len() {
                last_method_nf = Some(anyhow::anyhow!(
                    "Unicity {rpc_method} RPC error code={code:?}: {error}"
                ));
                continue;
            }
            let anyhow_err =
                anyhow::anyhow!("Unicity {rpc_method} RPC error code={code:?}: {error}");
            if code == Some(-32602) {
                return Err(InclusionProofQueryError::InvalidParams(anyhow_err));
            }
            return Err(InclusionProofQueryError::Other(anyhow_err));
        }

        let result = value.get("result").cloned().ok_or_else(|| {
            InclusionProofQueryError::Other(anyhow::anyhow!(
                "Unicity {rpc_method} missing result field"
            ))
        })?;
        let result_bytes = serde_json::to_vec(&result).map_err(|err| {
            InclusionProofQueryError::Other(anyhow::anyhow!("failed to encode inclusion proof: {err}"))
        })?;
        return Ok(hex::encode(result_bytes));
    }

    Err(InclusionProofQueryError::Other(
        last_method_nf.unwrap_or_else(|| {
            anyhow::anyhow!("Unicity inclusion proof query failed without a response")
        }),
    ))
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
    api_key: Option<&str>,
) -> Result<String> {
    let client = reqwest::Client::new();
    let mut req = client
        .post(url)
        .header("content-type", "application/json")
        .json(payload);

    if let Some(key) = api_key {
        let key = key.trim();
        if !key.is_empty() {
            req = req.header("X-API-Key", key);
        }
    }

    let resp = req
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

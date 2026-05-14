use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::identity;

use super::AppState;

/// `GET /identity`
///
/// Returns the node's public identity, its advertised p2p addresses, and a
/// self-signed proof of possession anchored to the current chain head block.
///
/// **Listen addresses are omitted:** internal bind multiaddrs (`network.listen_addrs`)
/// are not included in this JSON — only operator-configured `public_addrs` appear,
/// so the endpoint does not leak local or container topology.
///
/// A chain-anchored `proof.anchor` / `proof.signature` is produced only when the
/// binary is built with the **`evm-settlement`** feature (so Alloy is linked),
/// and settlement RPC is configured. Default/CI builds omit the proof even if
/// `settlement.evm_rpc_url` is set.
///
/// The signed payload is:
///   SHA256("sparkl-identity-v1" || ed25519_pubkey[32] || chain_id[8 BE] || block_hash[32])
///
/// Callers verify with Ed25519(signature, payload, ed25519_pubkey).
/// `block_hash` acts as a freshness nonce — proofs older than ~10 blocks should be rejected.
/// No operator address is bound here (this is a public identity proof, not a registration proof).
pub async fn identity(State(state): State<AppState>) -> Response {
    // -- 1. Resolve node public key
    let node_identity = match identity::current_identity() {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("identity unavailable: {e}") })),
            )
                .into_response();
        }
    };
    let ed25519_pubkey_bytes = node_identity.ed25519_pubkey;

    // -- 2. Derive on-chain nodeId (single canonical rule: see identity::on_chain_node_id_bytes)
    let node_id_hex = identity::on_chain_node_id_hex(&ed25519_pubkey_bytes);

    // -- 3. Fetch chain head block hash (if EVM configured)
    let chain_proof = if state.config.settlement.enabled
        && !state.config.settlement.evm_rpc_url.trim().is_empty()
        && !state.config
            .settlement
            .evm_rpc_url
            .contains("YOUR_POLKADOT")
    {
        fetch_chain_head(&state.config.settlement.evm_rpc_url).await
    } else {
        None
    };

    // -- 4. Build and sign the payload
    let (anchor, signature_hex) = match &chain_proof {
        Some(head) => {
            let block_hash_bytes = match hex::decode(head.block_hash.trim_start_matches("0x")) {
                Ok(b) if b.len() == 32 => b,
                _ => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "unexpected block hash length from chain" })),
                    )
                        .into_response();
                }
            };

            let mut hasher = Sha256::new();
            hasher.update(b"sparkl-identity-v1");
            hasher.update(ed25519_pubkey_bytes);
            hasher.update(head.chain_id.to_be_bytes());
            hasher.update(&block_hash_bytes);
            let payload: [u8; 32] = hasher.finalize().into();

            match identity::sign_challenge(&payload).await {
                Ok(sig) => (
                    Some(json!({
                        "chain_id": head.chain_id,
                        "block_number": head.block_number,
                        "block_hash": head.block_hash,
                    })),
                    Some(hex::encode(sig)),
                ),
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("signing failed: {e}") })),
                    )
                        .into_response();
                }
            }
        }
        // No EVM configured — return identity without a chain-anchored proof.
        None => (None, None),
    };

    // -- 5. Collect advertised public addresses (from config, already validated at startup)
    let public_addrs: Vec<&str> = state
        .config
        .network
        .public_addr
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut body = json!({
        "peer_id": node_identity.peer_id,
        "node_id": node_id_hex,
        "ed25519_pubkey": hex::encode(ed25519_pubkey_bytes),
        "x25519_pubkey": hex::encode(node_identity.x25519_pubkey),
        "version": env!("CARGO_PKG_VERSION"),
        "public_addrs": public_addrs,
        "proof": {
            "algorithm": "ed25519",
            "domain": "sparkl-identity-v1",
            "anchor": anchor,
            "signature": signature_hex,
        }
    });

    if let Ok(cert_type) = identity::attestation_cert_type() {
        body["key_source"] = json!(cert_type);
    }

    Json(body).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ChainHead {
    chain_id: u64,
    block_number: u64,
    block_hash: String, // "0x..." 32-byte hex
}

/// Best-effort head fetch — `None` if the RPC URL is invalid, the node is
/// unreachable, or the binary was built without `evm-settlement` (no Alloy).
#[cfg(feature = "evm-settlement")]
async fn fetch_chain_head(rpc_url: &str) -> Option<ChainHead> {
    use alloy::eips::BlockId;
    use alloy::providers::{Provider, ProviderBuilder};

    let url = rpc_url.trim().parse::<reqwest::Url>().ok()?;
    let provider = ProviderBuilder::new().connect_http(url);

    let block = provider.get_block(BlockId::latest()).await.ok()??;
    let chain_id = provider.get_chain_id().await.ok()?;

    let block_hash = block.header.hash;
    let block_number: u64 = block.header.number;

    Some(ChainHead {
        chain_id,
        block_number,
        block_hash: format!("0x{}", hex::encode(block_hash.as_slice())),
    })
}

#[cfg(not(feature = "evm-settlement"))]
async fn fetch_chain_head(_rpc_url: &str) -> Option<ChainHead> {
    None
}

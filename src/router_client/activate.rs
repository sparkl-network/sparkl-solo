//! Session activate over router tunnel: verify wallet proof and mint `sk_` bearer.

use alloy_primitives::{keccak256, Address};
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::{RegistryConfig, SettlementConfig};
use crate::identity::{self, NodeIdentity};
use crate::router_client::frames::NodeToRouterFrame;
use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use secp256k1::{Message, Secp256k1};

/// Handle `activate_request` from the router; returns wire frame to send.
pub async fn handle_activate_request(
    rid: Uuid,
    session_id_hex: &str,
    signature_hex: &str,
    block_number: u64,
    message: Option<String>,
    identity: &NodeIdentity,
    settlement: &SettlementConfig,
    registry: &RegistryConfig,
) -> NodeToRouterFrame {
    match handle_activate_inner(
        session_id_hex,
        signature_hex,
        block_number,
        message,
        identity,
        settlement,
        registry,
    )
    .await
    {
        Ok(api_key) => NodeToRouterFrame::ActivateResponse { rid, api_key },
        Err(e) => NodeToRouterFrame::Error {
            rid,
            code: 401,
            message: e.to_string(),
        },
    }
}

async fn handle_activate_inner(
    session_id_hex: &str,
    signature_hex: &str,
    block_number: u64,
    message: Option<String>,
    identity: &NodeIdentity,
    settlement: &SettlementConfig,
    registry: &RegistryConfig,
) -> Result<String> {
    let session_id = parse_session_id_u64(session_id_hex)?;
    let canonical = message
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("sparkl-activate:{session_id}:{block_number}"));

    let user = recover_activate_signer(canonical.as_bytes(), signature_hex)
        .map_err(|_| anyhow!("invalid activate signature"))?;

    #[cfg(feature = "evm-settlement")]
    if settlement.enabled {
        let escrow = settlement.escrow_contract.trim();
        let rpc = registry.effective_evm_rpc_url(settlement);
        let chain_sess =
            crate::settlement::evm::fetch_chain_session(escrow, rpc, session_id).await?;

        if chain_sess.user == Address::ZERO {
            return Err(anyhow!("unknown on-chain session"));
        }
        if chain_sess.settled {
            return Err(anyhow!("session already settled"));
        }
        if chain_sess.user != user {
            return Err(anyhow!("signature user does not match session owner"));
        }
        let expected_node =
            alloy_primitives::FixedBytes::from(identity::on_chain_node_id_from_identity(identity));
        if chain_sess.node_id != expected_node {
            return Err(anyhow!("session is not for this node"));
        }
        return mint_sk_bearer(session_id, user);
    }

    #[cfg(not(feature = "evm-settlement"))]
    let _ = (settlement, registry, identity);

    mint_sk_bearer(session_id, user)
}

fn parse_session_id_u64(s: &str) -> Result<u64> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x") {
        let hex = hex.trim_start_matches('0');
        if hex.is_empty() {
            return Ok(0);
        }
        if hex.len() <= 16 {
            return u64::from_str_radix(hex, 16).context("invalid session id hex");
        }
        let padded = format!("{:0>64}", hex);
        let bytes = hex::decode(&padded[..64.min(padded.len())]).context("invalid session id")?;
        if bytes.len() != 32 || bytes[..24] != [0u8; 24] {
            return Err(anyhow!("session id exceeds u64 range"));
        }
        return Ok(u64::from_be_bytes(bytes[24..32].try_into().unwrap()));
    }
    t.parse::<u64>().context("invalid session id")
}

fn recover_activate_signer(message: &[u8], sig_hex: &str) -> Result<Address> {
    let sig_bytes = hex::decode(sig_hex.strip_prefix("0x").unwrap_or(sig_hex))
        .context("invalid signature hex")?;
    if sig_bytes.len() != 65 {
        return Err(anyhow!("signature must be 65 bytes"));
    }
    let digest = eip191_message_digest(message);
    let msg = Message::from_digest(digest);
    let v = sig_bytes[64];
    let rec_byte = if v >= 27 { v - 27 } else { v };
    let rec_id = RecoveryId::from_u8_masked(rec_byte);
    let sig = RecoverableSignature::from_compact(&sig_bytes[..64], rec_id)
        .map_err(|_| anyhow!("invalid compact signature"))?;
    let pubkey = Secp256k1::verification_only()
        .recover_ecdsa(msg, &sig)
        .map_err(|_| anyhow!("ecdsa recover failed"))?;
    let uncompressed = pubkey.serialize_uncompressed();
    let hash = keccak256(&uncompressed[1..]);
    Ok(Address::from_slice(&hash[12..]))
}

fn eip191_message_digest(message: &[u8]) -> [u8; 32] {
    let len = message.len().to_string();
    let mut prefixed = Vec::with_capacity(26 + len.len() + message.len());
    prefixed.extend_from_slice(b"\x19Ethereum Signed Message:\n");
    prefixed.extend_from_slice(len.as_bytes());
    prefixed.extend_from_slice(message);
    keccak256(&prefixed).0
}

fn mint_sk_bearer(session_id: u64, user: Address) -> Result<String> {
    let ed_secret = identity::ed25519_secret_bytes().context("identity secret")?;
    let secret = derive_bearer_secret(&ed_secret, session_id, user);

    let mut session_bytes = [0u8; 32];
    session_bytes[24..32].copy_from_slice(&session_id.to_be_bytes());

    let mut payload = [0u8; 64];
    payload[..32].copy_from_slice(&session_bytes);
    payload[32..].copy_from_slice(&secret);

    Ok(format!("sk_{}", bs58::encode(payload).into_string()))
}

fn derive_bearer_secret(ed25519_secret: &[u8; 32], session_id: u64, user: Address) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"sparkl-sk-v1");
    h.update(ed25519_secret);
    h.update(session_id.to_be_bytes());
    h.update(user.as_slice());
    h.finalize().into()
}

/// Verify `Authorization: Bearer sk_...` when settlement auth is enabled.
pub fn verify_sk_bearer(token: &str, session_id: u64, user: Address) -> Result<(), &'static str> {
    let raw = token
        .strip_prefix("sk_")
        .ok_or("bearer must start with sk_")?;
    let bytes = bs58::decode(raw)
        .into_vec()
        .map_err(|_| "invalid base58 in sk_ token")?;
    if bytes.len() != 64 {
        return Err("sk_ token must decode to 64 bytes");
    }
    let mut sess = [0u8; 32];
    sess.copy_from_slice(&bytes[..32]);
    if sess[..24] != [0u8; 24] {
        return Err("session id in token exceeds u64 range");
    }
    let parsed_id = u64::from_be_bytes(sess[24..32].try_into().unwrap());
    if parsed_id != session_id {
        return Err("session id mismatch in sk_ token");
    }
    let ed_secret = identity::ed25519_secret_bytes().map_err(|_| "identity not loaded")?;
    let expected = derive_bearer_secret(&ed_secret, session_id, user);
    if bytes[32..] != expected {
        return Err("invalid sk_ secret");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_secret_deterministic() {
        let ed = [7u8; 32];
        let user = Address::from([8u8; 20]);
        assert_eq!(
            derive_bearer_secret(&ed, 42, user),
            derive_bearer_secret(&ed, 42, user),
        );
    }
}

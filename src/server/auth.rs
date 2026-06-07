//! Bearer session auth for OpenAI-compatible inference routes when settlement is enabled.

use alloy_primitives::Address;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
#[cfg(feature = "evm-settlement")]
use alloy_primitives::keccak256;
#[cfg(feature = "evm-settlement")]
use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
#[cfg(feature = "evm-settlement")]
use secp256k1::{Message, Secp256k1};
#[cfg(feature = "evm-settlement")]
use tracing::warn;

#[cfg(feature = "evm-settlement")]
use crate::identity;

use super::AppState;

/// Authenticated on-chain escrow session injected by [`require_session_bearer`].
#[derive(Clone, Debug)]
pub struct AuthenticatedEvmSession {
    pub session_id: u64,
    pub user: Address,
}

/// Axum middleware: `Authorization: Bearer <sessionId>` + optional EIP-191 proof headers.
///
/// Gated by `config.settlement.enabled` — when false, passes through unchanged (local dev).
pub async fn require_session_bearer(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    if !state.config.settlement.enabled {
        return next.run(request).await;
    }

    match authenticate(&state, request.headers()).await {
        Ok(auth) => {
            request.extensions_mut().insert(auth);
            next.run(request).await
        }
        Err(resp) => resp,
    }
}

async fn authenticate(
    #[cfg_attr(not(feature = "evm-settlement"), allow(unused_variables))]
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedEvmSession, Response> {
    let (session_id, _sk_token) = parse_bearer_session(headers).map_err(auth_error)?;

    #[cfg(feature = "evm-settlement")]
    {
        let escrow = state.config.settlement.escrow_contract.trim();
        let rpc = state
            .config
            .registry
            .effective_evm_rpc_url(&state.config.settlement);

        let chain_sess = crate::settlement::evm::fetch_chain_session(escrow, rpc, session_id)
            .await
            .map_err(|e| {
                warn!(%e, session_id, "escrow sessions() read failed");
                auth_error("failed to verify on-chain session")
            })?;

        if chain_sess.user == Address::ZERO {
            return Err(auth_error("unknown on-chain session"));
        }
        if chain_sess.settled {
            return Err(auth_error("session already settled"));
        }

        let expected_node_id =
            alloy_primitives::FixedBytes::from(identity::on_chain_node_id_from_identity(
                &state.identity,
            ));
        if chain_sess.node_id != expected_node_id {
            return Err(auth_error("session is not for this node"));
        }

        if let Some(token) = sk_token {
            crate::router_client::verify_sk_bearer(
                token,
                session_id,
                chain_sess.user,
            )
            .map_err(|e| auth_error_msg(e))?;
        } else {
            verify_session_user_proof(headers, chain_sess.user, session_id)?;
        }

        Ok(AuthenticatedEvmSession {
            session_id,
            user: chain_sess.user,
        })
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        let _ = session_id;
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "settlement auth requires building with --features evm-settlement",
                "type": "settlement_unavailable"
            })),
        )
            .into_response())
    }
}

/// `Authorization: Bearer <sessionId>` or `Bearer sk_...` after router activate.
fn parse_bearer_session(headers: &HeaderMap) -> Result<(u64, Option<&str>), &'static str> {
    let authz = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or("missing Authorization header")?;

    let token = authz
        .strip_prefix("Bearer ")
        .or_else(|| authz.strip_prefix("bearer "))
        .ok_or("Authorization must be Bearer <sessionId> or sk_...")?
        .trim();

    if token.is_empty() {
        return Err("empty bearer token");
    }

    if token.starts_with("sk_") {
        let bytes = bs58::decode(token.strip_prefix("sk_").unwrap_or(token))
            .into_vec()
            .map_err(|_| "invalid base58 in sk_ token")?;
        if bytes.len() != 64 {
            return Err("sk_ token must decode to 64 bytes");
        }
        if bytes[..24] != [0u8; 24] {
            return Err("session id in sk_ token exceeds u64 range");
        }
        let session_id = u64::from_be_bytes(bytes[24..32].try_into().unwrap());
        return Ok((session_id, Some(token)));
    }

    let session_id = token
        .parse::<u64>()
        .map_err(|_| "bearer token must be a numeric on-chain session id or sk_...")?;
    Ok((session_id, None))
}

fn auth_error_msg(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": message,
            "type": "invalid_session_auth"
        })),
    )
        .into_response()
}

/// When `X-Sparkl-Message` + `X-Sparkl-Signature` are present, require EIP-191 recovery to `session.user`.
/// If omitted, only on-chain session checks apply (weaker; clients should send both in production).
#[cfg(feature = "evm-settlement")]
fn verify_session_user_proof(
    headers: &HeaderMap,
    session_user: Address,
    session_id: u64,
) -> Result<(), Response> {
    let message = match headers.get("x-sparkl-message").and_then(|v| v.to_str().ok()) {
        Some(m) if !m.is_empty() => m,
        _ => return Ok(()),
    };

    let sig_hex = headers
        .get("x-sparkl-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| auth_error("X-Sparkl-Signature required when X-Sparkl-Message is set"))?;

    let recovered = recover_eip191_signer(message.as_bytes(), sig_hex)
        .map_err(|_| auth_error("invalid X-Sparkl-Signature"))?;

    if recovered != session_user {
        warn!(
            session_id,
            %session_user,
            %recovered,
            "session user does not match signature"
        );
        return Err(auth_error(
            "signature must recover to the session opener (escrow session user)",
        ));
    }

    Ok(())
}

#[cfg(feature = "evm-settlement")]
fn recover_eip191_signer(message: &[u8], sig_hex: &str) -> Result<Address, ()> {
    let sig_bytes = hex::decode(sig_hex.strip_prefix("0x").unwrap_or(sig_hex)).map_err(|_| ())?;
    if sig_bytes.len() != 65 {
        return Err(());
    }

    let v = sig_bytes[64];
    let rec_byte = if v >= 27 { v - 27 } else { v };
    let rec_id = RecoveryId::from_u8_masked(rec_byte);

    let sig =
        RecoverableSignature::from_compact(&sig_bytes[..64], rec_id).map_err(|_| ())?;

    let digest = eip191_message_digest(message);
    let msg = Message::from_digest(digest);
    let pubkey = Secp256k1::verification_only()
        .recover_ecdsa(msg, &sig)
        .map_err(|_| ())?;

    Ok(address_from_secp256k1_pubkey(&pubkey))
}

#[cfg(feature = "evm-settlement")]
fn eip191_message_digest(message: &[u8]) -> [u8; 32] {
    let len = message.len().to_string();
    let mut prefixed =
        Vec::with_capacity(26 + len.len() + message.len());
    prefixed.extend_from_slice(b"\x19Ethereum Signed Message:\n");
    prefixed.extend_from_slice(len.as_bytes());
    prefixed.extend_from_slice(message);
    keccak256(&prefixed).0
}

#[cfg(feature = "evm-settlement")]
fn address_from_secp256k1_pubkey(pubkey: &secp256k1::PublicKey) -> Address {
    let uncompressed = pubkey.serialize_uncompressed();
    let hash = keccak256(&uncompressed[1..]);
    Address::from_slice(&hash[12..])
}

fn auth_error(message: &'static str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": message,
            "type": "invalid_session_auth"
        })),
    )
        .into_response()
}

#[cfg(all(test, feature = "evm-settlement"))]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use secp256k1::Secp256k1;

    #[test]
    fn eip191_recovery_roundtrip() {
        let secp = Secp256k1::new();
        let sk = secp256k1::SecretKey::from_byte_array([0x11u8; 32]).expect("valid key");
        let message = b"sparkl-session-auth:42";
        let digest = eip191_message_digest(message);
        let msg = Message::from_digest(digest);
        let sig = secp.sign_ecdsa_recoverable(msg, &sk);
        let (rec_id, compact) = sig.serialize_compact();
        let mut wire = compact.to_vec();
        wire.push(i32::from(rec_id) as u8 + 27);

        let expected = address_from_secp256k1_pubkey(&sk.public_key(&secp));
        let recovered =
            recover_eip191_signer(message, &hex::encode(wire)).expect("recover");
        assert_eq!(recovered, expected);
        assert_ne!(recovered, Address::ZERO);
    }

    #[test]
    fn parse_bearer_numeric_session_id() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer 7".parse().expect("header value"),
        );
        let (id, sk) = parse_bearer_session(&headers).expect("parse");
        assert_eq!(id, 7);
        assert!(sk.is_none());
    }
}

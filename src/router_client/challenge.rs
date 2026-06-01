//! Router WSS connect challenge (matches sparkl-router `node_auth`).

use alloy_primitives::keccak256;

pub const CONNECT_DOMAIN: &[u8] = b"sparkl-router-connect:";

/// Build the 32-byte payload nodes must sign for `/node/connect`.
#[must_use]
pub fn connect_challenge_payload(nonce: &[u8; 32], block_number: u64) -> [u8; 32] {
    let mut buf = Vec::with_capacity(CONNECT_DOMAIN.len() + 32 + 8);
    buf.extend_from_slice(CONNECT_DOMAIN);
    buf.extend_from_slice(nonce);
    buf.extend_from_slice(&block_number.to_be_bytes());
    keccak256(&buf).0
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn payload_is_deterministic() {
        let nonce = [1u8; 32];
        assert_eq!(
            connect_challenge_payload(&nonce, 42),
            connect_challenge_payload(&nonce, 42),
        );
        assert_ne!(
            connect_challenge_payload(&nonce, 42),
            connect_challenge_payload(&nonce, 43),
        );
    }

    #[test]
    fn sign_roundtrip() {
        let signing_key = SigningKey::from_bytes(&rand::random());
        let nonce = [2u8; 32];
        let payload = connect_challenge_payload(&nonce, 100);
        let sig = signing_key.sign(&payload);
        let vk = signing_key.verifying_key();
        vk.verify_strict(&payload, &sig).expect("verify");
    }
}

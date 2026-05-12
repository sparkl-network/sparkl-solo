//! aggregator-go compatible `certification_request` CBOR (tags 39030 / 39031 / 39032).

use anyhow::{Context, Result};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use crate::receipts::ChunkReceipt;

const TAG_CERTIFICATION_REQUEST: u16 = 39030;
const TAG_CERTIFICATION_DATA: u16 = 39031;
const TAG_PREDICATE: u16 = 39032;

#[derive(Debug, Clone)]
pub struct CertificationRequestBuilt {
    pub state_id_raw: [u8; 32],
    pub request_hex: String,
}

fn write_tag(tag: u16, buf: &mut Vec<u8>) {
    buf.push(0xd9);
    buf.extend_from_slice(&tag.to_be_bytes());
}

/// CBOR definite unsigned integer, major type 0.
fn encode_uint_major0(n: u64, buf: &mut Vec<u8>) {
    if n <= 23 {
        buf.push(n as u8);
        return;
    }
    if n <= 0xff {
        buf.push(0x18);
        buf.push(n as u8);
        return;
    }
    if n <= 0xffff {
        buf.push(0x19);
        buf.extend_from_slice(&(n as u16).to_be_bytes());
        return;
    }
    buf.push(0x1a);
    buf.extend_from_slice(&(n as u32).to_be_bytes());
}

/// Definite-array header (major 4).
fn write_array_header(buf: &mut Vec<u8>, len: usize) {
    let n = len as u64;
    if n <= 23 {
        buf.push(0x80 | n as u8);
        return;
    }
    if n <= 0xff {
        buf.push(0x98);
        buf.push(n as u8);
        return;
    }
    if n <= 0xffff {
        buf.push(0x99);
        buf.extend_from_slice(&(n as u16).to_be_bytes());
        return;
    }
    buf.push(0x9a);
    buf.extend_from_slice(&(n as u32).to_be_bytes());
}

fn write_byte_string(slice: &[u8], buf: &mut Vec<u8>) {
    let n = slice.len();
    if n <= 23 {
        buf.push((2 << 5) | n as u8);
    } else if n <= 0xff {
        buf.push((2 << 5) | 24);
        buf.push(n as u8);
    } else if n <= 0xffff {
        buf.push((2 << 5) | 25);
        buf.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        buf.push((2 << 5) | 26);
        buf.extend_from_slice(&(n as u32).to_be_bytes());
    }
    buf.extend_from_slice(slice);
}

fn prefixed_byte_string_for_hash(slice: &[u8], h: &mut Sha256) {
    let n = slice.len();
    if n <= 23 {
        h.update([(2 << 5) | n as u8]);
    } else if n <= 0xff {
        h.update([(2 << 5) | 24, n as u8]);
    } else if n <= 0xffff {
        let mut hdr = [(2 << 5) | 25, 0u8, 0u8];
        hdr[1..].copy_from_slice(&(n as u16).to_be_bytes());
        h.update(hdr);
    } else {
        let mut hdr = vec![(2 << 5) | 26];
        hdr.extend_from_slice(&(n as u32).to_be_bytes());
        h.update(&hdr);
    }
    h.update(slice);
}

fn write_array_header_to_hasher(len: usize, h: &mut Sha256) {
    let mut v = Vec::new();
    write_array_header(&mut v, len);
    h.update(&v);
}

fn hash_state_id(predicate_cbor: &[u8], source: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    write_array_header_to_hasher(2, &mut h);
    h.update(predicate_cbor);
    prefixed_byte_string_for_hash(source, &mut h);
    h.finalize().into()
}

fn sig_data_raw_hash(source: &[u8; 32], transaction: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    write_array_header_to_hasher(2, &mut h);
    prefixed_byte_string_for_hash(source, &mut h);
    prefixed_byte_string_for_hash(transaction, &mut h);
    h.finalize().into()
}

fn encode_predicate_pay_to_pubkey(compressed_pubkey_33: &[u8; 33]) -> Vec<u8> {
    let mut out = Vec::new();
    write_tag(TAG_PREDICATE, &mut out);
    write_array_header(&mut out, 3);
    encode_uint_major0(1, &mut out);
    write_byte_string(&[1], &mut out);
    write_byte_string(compressed_pubkey_33.as_slice(), &mut out);
    out
}

fn encode_certification_data(
    predicate_cbor: &[u8],
    source: &[u8; 32],
    txn: &[u8; 32],
    witness65: &[u8; 65],
) -> Vec<u8> {
    let mut out = Vec::new();
    write_tag(TAG_CERTIFICATION_DATA, &mut out);
    write_array_header(&mut out, 5);
    encode_uint_major0(1, &mut out);
    out.extend_from_slice(predicate_cbor);
    write_byte_string(source.as_slice(), &mut out);
    write_byte_string(txn.as_slice(), &mut out);
    write_byte_string(witness65.as_slice(), &mut out);
    out
}

fn encode_certification_request(cert_data_cbor: &[u8], state_id: &[u8; 32], aggregate: u64) -> Vec<u8> {
    let mut out = Vec::new();
    write_tag(TAG_CERTIFICATION_REQUEST, &mut out);
    write_array_header(&mut out, 4);
    encode_uint_major0(1, &mut out);
    write_byte_string(state_id.as_slice(), &mut out);
    out.extend_from_slice(cert_data_cbor);
    encode_uint_major0(aggregate, &mut out);
    out
}

fn transaction_preimage(receipt: &ChunkReceipt) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(receipt.session_id.as_bytes());
    h.update(receipt.seq.to_le_bytes());
    h.update(receipt.content_hash);
    h.update(receipt.token_count.to_le_bytes());
    h.update(receipt.timestamp_ms.to_le_bytes());
    h.update(receipt.provider_sig.as_slice());
    h.finalize().into()
}

fn derived_secp256k1_secret(ed25519_secret: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(ed25519_secret);
    h.update(b"sparkl/unicity/v1/secp-anchor");
    h.finalize().into()
}

fn unicity_secp_secret_key(seed: &[u8; 32]) -> Result<SecretKey> {
    let mut acc = *seed;
    for _ in 0..16 {
        match SecretKey::from_byte_array(acc) {
            Ok(sk) => return Ok(sk),
            Err(_) => {
                let mut h = Sha256::new();
                h.update(acc);
                h.update(b"sparkl/unicity/resample");
                acc = h.finalize().into();
            }
        }
    }
    anyhow::bail!("could not derive valid secp256k1 secret for Unicity anchor")
}

/// Unicity witness: R (32) || S (32) || V where V matches Go `convertBtcecToUnicity` (recovery id 0..3).
fn sign_unicity_witness(sig_hash32: &[u8; 32], sk: &SecretKey) -> Result<[u8; 65]> {
    let secp = Secp256k1::signing_only();
    let msg = Message::from_digest(*sig_hash32);
    let sig_rec = secp.sign_ecdsa_recoverable(msg, sk);
    let (recovery_id, rs) = sig_rec.serialize_compact();
    let mut out = [0u8; 65];
    out[..32].copy_from_slice(&rs[..32]);
    out[32..64].copy_from_slice(&rs[32..64]);
    out[64] = i32::from(recovery_id) as u8;
    Ok(out)
}

/// Build hex CBOR `certification_request` + v2 `state_id` for aggregator-go.
pub fn build_certification_request(
    receipt: &ChunkReceipt,
    ed25519_secret: &[u8; 32],
) -> Result<CertificationRequestBuilt> {
    let canonical = crate::receipts::canonical_payload(receipt).context("canonical receipt")?;
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    let source: [u8; 32] = hasher.finalize().into();

    let secp_scalar = derived_secp256k1_secret(ed25519_secret);
    let sk = unicity_secp_secret_key(&secp_scalar)?;

    let pk_compressed = PublicKey::from_secret_key_global(&sk).serialize();

    let predicate_cbor = encode_predicate_pay_to_pubkey(&pk_compressed);
    let state_id = hash_state_id(&predicate_cbor, &source);
    let txn = transaction_preimage(receipt);
    let sig_hash = sig_data_raw_hash(&source, &txn);
    let witness = sign_unicity_witness(&sig_hash, &sk)?;

    let cert_data = encode_certification_data(&predicate_cbor, &source, &txn, &witness);
    let outer = encode_certification_request(&cert_data, &state_id, 1);

    Ok(CertificationRequestBuilt {
        state_id_raw: state_id,
        request_hex: hex::encode(&outer),
    })
}

#[cfg(all(test, feature = "unicity"))]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn certification_request_is_deterministic() {
        let secret = [42u8; 32];
        let receipt = ChunkReceipt {
            session_id: Uuid::nil(),
            provider_id: "mock-peer".into(),
            seq: 7,
            token_count: 50,
            content_hash: [1u8; 32],
            timestamp_ms: 1_700_000_000_000,
            provider_sig: [2u8; 64].to_vec(),
        };
        let a = build_certification_request(&receipt, &secret).unwrap();
        let b = build_certification_request(&receipt, &secret).unwrap();
        assert_eq!(a.state_id_raw, b.state_id_raw);
        assert_eq!(a.request_hex, b.request_hex);
        assert!(a.request_hex.starts_with('d')); // CBOR tagged (hex digit)
        assert_eq!(hex::decode(&a.request_hex).unwrap()[0], 0xd9);
        assert_eq!(a.state_id_raw.len(), 32);
    }
}

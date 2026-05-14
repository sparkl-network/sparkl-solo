use std::fs;
use std::path::PathBuf;
#[cfg(feature = "tpm")]
use std::process::Command;
use std::sync::RwLock;

use alloy_primitives::keccak256;
use anyhow::{anyhow, Context, Result};
use crypto_box::aead::Aead;
use crypto_box::{PublicKey as CryptoPublicKey, SalsaBox, SecretKey};
use ed25519_dalek::{Signer, SigningKey};
use once_cell::sync::OnceCell;
use rand::RngCore;
use serde::{Deserialize, Serialize};
#[cfg(feature = "tpm")]
use sha2::{Digest, Sha256};

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub peer_id: String,
    pub x25519_pubkey: [u8; 32],
    /// Verifying key bytes for Ed25519 (Dalek / signing identity).
    /// **Hub EVM `bytes32` node id** = [`on_chain_node_id_bytes`] of this field — do not use other hashes.
    pub ed25519_pubkey: [u8; 32],
}

/// `bytes32` **`nodeId`** for `ProviderRegistry` and `SettlementEscrow` on Hub EVM.
///
/// **Single canonical rule:** `keccak256(ed25519_pubkey)` over the raw 32-byte public key.
/// Do not use SHA256(x25519_pubkey), libp2p multihash digests, or other recipes for this value.
#[must_use]
pub fn on_chain_node_id_bytes(ed25519_pubkey: &[u8; 32]) -> [u8; 32] {
    keccak256(ed25519_pubkey).0
}

/// `0x`-prefixed 64-hex **`nodeId`** (same JSON field as **`GET /identity`** on the node).
#[must_use]
pub fn on_chain_node_id_hex(ed25519_pubkey: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(on_chain_node_id_bytes(ed25519_pubkey)))
}

#[must_use]
pub fn on_chain_node_id_from_identity(id: &NodeIdentity) -> [u8; 32] {
    on_chain_node_id_bytes(&id.ed25519_pubkey)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecret {
    x25519_secret: [u8; 32],
    ed25519_secret: [u8; 32],
}

#[derive(Debug, Clone)]
struct LoadedIdentity {
    pub public: NodeIdentity,
    pub x25519_secret: [u8; 32],
    pub ed25519_secret: [u8; 32],
    pub key_source: KeySource,
}

static IDENTITY: OnceCell<RwLock<Option<LoadedIdentity>>> = OnceCell::new();

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum KeySource {
    Software,
    TpmRng,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IdentityMeta {
    key_source: KeySource,
}

pub async fn load_or_generate(config: &Config) -> Result<NodeIdentity> {
    let dir = config.node.data_dir.clone();
    fs::create_dir_all(&dir).context("failed to create data dir")?;

    let public_path = dir.join("identity.json");
    let secret_path = dir.join("identity-secret.json");
    let meta_path = dir.join("identity-meta.json");

    let loaded = if public_path.exists() && secret_path.exists() {
        let public: NodeIdentity = serde_json::from_slice(
            &fs::read(&public_path).context("failed to read identity.json")?,
        )
        .context("invalid identity.json")?;
        let secret: StoredSecret = serde_json::from_slice(
            &fs::read(&secret_path).context("failed to read identity-secret.json")?,
        )
        .context("invalid identity-secret.json")?;
        let meta: IdentityMeta = if meta_path.exists() {
            serde_json::from_slice(
                &fs::read(&meta_path).context("failed to read identity-meta.json")?,
            )
            .context("invalid identity-meta.json")?
        } else {
            IdentityMeta {
                key_source: KeySource::Software,
            }
        };
        LoadedIdentity {
            public,
            x25519_secret: secret.x25519_secret,
            ed25519_secret: secret.ed25519_secret,
            key_source: meta.key_source,
        }
    } else {
        let preferred_source = key_source_for_generation();
        let (x25519_secret, ed25519_secret, key_source) =
            generate_secret_material(preferred_source)?;

        let x_secret = SecretKey::from(x25519_secret);
        let x_public: [u8; 32] = x_secret.public_key().to_bytes();
        let signing = SigningKey::from_bytes(&ed25519_secret);
        let ed_pub = signing.verifying_key().to_bytes();

        let peer_prefix = match key_source {
            KeySource::Software => "mock",
            KeySource::TpmRng => "tpm",
        };
        let peer_id = format!("{peer_prefix}-{}", hex::encode(&x_public[..8]));
        let public = NodeIdentity {
            peer_id,
            x25519_pubkey: x_public,
            ed25519_pubkey: ed_pub,
        };
        let secret = StoredSecret {
            x25519_secret,
            ed25519_secret,
        };

        fs::write(&public_path, serde_json::to_vec_pretty(&public)?)
            .context("failed to write identity.json")?;
        fs::write(&secret_path, serde_json::to_vec_pretty(&secret)?)
            .context("failed to write identity-secret.json")?;
        fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&IdentityMeta { key_source })?,
        )
        .context("failed to write identity-meta.json")?;

        LoadedIdentity {
            public,
            x25519_secret,
            ed25519_secret,
            key_source,
        }
    };

    let cell = IDENTITY.get_or_init(|| RwLock::new(None));
    let mut guard = cell
        .write()
        .map_err(|_| anyhow!("identity lock poisoned"))?;
    *guard = Some(loaded.clone());
    Ok(loaded.public)
}

pub async fn sign_challenge(nonce: &[u8; 32]) -> Result<[u8; 64]> {
    let loaded = require_loaded()?;
    let key = SigningKey::from_bytes(&loaded.ed25519_secret);
    Ok(key.sign(nonce).to_bytes())
}

pub async fn decrypt_request(ciphertext: &[u8], ephemeral_pubkey: &[u8; 32]) -> Result<Vec<u8>> {
    if ciphertext.len() < 24 {
        return Err(anyhow!("ciphertext shorter than nonce"));
    }
    let loaded = require_loaded()?;
    let secret_key = SecretKey::from(loaded.x25519_secret);
    let peer = CryptoPublicKey::from(*ephemeral_pubkey);
    let salsa = SalsaBox::new(&peer, &secret_key);
    let nonce = crypto_box::Nonce::from_slice(&ciphertext[..24]);
    let plaintext = salsa
        .decrypt(nonce, &ciphertext[24..])
        .map_err(|_| anyhow!("request decryption failed"))?;
    Ok(plaintext)
}

pub fn sign_bytes(payload: &[u8]) -> Result<[u8; 64]> {
    let loaded = require_loaded()?;
    let key = SigningKey::from_bytes(&loaded.ed25519_secret);
    Ok(key.sign(payload).to_bytes())
}

pub fn current_identity() -> Result<NodeIdentity> {
    Ok(require_loaded()?.public)
}

pub fn attestation_cert_type() -> Result<&'static str> {
    let loaded = require_loaded()?;
    Ok(match loaded.key_source {
        KeySource::Software => "mock-software",
        KeySource::TpmRng => "swtpm",
    })
}

fn require_loaded() -> Result<LoadedIdentity> {
    let cell = IDENTITY
        .get()
        .ok_or_else(|| anyhow!("identity not initialized"))?;
    let guard = cell.read().map_err(|_| anyhow!("identity lock poisoned"))?;
    guard
        .clone()
        .ok_or_else(|| anyhow!("identity not initialized"))
}

fn key_source_for_generation() -> KeySource {
    #[cfg(feature = "tpm")]
    {
        if tpm_runtime_requested() {
            return KeySource::TpmRng;
        }
    }
    KeySource::Software
}

fn generate_secret_material(key_source: KeySource) -> Result<([u8; 32], [u8; 32], KeySource)> {
    match key_source {
        KeySource::Software => {
            let mut x25519_secret = [0u8; 32];
            let mut ed25519_secret = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut x25519_secret);
            rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);
            Ok((x25519_secret, ed25519_secret, KeySource::Software))
        }
        KeySource::TpmRng => {
            #[cfg(feature = "tpm")]
            {
                match tpm_getrandom_64() {
                    Ok(seed) => {
                        let (x25519_secret, ed25519_secret) = derive_secrets_from_seed(seed);
                        Ok((x25519_secret, ed25519_secret, KeySource::TpmRng))
                    }
                    Err(_) => {
                        let mut x25519_secret = [0u8; 32];
                        let mut ed25519_secret = [0u8; 32];
                        rand::rngs::OsRng.fill_bytes(&mut x25519_secret);
                        rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);
                        Ok((x25519_secret, ed25519_secret, KeySource::Software))
                    }
                }
            }
            #[cfg(not(feature = "tpm"))]
            {
                let mut x25519_secret = [0u8; 32];
                let mut ed25519_secret = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut x25519_secret);
                rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);
                Ok((x25519_secret, ed25519_secret, KeySource::Software))
            }
        }
    }
}

#[cfg(feature = "tpm")]
fn derive_secrets_from_seed(seed: [u8; 64]) -> ([u8; 32], [u8; 32]) {
    let mut x_hasher = Sha256::new();
    x_hasher.update(seed);
    x_hasher.update(b"sparkl/x25519");
    let x25519_secret: [u8; 32] = x_hasher.finalize().into();

    let mut ed_hasher = Sha256::new();
    ed_hasher.update(seed);
    ed_hasher.update(b"sparkl/ed25519");
    let ed25519_secret: [u8; 32] = ed_hasher.finalize().into();

    (x25519_secret, ed25519_secret)
}

#[cfg(feature = "tpm")]
fn tpm_runtime_requested() -> bool {
    std::env::var("TCTI").is_ok() || std::env::var("TPM2TOOLS_TCTI").is_ok()
}

#[cfg(feature = "tpm")]
fn tpm_getrandom_64() -> Result<[u8; 64]> {
    let bytes = 64usize;
    let output = Command::new("tpm2_getrandom")
        .arg(bytes.to_string())
        .arg("--hex")
        .output()
        .context("failed to run tpm2_getrandom")?;
    if !output.status.success() {
        return Err(anyhow!(
            "tpm2_getrandom exited with status {}",
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout).context("tpm2_getrandom output was not utf8")?;
    let compact = stdout
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    if compact.len() < bytes * 2 {
        return Err(anyhow!(
            "tpm2_getrandom returned too few hex bytes: expected {}, got {}",
            bytes * 2,
            compact.len()
        ));
    }
    let raw = hex::decode(&compact[..bytes * 2]).context("invalid hex from tpm2_getrandom")?;
    let arr: [u8; 64] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("unexpected tpm2_getrandom length"))?;
    Ok(arr)
}

#[allow(dead_code)]
fn _identity_dir(data_dir: &PathBuf) -> PathBuf {
    data_dir.clone()
}

#[cfg(test)]
mod on_chain_node_id_tests {
    use super::{on_chain_node_id_bytes, on_chain_node_id_hex};
    use alloy_primitives::keccak256;

    #[test]
    fn on_chain_node_id_matches_keccak256_of_ed25519_pubkey() {
        let pk: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(17));
        let expected = keccak256(pk);
        assert_eq!(on_chain_node_id_bytes(&pk), expected.0);
    }

    #[test]
    fn on_chain_node_id_hex_format() {
        let pk = [0xabu8; 32];
        let h = on_chain_node_id_hex(&pk);
        assert!(h.starts_with("0x"));
        assert_eq!(h.len(), 2 + 64);
    }
}

use std::fs;
use std::path::{Path, PathBuf};
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

/// Deterministic X25519 encryption secret for `version >= 1` (rotation without extra backup).
#[must_use]
pub fn derive_versioned_x25519_secret(ed25519_secret: &[u8; 32], version: u32) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(ed25519_secret);
    h.update(b"sparkl/x25519-encryption-v");
    h.update(version.to_be_bytes());
    h.finalize().into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecret {
    x25519_secret: [u8; 32],
    ed25519_secret: [u8; 32],
    /// `0` = legacy random `x25519_secret`; `>= 1` = secret matches [`derive_versioned_x25519_secret`] for this version.
    #[serde(default)]
    x25519_version: u32,
    /// Preserved random X25519 secret after migrating from version 0 → 1 so old ConsumerKeys still decrypt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_x25519_secret: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
struct LoadedIdentity {
    pub public: NodeIdentity,
    pub x25519_secret: [u8; 32],
    pub x25519_version: u32,
    pub legacy_x25519_secret: Option<[u8; 32]>,
    pub ed25519_secret: [u8; 32],
    pub key_source: KeySource,
    data_dir: PathBuf,
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

/// Load existing `identity.json` + `identity-secret.json` only (no generation). Initializes [`IDENTITY`].
pub fn load_existing(config: &Config) -> Result<NodeIdentity> {
    let dir = config.node.data_dir.clone();
    let public_path = dir.join("identity.json");
    let secret_path = dir.join("identity-secret.json");
    let meta_path = dir.join("identity-meta.json");

    if !public_path.exists() || !secret_path.exists() {
        return Err(anyhow!(
            "identity files not found (expected {:?} and {:?}); refuse to generate new keys for rotate-encryption-key",
            public_path,
            secret_path
        ));
    }

    let public: NodeIdentity =
        serde_json::from_slice(&fs::read(&public_path).context("failed to read identity.json")?)
            .context("invalid identity.json")?;
    let secret: StoredSecret = serde_json::from_slice(
        &fs::read(&secret_path).context("failed to read identity-secret.json")?,
    )
    .context("invalid identity-secret.json")?;
    let meta: IdentityMeta = if meta_path.exists() {
        serde_json::from_slice(&fs::read(&meta_path).context("failed to read identity-meta.json")?)
            .context("invalid identity-meta.json")?
    } else {
        IdentityMeta {
            key_source: KeySource::Software,
        }
    };

    if secret.x25519_version >= 1 {
        let expected =
            derive_versioned_x25519_secret(&secret.ed25519_secret, secret.x25519_version);
        if expected != secret.x25519_secret {
            return Err(anyhow!(
                "identity-secret.json: x25519_secret does not match derived key for version {}",
                secret.x25519_version
            ));
        }
    }

    let loaded = LoadedIdentity {
        public,
        x25519_secret: secret.x25519_secret,
        x25519_version: secret.x25519_version,
        legacy_x25519_secret: secret.legacy_x25519_secret,
        ed25519_secret: secret.ed25519_secret,
        key_source: meta.key_source,
        data_dir: dir,
    };

    let cell = IDENTITY.get_or_init(|| RwLock::new(None));
    let mut guard = cell
        .write()
        .map_err(|_| anyhow!("identity lock poisoned"))?;
    *guard = Some(loaded.clone());
    Ok(loaded.public)
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

        if secret.x25519_version >= 1 {
            let expected =
                derive_versioned_x25519_secret(&secret.ed25519_secret, secret.x25519_version);
            if expected != secret.x25519_secret {
                return Err(anyhow!(
                    "identity-secret.json: x25519_secret does not match derived key for version {}",
                    secret.x25519_version
                ));
            }
        }

        LoadedIdentity {
            public,
            x25519_secret: secret.x25519_secret,
            x25519_version: secret.x25519_version,
            legacy_x25519_secret: secret.legacy_x25519_secret,
            ed25519_secret: secret.ed25519_secret,
            key_source: meta.key_source,
            data_dir: dir.clone(),
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
            x25519_version: 1,
            legacy_x25519_secret: None,
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
            x25519_version: 1,
            legacy_x25519_secret: None,
            ed25519_secret,
            key_source,
            data_dir: dir.clone(),
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

/// Decrypt using a specific historical encryption version (`0` = legacy random key on disk).
pub fn decrypt_request_versioned(
    ciphertext: &[u8],
    ephemeral_pubkey: &[u8; 32],
    version: u32,
) -> Result<Vec<u8>> {
    if ciphertext.len() < 24 {
        return Err(anyhow!("ciphertext shorter than nonce"));
    }
    let loaded = require_loaded()?;
    let sk_bytes = if version == 0 {
        if let Some(leg) = loaded.legacy_x25519_secret {
            leg
        } else if loaded.x25519_version == 0 {
            loaded.x25519_secret
        } else {
            return Err(anyhow!(
                "no legacy x25519 secret available for encryption version 0"
            ));
        }
    } else {
        derive_versioned_x25519_secret(&loaded.ed25519_secret, version)
    };

    let secret_key = SecretKey::from(sk_bytes);
    let peer = CryptoPublicKey::from(*ephemeral_pubkey);
    let salsa = SalsaBox::new(&peer, &secret_key);
    let nonce = crypto_box::Nonce::from_slice(&ciphertext[..24]);
    let plaintext = salsa
        .decrypt(nonce, &ciphertext[24..])
        .map_err(|_| anyhow!("request decryption failed (version {version})"))?;
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

/// Current X25519 encryption public key (matches on-chain headline when registered).
pub fn current_encryption_pubkey() -> Result<[u8; 32]> {
    let loaded = require_loaded()?;
    let secret_key = SecretKey::from(loaded.x25519_secret);
    Ok(secret_key.public_key().to_bytes())
}

pub fn current_encryption_key_version() -> Result<u32> {
    Ok(require_loaded()?.x25519_version)
}

/// Material for the next `ProviderRegistry.rotateEncryptionKey` call. `new_version` is `max(1, current+1)`.
pub fn prepare_encryption_rotation() -> Result<(u32, [u8; 32], [u8; 32])> {
    let loaded = require_loaded()?;
    let next_ver = (loaded.x25519_version + 1).max(1);
    let new_secret = derive_versioned_x25519_secret(&loaded.ed25519_secret, next_ver);
    let pk = SecretKey::from(new_secret).public_key().to_bytes();
    Ok((next_ver, new_secret, pk))
}

/// After a successful on-chain `rotateEncryptionKey`, persist the next derived secret and bump the local version.
pub fn persist_encryption_key_rotation(
    new_version: u32,
    new_x25519_secret: [u8; 32],
) -> Result<()> {
    let expected = derive_versioned_x25519_secret(&require_loaded()?.ed25519_secret, new_version);
    if expected != new_x25519_secret {
        return Err(anyhow!(
            "new x25519 secret does not match deterministic derivation for version {new_version}"
        ));
    }

    let cell = IDENTITY
        .get()
        .ok_or_else(|| anyhow!("identity not initialized"))?;
    let mut guard = cell
        .write()
        .map_err(|_| anyhow!("identity lock poisoned"))?;
    let loaded = guard
        .as_mut()
        .ok_or_else(|| anyhow!("identity not initialized"))?;

    let old_ver = loaded.x25519_version;
    let old_sec = loaded.x25519_secret;
    let mut legacy = loaded.legacy_x25519_secret;
    if old_ver == 0 {
        legacy = Some(old_sec);
    }

    let x_secret = SecretKey::from(new_x25519_secret);
    let x_public = x_secret.public_key().to_bytes();

    loaded.x25519_secret = new_x25519_secret;
    loaded.x25519_version = new_version;
    loaded.legacy_x25519_secret = legacy;
    loaded.public.x25519_pubkey = x_public;

    let secret = StoredSecret {
        x25519_secret: new_x25519_secret,
        ed25519_secret: loaded.ed25519_secret,
        x25519_version: new_version,
        legacy_x25519_secret: loaded.legacy_x25519_secret,
    };

    let dir = &loaded.data_dir;
    fs::write(
        dir.join("identity.json"),
        serde_json::to_vec_pretty(&loaded.public)?,
    )
    .context("failed to write identity.json")?;
    fs::write(
        dir.join("identity-secret.json"),
        serde_json::to_vec_pretty(&secret)?,
    )
    .context("failed to write identity-secret.json")?;

    Ok(())
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
            let mut ed25519_secret = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);
            let x25519_secret = derive_versioned_x25519_secret(&ed25519_secret, 1);
            Ok((x25519_secret, ed25519_secret, KeySource::Software))
        }
        KeySource::TpmRng => {
            #[cfg(feature = "tpm")]
            {
                match tpm_getrandom_64() {
                    Ok(seed) => {
                        let (_x_old, ed25519_secret) = derive_secrets_from_seed(seed);
                        let x25519_secret = derive_versioned_x25519_secret(&ed25519_secret, 1);
                        Ok((x25519_secret, ed25519_secret, KeySource::TpmRng))
                    }
                    Err(_) => {
                        let mut ed25519_secret = [0u8; 32];
                        rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);
                        let x25519_secret = derive_versioned_x25519_secret(&ed25519_secret, 1);
                        Ok((x25519_secret, ed25519_secret, KeySource::Software))
                    }
                }
            }
            #[cfg(not(feature = "tpm"))]
            {
                let mut ed25519_secret = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);
                let x25519_secret = derive_versioned_x25519_secret(&ed25519_secret, 1);
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
fn _identity_dir(data_dir: &Path) -> PathBuf {
    data_dir.to_path_buf()
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

#[cfg(test)]
mod versioned_derive_tests {
    use super::derive_versioned_x25519_secret;
    use crypto_box::SecretKey;

    #[test]
    fn derive_versioned_is_stable() {
        let ed = [7u8; 32];
        let a = derive_versioned_x25519_secret(&ed, 1);
        let b = derive_versioned_x25519_secret(&ed, 1);
        assert_eq!(a, b);
        assert_ne!(a, derive_versioned_x25519_secret(&ed, 2));
    }

    #[test]
    fn derive_versioned_pubkey_is_deterministic_from_ed_and_version() {
        let ed = [0xabu8; 32];
        for v in [1u32, 2u32, 99u32] {
            let sk = derive_versioned_x25519_secret(&ed, v);
            let pk1 = SecretKey::from(sk).public_key().to_bytes();
            let pk2 = SecretKey::from(derive_versioned_x25519_secret(&ed, v))
                .public_key()
                .to_bytes();
            assert_eq!(pk1, pk2);
        }
    }
}

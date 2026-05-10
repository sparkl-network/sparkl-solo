use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{anyhow, Context, Result};
use crypto_box::aead::Aead;
use crypto_box::{PublicKey as CryptoPublicKey, SalsaBox, SecretKey};
use ed25519_dalek::{Signer, SigningKey};
use once_cell::sync::OnceCell;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub peer_id: String,
    pub x25519_pubkey: [u8; 32],
    pub ed25519_pubkey: [u8; 32],
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
}

static IDENTITY: OnceCell<RwLock<Option<LoadedIdentity>>> = OnceCell::new();

pub async fn load_or_generate(config: &Config) -> Result<NodeIdentity> {
    let dir = config.node.data_dir.clone();
    fs::create_dir_all(&dir).context("failed to create data dir")?;

    let public_path = dir.join("identity.json");
    let secret_path = dir.join("identity-secret.json");

    let loaded = if public_path.exists() && secret_path.exists() {
        let public: NodeIdentity = serde_json::from_slice(
            &fs::read(&public_path).context("failed to read identity.json")?,
        )
        .context("invalid identity.json")?;
        let secret: StoredSecret = serde_json::from_slice(
            &fs::read(&secret_path).context("failed to read identity-secret.json")?,
        )
        .context("invalid identity-secret.json")?;
        LoadedIdentity {
            public,
            x25519_secret: secret.x25519_secret,
            ed25519_secret: secret.ed25519_secret,
        }
    } else {
        let mut x25519_secret = [0u8; 32];
        let mut ed25519_secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut x25519_secret);
        rand::rngs::OsRng.fill_bytes(&mut ed25519_secret);

        let x_secret = SecretKey::from(x25519_secret);
        let x_public: [u8; 32] = x_secret.public_key().to_bytes();
        let signing = SigningKey::from_bytes(&ed25519_secret);
        let ed_pub = signing.verifying_key().to_bytes();

        let peer_id = format!("mock-{}", hex::encode(&x_public[..8]));
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

        LoadedIdentity {
            public,
            x25519_secret,
            ed25519_secret,
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

fn require_loaded() -> Result<LoadedIdentity> {
    let cell = IDENTITY
        .get()
        .ok_or_else(|| anyhow!("identity not initialized"))?;
    let guard = cell.read().map_err(|_| anyhow!("identity lock poisoned"))?;
    guard
        .clone()
        .ok_or_else(|| anyhow!("identity not initialized"))
}

#[allow(dead_code)]
fn _identity_dir(data_dir: &PathBuf) -> PathBuf {
    data_dir.clone()
}

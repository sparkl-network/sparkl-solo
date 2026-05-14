// Provider Registration Flow — full on-chain integration with ProviderRegistry.
//
// When the `evm-settlement` feature is enabled, this module calls the deployed
// ProviderRegistry contract to register the node, submit TEE proofs, and
// query peer state.  Without that feature the module falls back to a
// log-only stub that keeps the node operational.
//
// See AGENTS.md for the stub list.
// See docs/MVP_ROADMAP.md for the full MVP context.

use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "evm-settlement")]
use anyhow::Context;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{error, info, warn};

#[cfg(feature = "evm-settlement")]
use alloy::primitives::{Address, Bytes, FixedBytes, U256};
#[cfg(feature = "evm-settlement")]
use alloy::providers::Provider;
#[cfg(feature = "evm-settlement")]
use alloy::signers::local::PrivateKeySigner;
#[cfg(feature = "evm-settlement")]
use alloy::sol;
#[cfg(feature = "evm-settlement")]
use alloy::transactors::Sendable;

use crate::config::RegistryConfig;
use crate::identity::NodeIdentity;
use crate::proxy::BackendProxy;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Mirrors `NodeInfo` in `contracts/src/SecurityTypes.sol`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub payout: String,
    pub fee_bps: u16,
    pub active: bool,
    pub supports_best_effort: bool,
    pub supports_tee: bool,
    pub tee_report_hash: String,
    pub metadata_uri: String,
    pub lifecycle: String,
}

/// Mirrors `NodeLifecycle` in `contracts/src/SecurityTypes.sol`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Lifecycle {
    #[default]
    Active,
    Chilled,
    Defunct,
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lifecycle::Active => write!(f, "Active"),
            Lifecycle::Chilled => write!(f, "Chilled"),
            Lifecycle::Defunct => write!(f, "Defunct"),
        }
    }
}

/// Mirrors `SecurityTier` in `contracts/src/SecurityTypes.sol`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SecurityTier {
    #[default]
    BestEffort,
    TeeVerified,
}

impl std::fmt::Display for SecurityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecurityTier::BestEffort => write!(f, "BEST_EFFORT"),
            SecurityTier::TeeVerified => write!(f, "TEE_VERIFIED"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderState {
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub models: Vec<String>,
    pub attestation_hash: String,
    pub gpu_memory_gb: u32,
    pub price_input_m: u64,
    pub price_output_m: u64,
    pub node_version: String,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InclusionProof {
    pub token_id: String,
    pub proof: String,
}

// ---------------------------------------------------------------------------
// Solidity ABIs (generated from ProviderRegistry.sol — hand-rolled for
// zero-dependency ABI embedding when evm-settlement is active)
// ---------------------------------------------------------------------------

#[cfg(feature = "evm-settlement")]
sol! {
    #[sol(rpc)]
    contract ProviderRegistry {
        function registerNode(
            bytes32 nodeId,
            address payout,
            bool supportsBestEffort,
            bool supportsTEE,
            string calldata metadataURI
        ) external;

        function chillNode(bytes32 nodeId) external;

        function setNodePayout(bytes32 nodeId, address payout) external;

        function setNodeActive(bytes32 nodeId, bool active) external;

        function setNodeMetadata(bytes32 nodeId, string calldata uri) external;

        function setTEEProof(bytes32 nodeId, bytes32 teeReportHash) external;

        function setNodePricing(
            bytes32 nodeId,
            uint8 tier,
            uint256 pricePer1kTokens
        ) external;

        function getProvider(bytes32 nodeId)
            external
            view
            returns (
                address payout,
                uint16 feeBps,
                bool active,
                bool supportsBestEffort,
                bool supportsTEE,
                bytes32 teeReportHash,
                string memory metadataURI,
                uint8 lifecycle
            );

        function supportsTier(bytes32 nodeId, uint8 tier)
            external
            view
            returns (bool);

        function nodeOperator(bytes32 nodeId) external view returns (address);

        function operatorNodes(address operator)
            external
            view
            returns (bytes32[] memory);
    }
}

// ---------------------------------------------------------------------------
// Helper: convert hex-encoded private key to a signer
// ---------------------------------------------------------------------------

#[cfg(feature = "evm-settlement")]
fn parse_evm_signer(pk: &str) -> Result<PrivateKeySigner> {
    let pk = pk.strip_prefix("0x").unwrap_or(pk);
    let bytes = hex::decode(pk)
        .map_err(|e| anyhow!("invalid evm_provider_wallet_private_key hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(anyhow!(
            "evm_provider_wallet_private_key must be 32 bytes, got {}",
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(PrivateKeySigner::from_bytes(&arr)?)
}

// ---------------------------------------------------------------------------
// register — call ProviderRegistry.registerNode()
// ---------------------------------------------------------------------------

#[allow(unused_variables)]
pub async fn register(
    identity: &NodeIdentity,
    state: &ProviderState,
    config: &RegistryConfig,
) -> Result<InclusionProof> {
    if !config.enabled {
        warn!(peer_id = %state.peer_id, "registry disabled; skipping register");
        return Ok(InclusionProof {
            token_id: "disabled".to_string(),
            proof: "disabled".to_string(),
        });
    }

    #[cfg(feature = "evm-settlement")]
    {
        // EVM provider wallet is required for on-chain registration.
        let pk = config
            .evm_provider_wallet_private_key
            .as_ref()
            .ok_or_else(|| anyhow!("registry enabled but evm_provider_wallet_private_key not configured"))?;

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = config
            .registry_contract_address
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let instance = ProviderRegistry::new(registry_addr, Arc::new(signer.create_provider()));

        // Derive a deterministic nodeId from the x25519 pubkey hash.
        let node_id = derive_node_id(identity);

        // Build metadata URI (CID or IPFS reference).
        let metadata_uri = format!(
            "ipfs://{}/provider/{}",
            sha256_hex(&state.peer_id.as_bytes()),
            state.peer_id
        );

        let supports_tee = !state.attestation_hash.is_empty();

        info!(
            node_id = %hex::encode(node_id),
            payout = %signer.address(),
            supports_best_effort = true,
            supports_tee = supports_tee,
            "registering node on ProviderRegistry"
        );

        let tx = instance
            .registerNode(
                node_id,
                signer.address(), // payout defaults to msg.sender
                true,             // supportsBestEffort
                supports_tee,     // supportsTEE
                &metadata_uri,
            )
            .send()
            .await
            .map_err(|e| anyhow!("registerNode tx failed: {e}"))?;

        let receipt = tx
            .get_receipt()
            .await
            .map_err(|e| anyhow!("registerNode receipt failed: {e}"))?;

        info!(
            tx_hash = %receipt.transaction_hash,
            block = receipt.block_number,
            "node registered successfully"
        );

        return Ok(InclusionProof {
            token_id: format!("tok-{}", hex::encode(node_id)),
            proof: hex::encode(receipt.transaction_hash.as_slice()),
        });
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; register is stubbed");
        Ok(InclusionProof {
            token_id: format!("tok-{}", state.peer_id),
            proof: "stub-proof".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// heartbeat — submit TEE proof update to ProviderRegistry
// ---------------------------------------------------------------------------

#[allow(unused_variables)]
pub async fn heartbeat(
    identity: &NodeIdentity,
    state: &ProviderState,
    config: &RegistryConfig,
) -> Result<()> {
    if !config.enabled {
        return Ok(());
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = config
            .evm_provider_wallet_private_key
            .as_ref()
            .ok_or_else(|| anyhow!("heartbeat: evm_provider_wallet_private_key not configured"))?;

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = config
            .registry_contract_address
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let instance = ProviderRegistry::new(registry_addr, Arc::new(signer.create_provider()));
        let node_id = derive_node_id(identity);

        // Only submit TEE proof if we have a non-empty attestation hash.
        if state.attestation_hash.is_empty() {
            info!(
                node_id = %hex::encode(node_id),
                "heartbeat: no attestation hash; skipping setTEEProof"
            );
            return Ok(());
        }

        // Parse the attestation hash as bytes32.
        let tee_hash: FixedBytes<32> = parse_bytes32(&state.attestation_hash).into();

        info!(
            node_id = %hex::encode(node_id),
            tee_report = %hex::encode(tee_hash.as_slice()),
            "submitting TEE proof via setTEEProof"
        );

        let tx = instance
            .setTEEProof(node_id, tee_hash)
            .send()
            .await
            .map_err(|e| anyhow!("setTEEProof tx failed: {e}"))?;

        let receipt = tx
            .get_receipt()
            .await
            .map_err(|e| anyhow!("setTEEProof receipt failed: {e}"))?;

        info!(
            tx_hash = %receipt.transaction_hash,
            "TEE proof submitted successfully"
        );

        // Update pricing if configured.
        if let Some(price) = config.tier_a_price_per_1k_tokens {
            let price_u256 = U256::from(price);
            let tx2 = instance
                .setNodePricing(node_id, 1u8, price_u256) // tier=TEE_VERIFIED=1
                .send()
                .await
                .map_err(|e| anyhow!("setNodePricing tx failed: {e}"))?;
            let _ = tx2.get_receipt().await;
            info!(price = price, "node pricing updated");
        }

        Ok(())
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; heartbeat is stubbed");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// deregister — chill the node on-chain
// ---------------------------------------------------------------------------

#[allow(unused_variables)]
pub async fn deregister(
    identity: &NodeIdentity,
    config: &RegistryConfig,
) -> Result<()> {
    if !config.enabled {
        warn!("registry disabled; skipping deregister");
        return Ok(());
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = config
            .evm_provider_wallet_private_key
            .as_ref()
            .ok_or_else(|| anyhow!("deregister: evm_provider_wallet_private_key not configured"))?;

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = config
            .registry_contract_address
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let instance = ProviderRegistry::new(registry_addr, Arc::new(signer.create_provider()));
        let node_id = derive_node_id(identity);

        info!(
            node_id = %hex::encode(node_id),
            "chilling node on ProviderRegistry"
        );

        let tx = instance
            .chillNode(node_id)
            .send()
            .await
            .map_err(|e| anyhow!("chillNode tx failed: {e}"))?;

        let receipt = tx
            .get_receipt()
            .await
            .map_err(|e| anyhow!("chillNode receipt failed: {e}"))?;

        info!(
            tx_hash = %receipt.transaction_hash,
            "node chilled successfully"
        );

        Ok(())
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; deregister is stubbed");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// get_peer_info — query ProviderRegistry for a node's on-chain state
// ---------------------------------------------------------------------------

#[allow(unused_variables)]
pub async fn get_peer_info(
    config: &RegistryConfig,
    node_id: [u8; 32],
) -> Result<Option<ProviderInfo>> {
    if !config.enabled {
        warn!("registry disabled; skipping get_peer_info");
        return Ok(None);
    }

    #[cfg(feature = "evm-settlement")]
    {
        let registry_addr: Address = config
            .registry_contract_address
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        // Read-only query — no signer needed.
        use alloy::providers::ProviderBuilder;
        let rpc_url = config
            .evm_rpc_url
            .as_ref()
            .ok_or_else(|| anyhow!("get_peer_info requires evm_rpc_url"))?;

        let provider = ProviderBuilder::new()
            .on_http(rpc_url.parse().context("invalid evm_rpc_url")?);

        let instance = ProviderRegistry::new(registry_addr, provider);
        let result = instance
            .getProvider(node_id)
            .call()
            .await
            .map_err(|e| anyhow!("getProvider call failed: {e}"))?;

        let lifecycle = match result.lifecycle {
            0 => Lifecycle::Active,
            1 => Lifecycle::Chilled,
            2 => Lifecycle::Defunct,
            _ => {
                warn!(lifecycle = result.lifecycle, "unknown lifecycle value");
                Lifecycle::Active
            }
        };

        Ok(Some(ProviderInfo {
            payout: hex::encode(result.payout.as_slice()),
            fee_bps: result.feeBps,
            active: result.active,
            supports_best_effort: result.supportsBestEffort,
            supports_tee: result.supportsTEE,
            tee_report_hash: hex::encode(result.teeReportHash.as_slice()),
            metadata_uri: result.metadataURI.to_string(),
            lifecycle: format!("{:?}", lifecycle),
        }))
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; get_peer_info is stubbed");
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// supports_tier — check if a node supports a given security tier
// ---------------------------------------------------------------------------

#[allow(unused_variables)]
pub async fn supports_tier(
    config: &RegistryConfig,
    node_id: [u8; 32],
    tier: SecurityTier,
) -> Result<bool> {
    if !config.enabled {
        return Ok(false);
    }

    #[cfg(feature = "evm-settlement")]
    {
        let registry_addr: Address = config
            .registry_contract_address
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        use alloy::providers::ProviderBuilder;
        let rpc_url = config
            .evm_rpc_url
            .as_ref()
            .ok_or_else(|| anyhow!("supports_tier requires evm_rpc_url"))?;

        let provider = ProviderBuilder::new()
            .on_http(rpc_url.parse().context("invalid evm_rpc_url")?);

        let instance = ProviderRegistry::new(registry_addr, provider);
        let tier_u8 = match tier {
            SecurityTier::BestEffort => 0u8,
            SecurityTier::TeeVerified => 1u8,
        };

        let result = instance
            .supportsTier(node_id, tier_u8)
            .call()
            .await
            .map_err(|e| anyhow!("supportsTier call failed: {e}"))?;

        Ok(result)
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        // Without EVM, fall back to local assessment.
        match tier {
            SecurityTier::TeeVerified => Ok(false),
            SecurityTier::BestEffort => Ok(true),
        }
    }
}

// ---------------------------------------------------------------------------
// startup_register_with_retry — register on startup with exponential backoff
// ---------------------------------------------------------------------------

pub async fn startup_register_with_retry(
    identity: Arc<NodeIdentity>,
    proxy: Arc<BackendProxy>,
    config: RegistryConfig,
    max_retries: u32,
    initial_delay_secs: u64,
) -> Result<InclusionProof> {
    let mut delay = Duration::from_secs(initial_delay_secs.max(1));

    for attempt in 1..=max_retries {
        info!(attempt, max_retries, "starting registration attempt");

        // Gather current model list for the state.
        let models = proxy
            .list_models()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>();

        let state = ProviderState {
            peer_id: identity.peer_id.clone(),
            multiaddrs: vec![],
            models,
            attestation_hash: String::new(), // will be filled by attestation module
            gpu_memory_gb: 0,
            price_input_m: 0,
            price_output_m: 0,
            node_version: env!("CARGO_PKG_VERSION").to_string(),
            last_seen_ms: 0,
        };

        match register(&identity, &state, &config).await {
            Ok(proof) => {
                info!(
                    token_id = %proof.token_id,
                    attempt,
                    "registration succeeded"
                );
                return Ok(proof);
            }
            Err(e) => {
                warn!(
                    attempt,
                    error = %e,
                    "registration attempt failed, retrying..."
                );
                if attempt < max_retries {
                    sleep(delay).await;
                    delay *= 2; // exponential backoff
                }
            }
        }
    }

    Err(anyhow!(
        "registration failed after {} retries (log-only mode)",
        max_retries
    ))
}

// ---------------------------------------------------------------------------
// run_heartbeat_loop — periodic heartbeat with proof submission
// ---------------------------------------------------------------------------

pub async fn run_heartbeat_loop(
    identity: Arc<NodeIdentity>,
    proxy: Arc<BackendProxy>,
    config: RegistryConfig,
) {
    if !config.enabled {
        warn!("registry heartbeat disabled");
        return;
    }

    info!(
        interval_secs = config.heartbeat_secs,
        "starting registry heartbeat loop"
    );

    loop {
        let models = proxy
            .list_models()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>();

        let state = ProviderState {
            peer_id: identity.peer_id.clone(),
            multiaddrs: vec![],
            models,
            attestation_hash: String::new(), // filled by attestation flow
            gpu_memory_gb: 0,
            price_input_m: 0,
            price_output_m: 0,
            node_version: env!("CARGO_PKG_VERSION").to_string(),
            last_seen_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };

        match heartbeat(&identity, &state, &config).await {
            Ok(()) => {
                info!(
                    peer_id = %identity.peer_id,
                    models = ?state.models,
                    "heartbeat OK"
                );
            }
            Err(e) => {
                error!(
                    peer_id = %identity.peer_id,
                    error = %e,
                    "heartbeat failed; continuing in log-only mode"
                );
            }
        }

        sleep(Duration::from_secs(config.heartbeat_secs.max(1))).await;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive a deterministic 32-byte nodeId from the node's x25519 pubkey.
#[allow(dead_code)]
fn derive_node_id(identity: &NodeIdentity) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&identity.x25519_pubkey);
    hasher.finalize().into()
}

/// Parse a hex string into bytes32.
pub fn parse_bytes32(s: &str) -> [u8; 32] {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).expect("valid hex");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    arr
}

/// SHA-256 of input bytes, returned as lowercase hex string.
#[allow(dead_code)]
fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(data);
    hex::encode(hash)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_node_id_deterministic() {
        let identity = NodeIdentity {
            peer_id: "test-peer".to_string(),
            x25519_pubkey: [42u8; 32],
            ed25519_pubkey: [99u8; 32],
        };
        let id1 = derive_node_id(&identity);
        let id2 = derive_node_id(&identity);
        assert_eq!(id1, id2, "nodeId should be deterministic");
    }

    #[test]
    fn test_derive_node_id_different_for_different_keys() {
        let identity1 = NodeIdentity {
            peer_id: "peer-1".to_string(),
            x25519_pubkey: [42u8; 32],
            ed25519_pubkey: [99u8; 32],
        };
        let identity2 = NodeIdentity {
            peer_id: "peer-2".to_string(),
            x25519_pubkey: [43u8; 32],
            ed25519_pubkey: [99u8; 32],
        };
        let id1 = derive_node_id(&identity1);
        let id2 = derive_node_id(&identity2);
        assert_ne!(id1, id2, "different keys should produce different nodeIds");
    }

    #[test]
    fn test_lifecycle_display() {
        assert_eq!(format!("{}", Lifecycle::Active), "Active");
        assert_eq!(format!("{}", Lifecycle::Chilled), "Chilled");
        assert_eq!(format!("{}", Lifecycle::Defunct), "Defunct");
    }

    #[test]
    fn test_security_tier_display() {
        assert_eq!(format!("{}", SecurityTier::BestEffort), "BEST_EFFORT");
        assert_eq!(format!("{}", SecurityTier::TeeVerified), "TEE_VERIFIED");
    }

    #[test]
    fn test_sha256_hex_consistency() {
        let h1 = sha256_hex(b"hello");
        let h2 = sha256_hex(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_parse_bytes32_valid() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_bytes32(hex);
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_parse_bytes32_with_0x_prefix() {
        let hex = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_bytes32(hex);
        assert_eq!(result.len(), 32);
    }

    #[test]
    #[should_panic]
    fn test_parse_bytes32_invalid_hex_panics() {
        parse_bytes32("not-hex!");
    }

    #[test]
    #[should_panic]
    fn test_parse_bytes32_wrong_length_panics() {
        parse_bytes32("deadbeef");
    }
}

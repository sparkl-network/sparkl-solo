// Provider registration: Hub EVM `ProviderRegistry` when `evm-settlement` is enabled;
// log-only stubs otherwise. Operator key and RPC come from `SettlementConfig` (see `RegistryConfig` docs).
//
// See AGENTS.md and docs/MVP_ROADMAP.md.

use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "evm-settlement")]
use anyhow::Context;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};

#[cfg(feature = "evm-settlement")]
use alloy::network::EthereumWallet;
#[cfg(feature = "evm-settlement")]
use alloy::primitives::{Address, FixedBytes, B256};
#[cfg(feature = "evm-settlement")]
use alloy::providers::ProviderBuilder;
#[cfg(feature = "evm-settlement")]
use alloy::signers::local::PrivateKeySigner;

use crate::attestation::{refresh_nras_tee_report_hash, NrasRuntimeState};
use crate::config::{AttestationConfig, RegistryConfig, SettlementConfig};
use crate::identity::NodeIdentity;
use crate::proxy::BackendProxy;
use crate::session::SecurityTier;

#[cfg(feature = "evm-settlement")]
alloy::sol!(
    #[sol(rpc)]
    ProviderRegistry,
    concat!(env!("CARGO_MANIFEST_DIR"), "/abi/ProviderRegistry.json")
);

/// Mirrors on-chain node info exposed to JSON consumers.
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
    #[serde(default)]
    pub encryption_pubkey: String,
    #[serde(default)]
    pub encryption_key_version: u32,
    #[serde(default)]
    pub encryption_keys_last_version: u32,
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
    Ok(PrivateKeySigner::from_bytes(&arr.into())?)
}

#[cfg(feature = "evm-settlement")]
fn registry_rpc_url(registry: &RegistryConfig, settlement: &SettlementConfig) -> Result<reqwest::Url> {
    registry
        .effective_evm_rpc_url(settlement)
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid registry/settlement EVM RPC URL")
}

#[allow(unused_variables)]
pub async fn register(
    identity: &NodeIdentity,
    state: &ProviderState,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
) -> Result<InclusionProof> {
    if !registry.enabled {
        warn!(peer_id = %state.peer_id, "registry disabled; skipping register");
        return Ok(InclusionProof {
            token_id: "disabled".to_string(),
            proof: "disabled".to_string(),
        });
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = settlement.evm_provider_wallet_private_key.trim();
        if pk.is_empty() {
            return Err(anyhow!(
                "registry enabled but settlement.evm_provider_wallet_private_key is not configured"
            ));
        }

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = registry
            .registry_contract_address
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let rpc_url = registry_rpc_url(registry, settlement)?;
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer.clone()))
            .fetch_chain_id()
            .connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &provider);

        let node_id_b256 = B256::from(crate::identity::on_chain_node_id_from_identity(identity));

        let metadata_uri = format!(
            "ipfs://{}/provider/{}",
            sha256_hex(state.peer_id.as_bytes()),
            state.peer_id
        );

        let supports_tee = !state.attestation_hash.is_empty();

        info!(
            node_id = %hex::encode(node_id_b256.as_slice()),
            payout = %signer.address(),
            supports_best_effort = true,
            supports_tee = supports_tee,
            "registering node on ProviderRegistry"
        );

        let pending = instance
            .registerNode(
                node_id_b256,
                signer.address(),
                true,
                supports_tee,
                metadata_uri,
                B256::from_slice(&identity.x25519_pubkey),
            )
            .send()
            .await
            .map_err(|e| anyhow!("registerNode send failed: {e}"))?;

        let tx_hash = pending
            .with_required_confirmations(1)
            .watch()
            .await
            .map_err(|e| anyhow!("registerNode confirmation failed: {e}"))?;

        info!(?tx_hash, "node registered successfully");

        Ok(InclusionProof {
            token_id: format!("tok-{}", hex::encode(node_id_b256.as_slice())),
            proof: format!("{tx_hash:#x}"),
        })
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

#[allow(unused_variables)]
pub async fn heartbeat(
    identity: &NodeIdentity,
    state: &ProviderState,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
) -> Result<()> {
    if !registry.enabled {
        return Ok(());
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = settlement.evm_provider_wallet_private_key.trim();
        if pk.is_empty() {
            return Err(anyhow!(
                "heartbeat: settlement.evm_provider_wallet_private_key not configured"
            ));
        }

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = registry
            .registry_contract_address
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let rpc_url = registry_rpc_url(registry, settlement)?;
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer.clone()))
            .fetch_chain_id()
            .connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &provider);

        let node_id_b256 = B256::from(crate::identity::on_chain_node_id_from_identity(identity));

        if state.attestation_hash.is_empty() {
            info!(
                node_id = %hex::encode(node_id_b256.as_slice()),
                "heartbeat: no attestation hash; skipping setTEEProof"
            );
            return Ok(());
        }

        let tee_hash = FixedBytes::<32>::from(parse_bytes32_strict(&state.attestation_hash)?);

        info!(
            node_id = %hex::encode(node_id_b256.as_slice()),
            tee_report = %hex::encode(tee_hash.as_slice()),
            "submitting TEE proof via setTEEProof"
        );

        let pending = instance
            .setTEEProof(node_id_b256, tee_hash)
            .send()
            .await
            .map_err(|e| anyhow!("setTEEProof send failed: {e}"))?;

        let tx_hash = pending
            .with_required_confirmations(1)
            .watch()
            .await
            .map_err(|e| anyhow!("setTEEProof confirmation failed: {e}"))?;

        info!(?tx_hash, "TEE proof submitted successfully");

        Ok(())
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; heartbeat is stubbed");
        Ok(())
    }
}

#[allow(unused_variables)]
pub async fn deregister(
    identity: &NodeIdentity,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
) -> Result<()> {
    if !registry.enabled {
        warn!("registry disabled; skipping deregister");
        return Ok(());
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = settlement.evm_provider_wallet_private_key.trim();
        if pk.is_empty() {
            return Err(anyhow!(
                "deregister: settlement.evm_provider_wallet_private_key not configured"
            ));
        }

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = registry
            .registry_contract_address
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let rpc_url = registry_rpc_url(registry, settlement)?;
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer.clone()))
            .fetch_chain_id()
            .connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &provider);
        let node_id_b256 = B256::from(crate::identity::on_chain_node_id_from_identity(identity));

        info!(
            node_id = %hex::encode(node_id_b256.as_slice()),
            "chilling node on ProviderRegistry"
        );

        let pending = instance
            .chillNode(node_id_b256)
            .send()
            .await
            .map_err(|e| anyhow!("chillNode send failed: {e}"))?;

        let tx_hash = pending
            .with_required_confirmations(1)
            .watch()
            .await
            .map_err(|e| anyhow!("chillNode confirmation failed: {e}"))?;

        info!(?tx_hash, "node chilled successfully");

        Ok(())
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; deregister is stubbed");
        Ok(())
    }
}

/// Result of [`rotate_encryption_key_with_signer`] (on-chain rotate + optional local persist).
#[cfg(feature = "evm-settlement")]
#[derive(Debug, Clone)]
pub enum RotateEncryptionKeyOutcome {
    DryRun {
        calldata_hex: String,
        new_x25519_pubkey_hex: String,
        node_id_hex: String,
        registry_address: String,
        next_encryption_version: u32,
    },
    Submitted {
        next_encryption_version: u32,
        tx_hash: String,
    },
}

/// On-chain encryption key rotation with an explicit operator key hex string.
///
/// When `dry_run` is true, returns ABI calldata and does not broadcast or touch disk.
/// When `enforce_registry_enabled` is true (library / server path), `registry.enabled` must be set.
#[cfg(feature = "evm-settlement")]
#[allow(unused_variables)]
pub async fn rotate_encryption_key_with_signer(
    identity: &NodeIdentity,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
    grace_period_secs: u64,
    operator_pk_hex: &str,
    dry_run: bool,
    enforce_registry_enabled: bool,
) -> Result<RotateEncryptionKeyOutcome> {
    if enforce_registry_enabled && !registry.enabled {
        return Err(anyhow!("registry disabled; cannot rotate encryption key on-chain"));
    }

    let pk = operator_pk_hex.trim();
    if pk.is_empty() {
        return Err(anyhow!("operator private key is empty"));
    }

    let registry_addr_str = registry.registry_contract_address.trim();
    if registry_addr_str.is_empty() {
        return Err(anyhow!(
            "registry_contract_address is empty; set it in config or enable settlement resolution"
        ));
    }

    let (next_ver, new_secret, pk_bytes) = crate::identity::prepare_encryption_rotation()?;
    let new_pubkey = B256::from(pk_bytes);

    let registry_addr: Address = registry_addr_str
        .parse()
        .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

    let rpc_url = registry_rpc_url(registry, settlement)?;
    let node_id_b256 = B256::from(crate::identity::on_chain_node_id_from_identity(identity));
    let node_id_hex = format!("0x{}", hex::encode(node_id_b256.as_slice()));
    let new_x25519_pubkey_hex = format!("0x{}", hex::encode(pk_bytes));

    if dry_run {
        let read_provider = ProviderBuilder::new().connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &read_provider);
        let call = instance.rotateEncryptionKey(node_id_b256, new_pubkey, grace_period_secs);
        let calldata = call.calldata();
        let calldata_hex = format!("0x{}", hex::encode(calldata));
        return Ok(RotateEncryptionKeyOutcome::DryRun {
            calldata_hex,
            new_x25519_pubkey_hex,
            node_id_hex,
            registry_address: registry_addr_str.to_string(),
            next_encryption_version: next_ver,
        });
    }

    let signer = parse_evm_signer(pk)?;
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer.clone()))
        .fetch_chain_id()
        .connect_http(rpc_url);
    let instance = ProviderRegistry::new(registry_addr, &provider);

    info!(
        node_id = %hex::encode(node_id_b256.as_slice()),
        next_ver,
        grace_period_secs,
        "rotating encryption key on ProviderRegistry"
    );

    let pending = instance
        .rotateEncryptionKey(node_id_b256, new_pubkey, grace_period_secs)
        .send()
        .await
        .map_err(|e| anyhow!("rotateEncryptionKey send failed: {e}"))?;

    let tx_hash = pending
        .with_required_confirmations(1)
        .watch()
        .await
        .map_err(|e| anyhow!("rotateEncryptionKey confirmation failed: {e}"))?;

    crate::identity::persist_encryption_key_rotation(next_ver, new_secret)?;
    info!(next_ver, "encryption key rotation persisted locally");
    Ok(RotateEncryptionKeyOutcome::Submitted {
        next_encryption_version: next_ver,
        tx_hash: format!("{tx_hash:#x}"),
    })
}

/// On-chain encryption key rotation + local `identity-secret.json` update (requires `evm-settlement`).
#[allow(unused_variables)]
pub async fn rotate_encryption_key(
    identity: &NodeIdentity,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
    grace_period_secs: u64,
) -> Result<u32> {
    if !registry.enabled {
        return Err(anyhow!("registry disabled; cannot rotate encryption key on-chain"));
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = settlement.evm_provider_wallet_private_key.trim();
        if pk.is_empty() {
            return Err(anyhow!(
                "rotate_encryption_key requires settlement.evm_provider_wallet_private_key"
            ));
        }
        match rotate_encryption_key_with_signer(
            identity,
            registry,
            settlement,
            grace_period_secs,
            pk,
            false,
            true,
        )
        .await?
        {
            RotateEncryptionKeyOutcome::Submitted {
                next_encryption_version,
                ..
            } => Ok(next_encryption_version),
            RotateEncryptionKeyOutcome::DryRun { .. } => Err(anyhow!(
                "internal error: dry-run returned from rotate_encryption_key"
            )),
        }
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        Err(anyhow!(
            "evm-settlement feature not enabled; cannot rotate encryption key on-chain"
        ))
    }
}

/// Mark a chilled node as defunct — final step of the lifecycle.
///
/// On-chain flow: `Active → Chilled → Defunct`.
/// `markDefunct` requires the node to be in `Chilled` state and to have
/// zero open sessions in the escrow. Only the registered node operator
/// can call this.
#[allow(unused_variables)]
pub async fn defunct(
    identity: &NodeIdentity,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
) -> Result<()> {
    if !registry.enabled {
        warn!("registry disabled; skipping defunct");
        return Ok(());
    }

    #[cfg(feature = "evm-settlement")]
    {
        let pk = settlement.evm_provider_wallet_private_key.trim();
        if pk.is_empty() {
            return Err(anyhow!(
                "defunct: settlement.evm_provider_wallet_private_key not configured"
            ));
        }

        let signer = parse_evm_signer(pk)?;
        let registry_addr: Address = registry
            .registry_contract_address
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let rpc_url = registry_rpc_url(registry, settlement)?;
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer.clone()))
            .fetch_chain_id()
            .connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &provider);
        let node_id_b256 = B256::from(crate::identity::on_chain_node_id_from_identity(identity));

        info!(
            node_id = %hex::encode(node_id_b256.as_slice()),
            "marking node defunct on ProviderRegistry"
        );

        let pending = instance
            .markDefunct(node_id_b256)
            .send()
            .await
            .map_err(|e| anyhow!("markDefunct send failed: {e}"))?;

        let tx_hash = pending
            .with_required_confirmations(1)
            .watch()
            .await
            .map_err(|e| anyhow!("markDefunct confirmation failed: {e}"))?;

        info!(?tx_hash, "node marked defunct successfully");

        Ok(())
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; defunct is stubbed");
        Ok(())
    }
}

#[allow(unused_variables)]
pub async fn get_peer_info(
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
    node_id: [u8; 32],
) -> Result<Option<ProviderInfo>> {
    if !registry.enabled {
        warn!("registry disabled; skipping get_peer_info");
        return Ok(None);
    }

    #[cfg(feature = "evm-settlement")]
    {
        let registry_addr: Address = registry
            .registry_contract_address
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let rpc_url = registry_rpc_url(registry, settlement)?;
        let read_provider = ProviderBuilder::new().connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &read_provider);
        let node_id_b256 = B256::from(node_id);

        let result = instance
            .getProvider(node_id_b256)
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
            payout: format!("{:#x}", result.payout),
            fee_bps: result.feeBps,
            active: result.active,
            supports_best_effort: result.supportsBestEffort,
            supports_tee: result.supportsTEE,
            tee_report_hash: hex::encode(result.teeReportHash.as_slice()),
            metadata_uri: result.metadataURI.to_string(),
            lifecycle: format!("{lifecycle}"),
            encryption_pubkey: format!("{:#x}", result.encryptionPubkey),
            encryption_key_version: result.encryptionKeyVersion,
            encryption_keys_last_version: result.encryptionKeysLastVersion,
        }))
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        warn!("evm-settlement feature not enabled; get_peer_info is stubbed");
        Ok(None)
    }
}

#[allow(unused_variables)]
pub async fn supports_tier(
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
    node_id: [u8; 32],
    tier: SecurityTier,
) -> Result<bool> {
    if !registry.enabled {
        return Ok(false);
    }

    #[cfg(feature = "evm-settlement")]
    {
        let registry_addr: Address = registry
            .registry_contract_address
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid registry_contract_address: {e}"))?;

        let rpc_url = registry_rpc_url(registry, settlement)?;
        let read_provider = ProviderBuilder::new().connect_http(rpc_url);
        let instance = ProviderRegistry::new(registry_addr, &read_provider);
        let node_id_b256 = B256::from(node_id);
        let tier_u8 = match tier {
            SecurityTier::BestEffort => 0u8,
            SecurityTier::TeeVerified => 1u8,
        };

        let result = instance
            .supportsTier(node_id_b256, tier_u8)
            .call()
            .await
            .map_err(|e| anyhow!("supportsTier call failed: {e}"))?;

        Ok(result)
    }

    #[cfg(not(feature = "evm-settlement"))]
    {
        match tier {
            SecurityTier::TeeVerified => Ok(false),
            SecurityTier::BestEffort => Ok(true),
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Retries and base delay for [`startup_register_with_retry`] (wired from `main.rs`).
pub const STARTUP_REGISTER_MAX_RETRIES: u32 = 3;
pub const STARTUP_REGISTER_INITIAL_DELAY_SECS: u64 = 30;

/// Issue #3: attempt on-chain registration at startup, then run the heartbeat loop (same task as `main.rs`).
pub async fn run_registry_startup_and_heartbeat(
    identity: Arc<NodeIdentity>,
    proxy: Arc<BackendProxy>,
    registry_cfg: RegistryConfig,
    settlement_cfg: SettlementConfig,
    attestation_cfg: AttestationConfig,
    nras_state: Arc<RwLock<NrasRuntimeState>>,
) {
    if let Err(e) = startup_register_with_retry(
        identity.clone(),
        proxy.clone(),
        registry_cfg.clone(),
        settlement_cfg.clone(),
        attestation_cfg.clone(),
        nras_state.clone(),
        STARTUP_REGISTER_MAX_RETRIES,
        STARTUP_REGISTER_INITIAL_DELAY_SECS,
    )
    .await
    {
        tracing::error!(
            error = %e,
            "startup registration failed; starting heartbeat anyway"
        );
    }
    run_heartbeat_loop(
        identity,
        proxy,
        registry_cfg,
        settlement_cfg,
        attestation_cfg,
        nras_state,
    )
    .await;
}

pub async fn startup_register_with_retry(
    identity: Arc<NodeIdentity>,
    proxy: Arc<BackendProxy>,
    registry: RegistryConfig,
    settlement: SettlementConfig,
    attestation: AttestationConfig,
    nras_state: Arc<RwLock<NrasRuntimeState>>,
    max_retries: u32,
    initial_delay_secs: u64,
) -> Result<InclusionProof> {
    let mut delay = Duration::from_secs(initial_delay_secs.max(1));

    for attempt in 1..=max_retries {
        info!(attempt, max_retries, "starting registration attempt");

        let models = proxy
            .list_models()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>();

        let attestation_hash =
            refresh_nras_tee_report_hash(&attestation, Some(identity.peer_id.clone()), nras_state.clone())
                .await;

        let state = ProviderState {
            peer_id: identity.peer_id.clone(),
            multiaddrs: vec![],
            models,
            attestation_hash,
            gpu_memory_gb: 0,
            price_input_m: 0,
            price_output_m: 0,
            node_version: env!("CARGO_PKG_VERSION").to_string(),
            last_seen_ms: 0,
        };

        match register(&identity, &state, &registry, &settlement).await {
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
                    delay *= 2;
                }
            }
        }
    }

    Err(anyhow!(
        "registration failed after {} retries",
        max_retries
    ))
}

pub async fn run_heartbeat_loop(
    identity: Arc<NodeIdentity>,
    proxy: Arc<BackendProxy>,
    registry_cfg: RegistryConfig,
    settlement_cfg: SettlementConfig,
    attestation_cfg: AttestationConfig,
    nras_state: Arc<RwLock<NrasRuntimeState>>,
) {
    if !registry_cfg.enabled {
        warn!("registry heartbeat disabled");
        return;
    }

    info!(
        interval_secs = registry_cfg.heartbeat_secs,
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

        let attestation_hash =
            refresh_nras_tee_report_hash(&attestation_cfg, Some(identity.peer_id.clone()), nras_state.clone())
                .await;

        let state = ProviderState {
            peer_id: identity.peer_id.clone(),
            multiaddrs: vec![],
            models,
            attestation_hash,
            gpu_memory_gb: 0,
            price_input_m: 0,
            price_output_m: 0,
            node_version: env!("CARGO_PKG_VERSION").to_string(),
            last_seen_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };

        match heartbeat(&identity, &state, &registry_cfg, &settlement_cfg).await {
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
                    "heartbeat failed; continuing"
                );
            }
        }

        sleep(Duration::from_secs(registry_cfg.heartbeat_secs.max(1))).await;
    }
}

fn parse_bytes32_strict(s: &str) -> Result<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).map_err(|e| anyhow!("invalid tee hash hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(anyhow!("tee hash must be 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// SHA-256 of input bytes, returned as lowercase hex string.
#[cfg(any(test, feature = "evm-settlement"))]
fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(data);
    hex::encode(hash)
}

/// Parse a hex string into `bytes32` (test / tooling).
pub fn parse_bytes32(s: &str) -> [u8; 32] {
    parse_bytes32_strict(s).expect("valid 32-byte hex")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::attestation::NrasRuntimeState;
    use crate::config::{AttestationConfig, BackendConfig, RegistryConfig, SettlementConfig};
    use crate::identity::{on_chain_node_id_bytes, on_chain_node_id_from_identity, NodeIdentity};
    use crate::proxy::BackendProxy;
    use tokio::sync::RwLock;

    fn test_identity() -> Arc<NodeIdentity> {
        Arc::new(NodeIdentity {
            peer_id: "12D3KooWTestPeerIdForRegistryStartup".to_string(),
            x25519_pubkey: [1u8; 32],
            ed25519_pubkey: [2u8; 32],
        })
    }

    fn test_proxy() -> Arc<BackendProxy> {
        Arc::new(
            BackendProxy::new(&BackendConfig {
                url: "http://127.0.0.1:9".to_string(),
                health_path: "/health".to_string(),
                models_path: "/v1/models".to_string(),
                timeout_secs: 1,
            })
            .expect("backend proxy"),
        )
    }

    fn test_registry_config(enabled: bool) -> RegistryConfig {
        RegistryConfig {
            registry_contract_address: "0x0000000000000000000000000000000000000001".to_string(),
            evm_rpc_url: String::new(),
            heartbeat_secs: 60,
            enabled,
        }
    }

    fn test_settlement_config() -> SettlementConfig {
        SettlementConfig {
            epoch_secs: 300,
            evm_rpc_url: "http://127.0.0.1:8545".to_string(),
            escrow_contract: "0x0000000000000000000000000000000000000002".to_string(),
            sparkl_network_config_address: String::new(),
            enabled: false,
            evm_provider_wallet_private_key: String::new(),
            evm_settlement_operator_wallet_private_key: String::new(),
            usage_internal_units_per_micro_usd: 1,
            tee_tick_secs: 30,
            tee_settle_tokens_threshold: 1,
            usage_tolerance_bps: 100,
            tee_settle_every_n_blocks: 0,
            session_min_deposit: 1_000_000_000_000_000_000,
        }
    }

    fn test_attestation_config() -> AttestationConfig {
        AttestationConfig {
            nras_url: String::new(),
            nras_enabled: false,
            cert_ttl_days: 7,
            nras_quote_hex: String::new(),
            nras_signature_hex: String::new(),
        }
    }

    fn test_nras_state() -> Arc<RwLock<NrasRuntimeState>> {
        Arc::new(RwLock::new(NrasRuntimeState::default()))
    }

    #[tokio::test]
    async fn startup_register_succeeds_when_registry_disabled() {
        let proof = startup_register_with_retry(
            test_identity(),
            test_proxy(),
            test_registry_config(false),
            test_settlement_config(),
            test_attestation_config(),
            test_nras_state(),
            1,
            0,
        )
        .await
        .expect("register");
        assert_eq!(proof.token_id, "disabled");
        assert_eq!(proof.proof, "disabled");
    }

    #[cfg(not(feature = "evm-settlement"))]
    #[tokio::test]
    async fn startup_register_succeeds_stub_when_registry_enabled() {
        let proof = startup_register_with_retry(
            test_identity(),
            test_proxy(),
            test_registry_config(true),
            test_settlement_config(),
            test_attestation_config(),
            test_nras_state(),
            1,
            0,
        )
        .await
        .expect("stub register");
        assert_eq!(proof.proof, "stub-proof");
    }

    #[cfg(feature = "evm-settlement")]
    #[tokio::test]
    async fn startup_register_fails_without_provider_wallet_key() {
        let err = startup_register_with_retry(
            test_identity(),
            test_proxy(),
            test_registry_config(true),
            test_settlement_config(),
            test_attestation_config(),
            test_nras_state(),
            2,
            0,
        )
        .await
        .expect_err("missing key");
        assert!(
            err.to_string().contains("registration failed after 2 retries"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn run_registry_startup_and_heartbeat_returns_when_registry_disabled() {
        let started = std::time::Instant::now();
        run_registry_startup_and_heartbeat(
            test_identity(),
            test_proxy(),
            test_registry_config(false),
            test_settlement_config(),
            test_attestation_config(),
            test_nras_state(),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "expected immediate return when registry disabled in heartbeat"
        );
    }

    #[tokio::test]
    async fn run_registry_startup_and_heartbeat_enters_heartbeat_when_registry_enabled() {
        let result = tokio::time::timeout(
            Duration::from_millis(800),
            run_registry_startup_and_heartbeat(
                test_identity(),
                test_proxy(),
                test_registry_config(true),
                test_settlement_config(),
                test_attestation_config(),
                test_nras_state(),
            ),
        )
        .await;
        assert!(
            result.is_err(),
            "heartbeat should loop when registry.enabled is true"
        );
    }

    #[test]
    fn startup_register_constants_match_roadmap() {
        assert_eq!(STARTUP_REGISTER_MAX_RETRIES, 3);
        assert_eq!(STARTUP_REGISTER_INITIAL_DELAY_SECS, 30);
    }

    #[test]
    fn test_on_chain_node_id_deterministic() {
        let identity = NodeIdentity {
            peer_id: "test-peer".to_string(),
            x25519_pubkey: [42u8; 32],
            ed25519_pubkey: [99u8; 32],
        };
        let id1 = on_chain_node_id_from_identity(&identity);
        let id2 = on_chain_node_id_from_identity(&identity);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_on_chain_node_id_different_for_different_keys() {
        let identity1 = NodeIdentity {
            peer_id: "peer-1".to_string(),
            x25519_pubkey: [42u8; 32],
            ed25519_pubkey: [99u8; 32],
        };
        let identity2 = NodeIdentity {
            peer_id: "peer-2".to_string(),
            x25519_pubkey: [43u8; 32],
            ed25519_pubkey: [100u8; 32],
        };
        let id1 = on_chain_node_id_from_identity(&identity1);
        let id2 = on_chain_node_id_from_identity(&identity2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_matches_identity_module_rule() {
        let pk = [7u8; 32];
        assert_eq!(on_chain_node_id_bytes(&pk), on_chain_node_id_from_identity(&NodeIdentity {
            peer_id: "x".into(),
            x25519_pubkey: [0u8; 32],
            ed25519_pubkey: pk,
        }));
    }

    #[test]
    fn test_lifecycle_display() {
        assert_eq!(format!("{}", Lifecycle::Active), "Active");
        assert_eq!(format!("{}", Lifecycle::Chilled), "Chilled");
        assert_eq!(format!("{}", Lifecycle::Defunct), "Defunct");
    }

    #[test]
    fn test_sha256_hex_consistency() {
        let h1 = sha256_hex(b"hello");
        let h2 = sha256_hex(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn test_parse_bytes32_valid() {
        let hex =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_bytes32(hex);
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_parse_bytes32_with_0x_prefix() {
        let hex =
            "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
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

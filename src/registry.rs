// STUB: This module is intentionally incomplete.
// Safe to extend. Do not call from production paths without a feature flag.
// See AGENTS.md for the full stub list.
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::config::{RegistryConfig, SettlementConfig};
use crate::identity::NodeIdentity;
use crate::proxy::BackendProxy;

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

pub async fn register(
    _identity: &NodeIdentity,
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
    info!("registry enabled but remote registration is stubbed in prototype");
    Ok(InclusionProof {
        token_id: format!("tok-{}", state.peer_id),
        proof: "stub-proof".to_string(),
    })
}

pub async fn run_heartbeat_loop(
    identity: Arc<NodeIdentity>,
    proxy: Arc<BackendProxy>,
    registry_cfg: RegistryConfig,
    settlement_cfg: SettlementConfig,
) {
    if !registry_cfg.enabled {
        warn!("registry heartbeat disabled");
        return;
    }

    let rpc_url = registry_cfg.effective_evm_rpc_url(&settlement_cfg);
    info!(
        peer_id = %identity.peer_id,
        provider_registry = %registry_cfg.registry_contract_address,
        evm_rpc_effective = %rpc_url,
        operator_evm_key_configured = %!settlement_cfg.evm_provider_wallet_private_key.trim().is_empty(),
        "registry heartbeat loop (stub — no on-chain calls yet)"
    );

    loop {
        let models = proxy
            .list_models()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>();
        info!(peer_id = %identity.peer_id, ?models, "registry heartbeat tick");
        sleep(Duration::from_secs(registry_cfg.heartbeat_secs.max(1))).await;
    }
}

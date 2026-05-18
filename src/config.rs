use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::session::SecurityTier;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub node: NodeConfig,
    pub network: NetworkConfig,
    pub backend: BackendConfig,
    pub attestation: AttestationConfig,
    pub registry: RegistryConfig,
    pub settlement: SettlementConfig,
    pub pricing: PricingConfig,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum NodeMode {
    Solo,
    Farm,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NodeConfig {
    pub name: String,
    pub data_dir: PathBuf,
    pub log_level: String,
    pub mode: NodeMode,
    #[serde(default = "default_receipt_cadence_tokens")]
    pub receipt_cadence_tokens: u64,
    #[serde(default)]
    pub include_models: Vec<String>,
    #[serde(default)]
    pub exclude_models: Vec<String>,
    /// Security tier recorded on new inference sessions (`best_effort` | `tee_verified`).
    #[serde(default = "default_session_security_tier")]
    pub session_security_tier: SecurityTier,
}

impl NodeConfig {
    pub fn is_model_allowed(&self, model_id: &str) -> bool {
        let included = self.include_models.is_empty()
            || self
                .include_models
                .iter()
                .any(|allowed| allowed == model_id);
        included
            && !self
                .exclude_models
                .iter()
                .any(|blocked| blocked == model_id)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct NetworkConfig {
    pub listen_addrs: Vec<String>,
    pub inference_port: u16,
    pub external_ip: Option<String>,
    pub bootstrap_peers: Vec<String>,
    #[serde(default)]
    pub public_addr: Vec<String>,
    #[serde(default = "default_expose_status_detail")]
    pub expose_status_detail: bool,
    #[serde(default = "default_allow_non_globals_in_dht")]
    pub allow_non_globals_in_dht: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BackendConfig {
    pub url: String,
    pub health_path: String,
    pub models_path: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AttestationConfig {
    pub nras_url: String,
    pub nras_enabled: bool,
    pub cert_ttl_days: u64,
    /// Hex-encoded raw TEE quote (**`full_attestation_flow` `quote`** string). Env: **`SPARKLE_ATTESTATION__NRAS_QUOTE_HEX`**.
    #[serde(default)]
    pub nras_quote_hex: String,
    /// Hex-encoded **`full_attestation_flow` `signature`** string. Env: **`SPARKLE_ATTESTATION__NRAS_SIGNATURE_HEX`**.
    #[serde(default)]
    pub nras_signature_hex: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegistryConfig {
    /// Hub EVM **`ProviderRegistry`** contract address (`0x` + 40 hex). When `evm-settlement` is enabled
    /// and `SparklNetworkConfig` bootstrap resolves, startup may overwrite this from the network config.
    #[serde(default = "default_registry_contract_address")]
    pub registry_contract_address: String,
    /// Optional **JSON-RPC HTTP(S) URL** for registry calls. When empty, **`settlement.evm_rpc_url`**
    /// is used (same Hub endpoint as escrow). Operator-signed txs use **`settlement.evm_provider_wallet_private_key`** only.
    #[serde(default)]
    pub evm_rpc_url: String,
    pub heartbeat_secs: u64,
    pub enabled: bool,
}

impl RegistryConfig {
    /// RPC URL for Hub EVM registry interactions: `registry.evm_rpc_url` if set, else settlement’s URL.
    pub fn effective_evm_rpc_url<'a>(&'a self, settlement: &'a SettlementConfig) -> &'a str {
        let u = self.evm_rpc_url.trim();
        if !u.is_empty() {
            u
        } else {
            settlement.evm_rpc_url.trim()
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SettlementConfig {
    pub epoch_secs: u64,
    pub evm_rpc_url: String,
    pub escrow_contract: String,
    /// Optional `SparklNetworkConfig` bootstrap address (`0x` + 40 hex). When non-empty and not
    /// the zero address, overrides the compile-time `SPARKL_NETWORK_CONFIG_ADDRESS` in `network_config.rs`
    /// (for local/dev). Empty: use the baked-in constant. Env: `SPARKLE_SETTLEMENT__SPARKL_NETWORK_CONFIG_ADDRESS`.
    #[serde(default)]
    pub sparkl_network_config_address: String,
    pub enabled: bool,
    /// Hex-encoded secp256k1 key (`0x` optional). Must match `ProviderRegistry.nodeOperator(nodeId)` for each
    /// session’s `nodeId` when sending `recordUsage` / provider-side escrow calls.
    #[serde(default)]
    pub evm_provider_wallet_private_key: String,
    /// Hex-encoded secp256k1 key (`0x` optional). Must match `SettlementEscrow.settlementOperator` for operator settles.
    #[serde(default, alias = "evm_user_wallet_private_key")]
    pub evm_settlement_operator_wallet_private_key: String,
    /// Maps off-chain micro-USD (`Session::amount_micro_usd`) into escrow internal DOT units (18 decimals).
    #[serde(default = "default_usage_internal_units_per_micro_usd")]
    pub usage_internal_units_per_micro_usd: u128,
    /// Wall-clock cadence for `TEE_VERIFIED` streaming settlement touches (seconds).
    #[serde(default = "default_settlement_tee_tick_secs")]
    pub tee_tick_secs: u64,
    /// Minimum `tokens_output` delta since last TEE anchor before a streaming partial settle attempt.
    #[serde(default = "default_tee_settle_tokens_threshold")]
    pub tee_settle_tokens_threshold: u64,
    /// Allowed positive deviation of on-chain `usageRecorded` vs local bill for TEE streams (basis points).
    #[serde(default = "default_usage_tolerance_bps")]
    pub usage_tolerance_bps: u16,
    /// When non-zero, further throttle TEE touches to at least this many new RPC head blocks since last eligible settle.
    #[serde(default)]
    pub tee_settle_every_n_blocks: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PricingConfig {
    pub micro_usd_per_m_input_tokens: u64,
    pub micro_usd_per_m_output_tokens: u64,
}

pub fn load(config_path: Option<&Path>) -> Result<Config> {
    let path = config_path
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| PathBuf::from("config/default.toml"));

    let mut builder = config::Config::builder()
        .add_source(config::File::from(path.clone()).required(false))
        .add_source(config::Environment::with_prefix("SPARKLE").separator("__"));

    if !path.exists() {
        builder = builder
            .add_source(config::File::from(PathBuf::from("config/default.toml")).required(true));
    }

    let mut cfg: Config = builder
        .build()
        .context("failed to build config")?
        .try_deserialize()
        .context("failed to deserialize config")?;

    cfg.node.data_dir = expand_home(cfg.node.data_dir);
    Ok(cfg)
}

fn expand_home(path: PathBuf) -> PathBuf {
    let display = path.to_string_lossy();
    if let Some(suffix) = display.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(suffix);
        }
    }
    path
}

fn default_receipt_cadence_tokens() -> u64 {
    50
}

fn default_allow_non_globals_in_dht() -> bool {
    false
}

fn default_expose_status_detail() -> bool {
    false
}

fn default_registry_contract_address() -> String {
    "0x0000000000000000000000000000000000000000".to_string()
}

fn default_usage_internal_units_per_micro_usd() -> u128 {
    1_000_000_000_000
}

fn default_session_security_tier() -> SecurityTier {
    SecurityTier::BestEffort
}

fn default_settlement_tee_tick_secs() -> u64 {
    60
}

fn default_tee_settle_tokens_threshold() -> u64 {
    256
}

fn default_usage_tolerance_bps() -> u16 {
    100
}

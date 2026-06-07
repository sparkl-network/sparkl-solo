use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::models::validate_features;

use crate::session::SecurityTier;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub node: NodeConfig,
    pub network: NetworkConfig,
    pub backend: BackendConfig,
    pub attestation: AttestationConfig,
    pub registry: RegistryConfig,
    pub settlement: SettlementConfig,
    #[serde(default)]
    pub router: RouterConfig,
    #[serde(default)]
    pub capacity: CapacityConfig,
    /// Published model catalog (`[[models]]`). When non-empty, only listed models that exist on the backend are advertised.
    #[serde(default)]
    pub models: Vec<ModelEntryConfig>,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct CapacityConfig {
    #[serde(default = "default_queue_depth_ratio")]
    pub queue_depth_ratio: f64,
    #[serde(default = "default_queue_wait_timeout_secs")]
    pub queue_wait_timeout_secs: u64,
}

fn default_queue_depth_ratio() -> f64 {
    1.0
}

fn default_queue_wait_timeout_secs() -> u64 {
    60
}

/// Operator-defined model offering (`[[models]]` in config).
#[derive(Debug, Deserialize, Clone)]
pub struct ModelEntryConfig {
    pub id: String,
    #[serde(default)]
    pub quantization: String,
    #[serde(default)]
    pub parameter_count: String,
    #[serde(default)]
    pub context_size: u32,
    #[serde(default)]
    pub concurrency: u32,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub features: HashMap<String, String>,
}

impl ModelEntryConfig {
    pub fn validate(&self) -> Result<()> {
        let id = self.id.trim();
        if id.is_empty() {
            anyhow::bail!("[[models]] entry requires non-empty id");
        }
        validate_features(&self.features, id)
    }
}

/// Outbound WebSocket tunnel to sparkl-router (`/node/connect`).
#[derive(Debug, Default, Deserialize, Clone)]
pub struct RouterConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Full `ws://` or `wss://` URL (path `/node/connect` appended if missing).
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_router_reconnect_min_secs")]
    pub reconnect_min_secs: u64,
    #[serde(default = "default_router_reconnect_max_secs")]
    pub reconnect_max_secs: u64,
    /// Empty → `http://127.0.0.1:{network.inference_port}`.
    #[serde(default)]
    pub local_inference_base: String,
}

impl RouterConfig {
    pub fn effective_local_inference_base(&self, inference_port: u16) -> String {
        let t = self.local_inference_base.trim();
        if !t.is_empty() {
            return t.trim_end_matches('/').to_string();
        }
        format!("http://127.0.0.1:{inference_port}")
    }
}

fn default_router_reconnect_min_secs() -> u64 {
    1
}

fn default_router_reconnect_max_secs() -> u64 {
    60
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum NodeMode {
    Solo,
    Farm,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NodeConfig {
    /// Operator-facing label for logs, portal directory, and router tunnel status (max 128 chars).
    #[serde(alias = "name")]
    pub moniker: String,
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
    /// Trimmed moniker, or `None` when unset/blank.
    pub fn display_moniker(&self) -> Option<&str> {
        let m = self.moniker.trim();
        if m.is_empty() {
            None
        } else {
            Some(m)
        }
    }

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
    /// Wall-clock cadence for `TEE_VERIFIED` streaming settlement touches (seconds).
    #[serde(default = "default_settlement_tee_tick_secs")]
    pub tee_tick_secs: u64,
    /// Minimum `tokens_output` delta since last TEE anchor before a streaming partial settle attempt.
    #[serde(default = "default_tee_settle_tokens_threshold")]
    pub tee_settle_tokens_threshold: u64,
    /// When non-zero, further throttle TEE touches to at least this many new RPC head blocks since last eligible settle.
    #[serde(default)]
    pub tee_settle_every_n_blocks: u64,
    /// Minimum internal deposit (in escrow internal units) when opening a session on-chain.
    /// Must be > 0 to satisfy `openSession`'s `BadAmount` revert. Default: 1e18 (1 unit at 18 decimals).
    #[serde(default = "default_session_min_deposit")]
    pub session_min_deposit: u64,
    /// When true and `[router].enabled`, the node does not submit `recordUsage` (router meters instead).
    #[serde(default = "default_true")]
    pub router_usage_metering: bool,
}

fn default_true() -> bool {
    true
}

fn default_session_min_deposit() -> u64 {
    1_000_000_000_000_000_000u64 // 1e18
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
    cfg.node.moniker = crate::metadata_uri::normalize_moniker(&cfg.node.moniker)?;
    for entry in &cfg.models {
        entry.validate().context("invalid [[models]] entry")?;
    }
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

fn default_session_security_tier() -> SecurityTier {
    SecurityTier::BestEffort
}

fn default_settlement_tee_tick_secs() -> u64 {
    60
}

fn default_tee_settle_tokens_threshold() -> u64 {
    256
}

#[cfg(test)]
mod tests {
    fn minimal_toml(moniker_line: &str) -> String {
        format!(
            r#"
[node]
{moniker_line}
data_dir = "./data"
log_level = "info"
mode = "solo"

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/30333"]
inference_port = 9944
bootstrap_peers = []

[backend]
url = "http://127.0.0.1:1"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 1

[attestation]
nras_url = "http://127.0.0.1"
nras_enabled = false
cert_ttl_days = 1

[registry]
heartbeat_secs = 60
enabled = false

[settlement]
epoch_secs = 60
evm_rpc_url = "http://127.0.0.1"
escrow_contract = "0x0000000000000000000000000000000000000000"
enabled = false
"#,
            moniker_line = moniker_line
        )
    }

    fn write_temp_config(toml: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sparkl-config-test-{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, toml).unwrap();
        path
    }

    #[test]
    fn moniker_name_alias_deserializes() {
        let path = write_temp_config(&minimal_toml(r#"name = "legacy-name""#));
        let cfg = super::load(Some(&path)).unwrap();
        assert_eq!(cfg.node.moniker, "legacy-name");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_rejects_moniker_over_128() {
        let long = "x".repeat(129);
        let path = write_temp_config(&minimal_toml(&format!(r#"moniker = "{long}""#)));
        let err = super::load(Some(&path)).unwrap_err();
        assert!(err.to_string().contains("128"));
        let _ = std::fs::remove_file(path);
    }
}


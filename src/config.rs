use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

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
}

#[derive(Debug, Deserialize, Clone)]
pub struct NetworkConfig {
    pub listen_addrs: Vec<String>,
    pub inference_port: u16,
    pub external_ip: Option<String>,
    pub bootstrap_peers: Vec<String>,
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegistryConfig {
    pub unicity_aggregator_url: String,
    pub heartbeat_secs: u64,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SettlementConfig {
    pub epoch_secs: u64,
    pub evm_rpc_url: String,
    pub escrow_contract: String,
    pub enabled: bool,
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

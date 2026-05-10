use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Utc;
use sparkl_solo::config;
use sparkl_solo::identity;
use sparkl_solo::network;
use sparkl_solo::proxy::BackendProxy;
use sparkl_solo::registry;
use sparkl_solo::server::{self, AppState};
use sparkl_solo::session::SessionManager;
use sparkl_solo::settlement;
use sparkl_solo::store::Store;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_cli_args(std::env::args().skip(1))?;
    let mut cfg = config::load(cli.config_path.as_deref())?;
    apply_cli_overrides(&mut cfg, &cli)?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(cfg.node.log_level.clone()))
        .init();

    let store = Arc::new(Store::open(&cfg.node.data_dir)?);
    let pruned_sessions = store.prune_old_sessions(Duration::from_secs(60 * 60 * 24 * 30))?;
    if pruned_sessions > 0 {
        info!(pruned_sessions, "pruned completed sessions from local store");
    }
    let identity = identity::load_or_generate(&cfg).await?;
    let (_swarm_handle, swarm_cmd) =
        network::start_swarm(&identity, &cfg.network, &cfg.node.data_dir).await?;

    let proxy = Arc::new(BackendProxy::new(&cfg.backend)?);
    if let Err(err) = proxy.check_health().await {
        info!(%err, "backend health check failed on startup; continuing prototype mode");
    }

    let sessions = Arc::new(SessionManager::new(store.clone()));
    sessions.recover_from_store()?;

    if cfg.registry.enabled {
        let identity_arc = Arc::new(identity.clone());
        let proxy_arc = proxy.clone();
        let registry_cfg = cfg.registry.clone();
        tokio::spawn(async move {
            registry::run_heartbeat_loop(identity_arc, proxy_arc, registry_cfg).await;
        });
    }

    if cfg.settlement.enabled {
        let session_arc = sessions.clone();
        let store_arc = store.clone();
        let identity_arc = Arc::new(identity.clone());
        let settlement_cfg = cfg.settlement.clone();
        tokio::spawn(async move {
            settlement::run_epoch_loop(session_arc, store_arc, identity_arc, settlement_cfg).await;
        });
    }

    let state = AppState {
        config: cfg.clone(),
        identity: identity.clone(),
        proxy,
        sessions,
        swarm_cmd: Some(swarm_cmd),
        started_at: Utc::now(),
    };

    let app = server::router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.network.inference_port));
    let listener = TcpListener::bind(addr).await?;
    info!("sparkl-solo ready on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

struct CliArgs {
    config_path: Option<PathBuf>,
    node_name: Option<String>,
    data_dir: Option<PathBuf>,
    log_level: Option<String>,
    mode: Option<String>,
    receipt_cadence_tokens: Option<u64>,
    include_models: Vec<String>,
    exclude_models: Vec<String>,
    listen_addrs: Vec<String>,
    inference_port: Option<u16>,
    external_ip: Option<String>,
    bootstrap_peers: Vec<String>,
    public_addr: Vec<String>,
    allow_non_globals_in_dht: Option<bool>,
    backend_url: Option<String>,
    backend_health_path: Option<String>,
    backend_models_path: Option<String>,
    backend_timeout_secs: Option<u64>,
    nras_url: Option<String>,
    nras_enabled: Option<bool>,
    cert_ttl_days: Option<u64>,
    registry_url: Option<String>,
    registry_heartbeat_secs: Option<u64>,
    registry_enabled: Option<bool>,
    settlement_epoch_secs: Option<u64>,
    evm_rpc_url: Option<String>,
    escrow_contract: Option<String>,
    settlement_enabled: Option<bool>,
    price_input_micro_usd_per_m: Option<u64>,
    price_output_micro_usd_per_m: Option<u64>,
}

fn parse_cli_args<I>(mut args: I) -> Result<CliArgs>
where
    I: Iterator<Item = String>,
{
    let mut out = CliArgs {
        config_path: None,
        node_name: None,
        data_dir: None,
        log_level: None,
        mode: None,
        receipt_cadence_tokens: None,
        include_models: Vec::new(),
        exclude_models: Vec::new(),
        listen_addrs: Vec::new(),
        inference_port: None,
        external_ip: None,
        bootstrap_peers: Vec::new(),
        public_addr: Vec::new(),
        allow_non_globals_in_dht: None,
        backend_url: None,
        backend_health_path: None,
        backend_models_path: None,
        backend_timeout_secs: None,
        nras_url: None,
        nras_enabled: None,
        cert_ttl_days: None,
        registry_url: None,
        registry_heartbeat_secs: None,
        registry_enabled: None,
        settlement_epoch_secs: None,
        evm_rpc_url: None,
        escrow_contract: None,
        settlement_enabled: None,
        price_input_micro_usd_per_m: None,
        price_output_micro_usd_per_m: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                if let Some(path) = args.next() {
                    out.config_path = Some(PathBuf::from(path));
                } else {
                    return Err(anyhow!("--config requires a path value"));
                }
            }
            "--receipt-cadence" => {
                let Some(value) = args.next() else {
                    return Err(anyhow!("--receipt-cadence requires a numeric value"));
                };
                let parsed = value
                    .parse::<u64>()
                    .map_err(|_| anyhow!("invalid --receipt-cadence value: `{value}`"))?;
                out.receipt_cadence_tokens = Some(parsed.max(1));
            }
            "--name" => out.node_name = Some(required_value(&mut args, "--name")?),
            "--data-dir" => out.data_dir = Some(PathBuf::from(required_value(&mut args, "--data-dir")?)),
            "--log-level" => out.log_level = Some(required_value(&mut args, "--log-level")?),
            "--mode" => out.mode = Some(required_value(&mut args, "--mode")?),
            "--include-models" => {
                let value = required_value(&mut args, "--include-models")?;
                out.include_models.extend(parse_model_list(&value));
            }
            "--exclude-models" => {
                let value = required_value(&mut args, "--exclude-models")?;
                out.exclude_models.extend(parse_model_list(&value));
            }
            "--listen-addrs" => {
                let value = required_value(&mut args, "--listen-addrs")?;
                out.listen_addrs.extend(parse_model_list(&value));
            }
            "--inference-port" => out.inference_port = Some(parse_u16_flag(&mut args, "--inference-port")?),
            "--external-ip" => out.external_ip = Some(required_value(&mut args, "--external-ip")?),
            "--bootstrap-peers" => {
                let value = required_value(&mut args, "--bootstrap-peers")?;
                out.bootstrap_peers.extend(parse_model_list(&value));
            }
            "--public-addr" => {
                let value = required_value(&mut args, "--public-addr")?;
                out.public_addr.extend(parse_model_list(&value));
            }
            "--allow-non-globals-in-dht" => {
                out.allow_non_globals_in_dht = Some(parse_bool_flag(&mut args, "--allow-non-globals-in-dht")?);
            }
            "--backend-url" => out.backend_url = Some(required_value(&mut args, "--backend-url")?),
            "--backend-health-path" => out.backend_health_path = Some(required_value(&mut args, "--backend-health-path")?),
            "--backend-models-path" => out.backend_models_path = Some(required_value(&mut args, "--backend-models-path")?),
            "--backend-timeout-secs" => out.backend_timeout_secs = Some(parse_u64_flag(&mut args, "--backend-timeout-secs")?),
            "--nras-url" => out.nras_url = Some(required_value(&mut args, "--nras-url")?),
            "--nras-enabled" => out.nras_enabled = Some(parse_bool_flag(&mut args, "--nras-enabled")?),
            "--cert-ttl-days" => out.cert_ttl_days = Some(parse_u64_flag(&mut args, "--cert-ttl-days")?),
            "--registry-url" => out.registry_url = Some(required_value(&mut args, "--registry-url")?),
            "--registry-heartbeat-secs" => out.registry_heartbeat_secs = Some(parse_u64_flag(&mut args, "--registry-heartbeat-secs")?),
            "--registry-enabled" => out.registry_enabled = Some(parse_bool_flag(&mut args, "--registry-enabled")?),
            "--settlement-epoch-secs" => out.settlement_epoch_secs = Some(parse_u64_flag(&mut args, "--settlement-epoch-secs")?),
            "--evm-rpc-url" => out.evm_rpc_url = Some(required_value(&mut args, "--evm-rpc-url")?),
            "--escrow-contract" => out.escrow_contract = Some(required_value(&mut args, "--escrow-contract")?),
            "--settlement-enabled" => out.settlement_enabled = Some(parse_bool_flag(&mut args, "--settlement-enabled")?),
            "--price-input-micro-usd-per-m" => {
                out.price_input_micro_usd_per_m = Some(parse_u64_flag(&mut args, "--price-input-micro-usd-per-m")?);
            }
            "--price-output-micro-usd-per-m" => {
                out.price_output_micro_usd_per_m =
                    Some(parse_u64_flag(&mut args, "--price-output-micro-usd-per-m")?);
            }
            _ => {}
        }
    }
    Ok(out)
}

fn parse_model_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn required_value<I>(args: &mut I, flag: &str) -> Result<String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| anyhow!("{flag} requires a value"))
}

fn parse_u64_flag<I>(args: &mut I, flag: &str) -> Result<u64>
where
    I: Iterator<Item = String>,
{
    let value = required_value(args, flag)?;
    value
        .parse::<u64>()
        .map_err(|_| anyhow!("invalid {flag} value: `{value}`"))
}

fn parse_u16_flag<I>(args: &mut I, flag: &str) -> Result<u16>
where
    I: Iterator<Item = String>,
{
    let value = required_value(args, flag)?;
    value
        .parse::<u16>()
        .map_err(|_| anyhow!("invalid {flag} value: `{value}`"))
}

fn parse_bool_flag<I>(args: &mut I, flag: &str) -> Result<bool>
where
    I: Iterator<Item = String>,
{
    let value = required_value(args, flag)?;
    match value.as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(anyhow!("invalid {flag} value: `{value}` (use true/false)")),
    }
}

fn apply_cli_overrides(cfg: &mut config::Config, cli: &CliArgs) -> Result<()> {
    if let Some(v) = &cli.node_name {
        cfg.node.name = v.clone();
    }
    if let Some(v) = &cli.data_dir {
        cfg.node.data_dir = v.clone();
    }
    if let Some(v) = &cli.log_level {
        cfg.node.log_level = v.clone();
    }
    if let Some(v) = &cli.mode {
        cfg.node.mode = match v.as_str() {
            "solo" => config::NodeMode::Solo,
            "farm" => config::NodeMode::Farm,
            _ => return Err(anyhow!("invalid --mode value: `{v}` (use solo|farm)")),
        };
    }
    if let Some(v) = cli.receipt_cadence_tokens {
        cfg.node.receipt_cadence_tokens = v.max(1);
    }
    if !cli.include_models.is_empty() {
        cfg.node.include_models = cli.include_models.clone();
    }
    if !cli.exclude_models.is_empty() {
        cfg.node.exclude_models = cli.exclude_models.clone();
    }
    if !cli.listen_addrs.is_empty() {
        cfg.network.listen_addrs = cli.listen_addrs.clone();
    }
    if let Some(v) = cli.inference_port {
        cfg.network.inference_port = v;
    }
    if let Some(v) = &cli.external_ip {
        cfg.network.external_ip = Some(v.clone());
    }
    if !cli.bootstrap_peers.is_empty() {
        cfg.network.bootstrap_peers = cli.bootstrap_peers.clone();
    }
    if !cli.public_addr.is_empty() {
        cfg.network.public_addr = cli.public_addr.clone();
    }
    if let Some(v) = cli.allow_non_globals_in_dht {
        cfg.network.allow_non_globals_in_dht = v;
    }
    if let Some(v) = &cli.backend_url {
        cfg.backend.url = v.clone();
    }
    if let Some(v) = &cli.backend_health_path {
        cfg.backend.health_path = v.clone();
    }
    if let Some(v) = &cli.backend_models_path {
        cfg.backend.models_path = v.clone();
    }
    if let Some(v) = cli.backend_timeout_secs {
        cfg.backend.timeout_secs = v;
    }
    if let Some(v) = &cli.nras_url {
        cfg.attestation.nras_url = v.clone();
    }
    if let Some(v) = cli.nras_enabled {
        cfg.attestation.nras_enabled = v;
    }
    if let Some(v) = cli.cert_ttl_days {
        cfg.attestation.cert_ttl_days = v;
    }
    if let Some(v) = &cli.registry_url {
        cfg.registry.unicity_aggregator_url = v.clone();
    }
    if let Some(v) = cli.registry_heartbeat_secs {
        cfg.registry.heartbeat_secs = v;
    }
    if let Some(v) = cli.registry_enabled {
        cfg.registry.enabled = v;
    }
    if let Some(v) = cli.settlement_epoch_secs {
        cfg.settlement.epoch_secs = v;
    }
    if let Some(v) = &cli.evm_rpc_url {
        cfg.settlement.evm_rpc_url = v.clone();
    }
    if let Some(v) = &cli.escrow_contract {
        cfg.settlement.escrow_contract = v.clone();
    }
    if let Some(v) = cli.settlement_enabled {
        cfg.settlement.enabled = v;
    }
    if let Some(v) = cli.price_input_micro_usd_per_m {
        cfg.pricing.micro_usd_per_m_input_tokens = v;
    }
    if let Some(v) = cli.price_output_micro_usd_per_m {
        cfg.pricing.micro_usd_per_m_output_tokens = v;
    }
    Ok(())
}

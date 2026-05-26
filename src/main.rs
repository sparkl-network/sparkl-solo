use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Utc;
use sparkl_solo::attestation::{refresh_nras_tee_report_hash, NrasRuntimeState};
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
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let mut argv = std::env::args().collect::<Vec<_>>();
    if argv.len() >= 2 && argv[1] == "rotate-encryption-key" {
        argv.remove(1);
        return run_rotate_encryption_key(argv).await;
    }

    let cli = parse_cli_args(argv.into_iter().skip(1))?;
    let mut cfg = config::load(cli.config_path.as_deref())?;
    apply_cli_overrides(&mut cfg, &cli)?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(cfg.node.log_level.clone()))
        .init();

    #[cfg(feature = "evm-settlement")]
    if cfg.settlement.enabled {
        let rpc = cfg.registry.effective_evm_rpc_url(&cfg.settlement);
        let resolved = sparkl_solo::network_config::resolve_with_overrides(
            rpc,
            &cfg.registry,
            &cfg.settlement,
        )
        .await
        .map_err(|e| e.context("resolving hub contract addresses"))?;
        cfg.registry.registry_contract_address =
            sparkl_solo::network_config::format_address_cfg(resolved.provider_registry);
        cfg.settlement.escrow_contract =
            sparkl_solo::network_config::format_address_cfg(resolved.settlement_escrow);
        match sparkl_solo::network_config::effective_network_config_bootstrap_address(
            &cfg.settlement.sparkl_network_config_address,
        ) {
            Some(bootstrap) => info!(
                version = resolved.version,
                bootstrap = %bootstrap,
                "resolved hub contracts (SparklNetworkConfig bootstrap or config fallback)"
            ),
            None => info!(
                version = resolved.version,
                "resolved hub contracts from config/TOML (bootstrap address unset)"
            ),
        }
    }

    let store = Arc::new(Store::open(&cfg.node.data_dir)?);
    let pruned_sessions = store.prune_old_sessions(Duration::from_secs(60 * 60 * 24 * 30))?;
    if pruned_sessions > 0 {
        info!(
            pruned_sessions,
            "pruned completed sessions from local store"
        );
    }
    let _identity_boot = identity::load_or_generate(&cfg).await?;
    let nras_state = Arc::new(RwLock::new(NrasRuntimeState::default()));

    let (swarm_handle, swarm_cmd) =
        network::start_swarm(&_identity_boot, &cfg.network, &cfg.node.data_dir).await?;
    let identity = identity::bind_libp2p_peer_id(&swarm_handle.peer_id)?;
    info!(peer_id = %identity.peer_id, "libp2p peer id bound as canonical node identity");
    let identity_arc = Arc::new(identity.clone());

    let proxy = Arc::new(BackendProxy::new(&cfg.backend)?);
    if let Err(err) = proxy.check_health().await {
        info!(%err, "backend health check failed on startup; continuing prototype mode");
    }

    let sessions = Arc::new(SessionManager::new(store.clone()));
    sessions.recover_from_store()?;

    if cfg.registry.enabled {
        let identity_loop = identity_arc.clone();
        let proxy_arc = proxy.clone();
        let registry_cfg = cfg.registry.clone();
        let settlement_cfg = cfg.settlement.clone();
        let attestation_cfg = cfg.attestation.clone();
        let nr = nras_state.clone();
        tokio::spawn(async move {
            registry::run_registry_startup_and_heartbeat(
                identity_loop,
                proxy_arc,
                registry_cfg,
                settlement_cfg,
                attestation_cfg,
                nr,
            )
            .await;
        });
    } else if cfg.attestation.nras_enabled {
        let attestation_cfg = cfg.attestation.clone();
        let nr = nras_state.clone();
        let id = identity_arc.clone();
        let tick_secs = cfg.registry.heartbeat_secs.max(5);
        tokio::spawn(async move {
            loop {
                refresh_nras_tee_report_hash(
                    &attestation_cfg,
                    Some(id.peer_id.clone()),
                    nr.clone(),
                )
                .await;
                sleep(Duration::from_secs(tick_secs)).await;
            }
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
        nras_state,
    };

    let app = server::router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.network.inference_port));
    let listener = TcpListener::bind(addr).await?;
    info!("sparkl-solo ready on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_rotate_encryption_key(argv: Vec<String>) -> Result<()> {
    #[cfg(not(feature = "evm-settlement"))]
    {
        let _ = argv;
        eprintln!(
            "The `rotate-encryption-key` command requires the `evm-settlement` feature.\n\
             Rebuild with: cargo build --features evm-settlement --bin sparkl-solo"
        );
        std::process::exit(1);
    }

    #[cfg(feature = "evm-settlement")]
    {
        use anyhow::Context;
        use chrono::Utc;
        use sparkl_solo::registry::{
            rotate_encryption_key_with_signer, RotateEncryptionKeyOutcome,
        };

        let rotate_cli = sparkl_solo::cli::rotate_encryption_key::parse_rotate_encryption_key_args(
            argv.into_iter(),
        )?;
        let mut cfg = config::load(rotate_cli.config_path.as_deref())?;

        if cfg.settlement.enabled {
            let rpc = cfg.registry.effective_evm_rpc_url(&cfg.settlement);
            let resolved = sparkl_solo::network_config::resolve_with_overrides(
                rpc,
                &cfg.registry,
                &cfg.settlement,
            )
            .await
            .context("resolving hub contract addresses")?;
            cfg.registry.registry_contract_address =
                sparkl_solo::network_config::format_address_cfg(resolved.provider_registry);
            cfg.settlement.escrow_contract =
                sparkl_solo::network_config::format_address_cfg(resolved.settlement_escrow);
        }

        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(cfg.node.log_level.clone()))
            .init();

        let operator_key =
            resolve_rotation_operator_key(rotate_cli.operator_key.as_deref(), &cfg.settlement)?;

        identity::load_existing(&cfg)?;

        let id = identity::current_identity()?;
        let old_ver = identity::current_encryption_key_version()?;

        let outcome = rotate_encryption_key_with_signer(
            &id,
            &cfg.registry,
            &cfg.settlement,
            rotate_cli.grace_period_secs,
            &operator_key,
            rotate_cli.dry_run,
            false,
        )
        .await?;

        let next_ver = match &outcome {
            RotateEncryptionKeyOutcome::DryRun {
                next_encryption_version,
                ..
            } => *next_encryption_version,
            RotateEncryptionKeyOutcome::Submitted {
                next_encryption_version,
                ..
            } => *next_encryption_version,
        };

        let approx_end =
            Utc::now() + chrono::Duration::seconds(rotate_cli.grace_period_secs as i64);
        info!(
            old_x25519_version = old_ver,
            new_x25519_version = next_ver,
            grace_period_secs = rotate_cli.grace_period_secs,
            approx_previous_key_deprecation_end = %approx_end.to_rfc3339(),
            "encryption key rotation (wall-clock deprecation hint; chain uses block timestamps)"
        );

        match outcome {
            RotateEncryptionKeyOutcome::DryRun {
                calldata_hex,
                new_x25519_pubkey_hex,
                node_id_hex,
                registry_address,
                ..
            } => {
                println!("dry-run: no transaction sent, identity files unchanged");
                println!("registry_address: {registry_address}");
                println!("nodeId: {node_id_hex}");
                println!("new_x25519_pubkey: {new_x25519_pubkey_hex}");
                println!("rotateEncryptionKey calldata: {calldata_hex}");
            }
            RotateEncryptionKeyOutcome::Submitted { tx_hash, .. } => {
                info!(%tx_hash, "rotateEncryptionKey transaction confirmed");
            }
        }

        Ok(())
    }
}

#[cfg(feature = "evm-settlement")]
fn resolve_rotation_operator_key(
    flag: Option<&str>,
    settlement: &sparkl_solo::config::SettlementConfig,
) -> Result<String> {
    if let Some(k) = flag {
        let t = k.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    if let Ok(k) = std::env::var("SETTLEMENT_KEY") {
        let t = k.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let pk = settlement.evm_provider_wallet_private_key.trim();
    if !pk.is_empty() {
        return Ok(pk.to_string());
    }
    Err(anyhow!(
        "operator key: pass --operator-key, set SETTLEMENT_KEY, or configure settlement.evm_provider_wallet_private_key"
    ))
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
    expose_status_detail: Option<bool>,
    allow_non_globals_in_dht: Option<bool>,
    backend_url: Option<String>,
    backend_health_path: Option<String>,
    backend_models_path: Option<String>,
    backend_timeout_secs: Option<u64>,
    nras_url: Option<String>,
    nras_enabled: Option<bool>,
    nras_quote_hex: Option<String>,
    nras_signature_hex: Option<String>,
    cert_ttl_days: Option<u64>,
    registry_contract: Option<String>,
    registry_evm_rpc_url: Option<String>,
    registry_heartbeat_secs: Option<u64>,
    registry_enabled: Option<bool>,
    settlement_epoch_secs: Option<u64>,
    evm_rpc_url: Option<String>,
    escrow_contract: Option<String>,
    settlement_enabled: Option<bool>,
    sparkl_network_config_address: Option<String>,
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
        expose_status_detail: None,
        allow_non_globals_in_dht: None,
        backend_url: None,
        backend_health_path: None,
        backend_models_path: None,
        backend_timeout_secs: None,
        nras_url: None,
        nras_enabled: None,
        nras_quote_hex: None,
        nras_signature_hex: None,
        cert_ttl_days: None,
        registry_contract: None,
        registry_evm_rpc_url: None,
        registry_heartbeat_secs: None,
        registry_enabled: None,
        settlement_epoch_secs: None,
        evm_rpc_url: None,
        escrow_contract: None,
        settlement_enabled: None,
        sparkl_network_config_address: None,
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
            "--data-dir" => {
                out.data_dir = Some(PathBuf::from(required_value(&mut args, "--data-dir")?))
            }
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
            "--inference-port" => {
                out.inference_port = Some(parse_u16_flag(&mut args, "--inference-port")?)
            }
            "--external-ip" => out.external_ip = Some(required_value(&mut args, "--external-ip")?),
            "--bootstrap-peers" => {
                let value = required_value(&mut args, "--bootstrap-peers")?;
                out.bootstrap_peers.extend(parse_model_list(&value));
            }
            "--public-addr" => {
                let value = required_value(&mut args, "--public-addr")?;
                out.public_addr.extend(parse_model_list(&value));
            }
            "--expose-status-detail" => {
                out.expose_status_detail =
                    Some(parse_bool_flag(&mut args, "--expose-status-detail")?);
            }
            "--allow-non-globals-in-dht" => {
                out.allow_non_globals_in_dht =
                    Some(parse_bool_flag(&mut args, "--allow-non-globals-in-dht")?);
            }
            "--backend-url" => out.backend_url = Some(required_value(&mut args, "--backend-url")?),
            "--backend-health-path" => {
                out.backend_health_path = Some(required_value(&mut args, "--backend-health-path")?)
            }
            "--backend-models-path" => {
                out.backend_models_path = Some(required_value(&mut args, "--backend-models-path")?)
            }
            "--backend-timeout-secs" => {
                out.backend_timeout_secs =
                    Some(parse_u64_flag(&mut args, "--backend-timeout-secs")?)
            }
            "--nras-url" => out.nras_url = Some(required_value(&mut args, "--nras-url")?),
            "--nras-enabled" => {
                out.nras_enabled = Some(parse_bool_flag(&mut args, "--nras-enabled")?)
            }
            "--nras-quote-hex" => {
                out.nras_quote_hex = Some(required_value(&mut args, "--nras-quote-hex")?)
            }
            "--nras-signature-hex" => {
                out.nras_signature_hex = Some(required_value(&mut args, "--nras-signature-hex")?)
            }
            "--cert-ttl-days" => {
                out.cert_ttl_days = Some(parse_u64_flag(&mut args, "--cert-ttl-days")?)
            }
            "--registry-contract" => {
                out.registry_contract = Some(required_value(&mut args, "--registry-contract")?)
            }
            "--registry-evm-rpc-url" => {
                out.registry_evm_rpc_url =
                    Some(required_value(&mut args, "--registry-evm-rpc-url")?)
            }
            "--registry-heartbeat-secs" => {
                out.registry_heartbeat_secs =
                    Some(parse_u64_flag(&mut args, "--registry-heartbeat-secs")?)
            }
            "--registry-enabled" => {
                out.registry_enabled = Some(parse_bool_flag(&mut args, "--registry-enabled")?)
            }
            "--settlement-epoch-secs" => {
                out.settlement_epoch_secs =
                    Some(parse_u64_flag(&mut args, "--settlement-epoch-secs")?)
            }
            "--evm-rpc-url" => out.evm_rpc_url = Some(required_value(&mut args, "--evm-rpc-url")?),
            "--escrow-contract" => {
                out.escrow_contract = Some(required_value(&mut args, "--escrow-contract")?)
            }
            "--settlement-enabled" => {
                out.settlement_enabled = Some(parse_bool_flag(&mut args, "--settlement-enabled")?)
            }
            "--sparkl-network-config-address" => {
                out.sparkl_network_config_address = Some(required_value(
                    &mut args,
                    "--sparkl-network-config-address",
                )?)
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
    if let Some(v) = cli.expose_status_detail {
        cfg.network.expose_status_detail = v;
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
    if let Some(v) = &cli.nras_quote_hex {
        cfg.attestation.nras_quote_hex = v.clone();
    }
    if let Some(v) = &cli.nras_signature_hex {
        cfg.attestation.nras_signature_hex = v.clone();
    }
    if let Some(v) = cli.cert_ttl_days {
        cfg.attestation.cert_ttl_days = v;
    }
    if let Some(v) = &cli.registry_contract {
        cfg.registry.registry_contract_address = v.clone();
    }
    if let Some(v) = &cli.registry_evm_rpc_url {
        cfg.registry.evm_rpc_url = v.clone();
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
    if let Some(v) = &cli.sparkl_network_config_address {
        cfg.settlement.sparkl_network_config_address = v.clone();
    }
    Ok(())
}

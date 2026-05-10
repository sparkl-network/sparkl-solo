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
    if let Some(cadence) = cli.receipt_cadence_tokens {
        cfg.node.receipt_cadence_tokens = cadence.max(1);
    }
    if !cli.include_models.is_empty() {
        cfg.node.include_models = cli.include_models.clone();
    }
    if !cli.exclude_models.is_empty() {
        cfg.node.exclude_models = cli.exclude_models.clone();
    }
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
    receipt_cadence_tokens: Option<u64>,
    include_models: Vec<String>,
    exclude_models: Vec<String>,
}

fn parse_cli_args<I>(mut args: I) -> Result<CliArgs>
where
    I: Iterator<Item = String>,
{
    let mut out = CliArgs {
        config_path: None,
        receipt_cadence_tokens: None,
        include_models: Vec::new(),
        exclude_models: Vec::new(),
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
            "--include-models" => {
                let Some(value) = args.next() else {
                    return Err(anyhow!("--include-models requires a comma-separated value"));
                };
                out.include_models.extend(parse_model_list(&value));
            }
            "--exclude-models" => {
                let Some(value) = args.next() else {
                    return Err(anyhow!("--exclude-models requires a comma-separated value"));
                };
                out.exclude_models.extend(parse_model_list(&value));
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

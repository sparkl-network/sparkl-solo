use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
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
    let cfg = config::load(None)?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(cfg.node.log_level.clone()))
        .init();

    let store = Arc::new(Store::open(&cfg.node.data_dir)?);
    let identity = identity::load_or_generate(&cfg).await?;
    let _ = network::start_swarm(&identity, &cfg.network).await?;

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
        started_at: Utc::now(),
    };

    let app = server::router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.network.inference_port));
    let listener = TcpListener::bind(addr).await?;
    info!("sparkl-solo ready on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

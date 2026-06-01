//! Outbound WebSocket client to sparkl-router (`/node/connect`).

mod activate;
mod challenge;
mod connect;
mod forward;
mod frames;
mod url;

pub use activate::verify_sk_bearer;
pub use challenge::connect_challenge_payload;
pub use url::normalize_router_ws_url;

use std::time::Duration;

use rand::Rng;
use reqwest::Client;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::config::Config;
use crate::identity::NodeIdentity;

/// Background task: maintain router tunnel with exponential backoff reconnect.
pub async fn run(cfg: Config, identity: NodeIdentity) {
    let ws_url = match normalize_router_ws_url(&cfg.router.url) {
        Ok(u) => u,
        Err(e) => {
            warn!(%e, "invalid router.url; router client disabled");
            return;
        }
    };

    let local_base = cfg
        .router
        .effective_local_inference_base(cfg.network.inference_port);
    let min_delay = Duration::from_secs(cfg.router.reconnect_min_secs.max(1));
    let max_delay = Duration::from_secs(
        cfg.router
            .reconnect_max_secs
            .max(min_delay.as_secs()),
    );

    let http = match Client::builder()
        .timeout(Duration::from_secs(cfg.backend.timeout_secs.max(30)))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(%e, "failed to build HTTP client for router forwarder");
            return;
        }
    };

    let mut backoff = min_delay;

    loop {
        info!(%ws_url, %local_base, "connecting to sparkl-router");

        let result = connect::run_connected_session(
            &ws_url,
            &identity,
            http.clone(),
            local_base.clone(),
            cfg.settlement.clone(),
            cfg.registry.clone(),
        )
        .await;

        match result {
            Ok(()) => warn!("router tunnel closed; reconnecting"),
            Err(e) => warn!(%e, "router tunnel error; reconnecting"),
        }

        let jitter = rand::thread_rng().gen_range(0..=backoff.as_millis() as u64 / 4);
        sleep(backoff + Duration::from_millis(jitter)).await;
        backoff = (backoff * 2).min(max_delay);
    }
}

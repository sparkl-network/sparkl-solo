#[cfg(feature = "evm-settlement")]
pub(crate) mod evm;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::config::{RouterConfig, SettlementConfig};
use crate::identity::NodeIdentity;
use crate::session::{Session, SessionManager};
use crate::store::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochBatch {
    pub epoch_id: u64,
    pub provider_peer_id: String,
    pub session_count: u32,
    pub total_tokens_output: u64,
    pub total_micro_usd: u64,
    pub receipts_root: [u8; 32],
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

pub async fn run_epoch_loop(
    sessions: Arc<SessionManager>,
    store: Arc<Store>,
    identity: Arc<NodeIdentity>,
    config: SettlementConfig,
    router: RouterConfig,
) {
    if !config.enabled {
        warn!("settlement disabled; running log-only mode");
        return;
    }

    let tee_every = config.tee_tick_secs.max(1);
    let epoch_every = config.epoch_secs.max(1);
    let mut tee_remain = tee_every;
    let mut epoch_remain = epoch_every;
    let mut epoch_id: u64 = 0;
    #[cfg_attr(not(feature = "evm-settlement"), allow(unused_variables, unused_mut))]
    let mut tee_last_eligible_block: Option<u64> = None;

    loop {
        let sleep_secs = tee_remain.min(epoch_remain).max(1);
        sleep(Duration::from_secs(sleep_secs)).await;

        tee_remain = tee_remain.saturating_sub(sleep_secs);
        epoch_remain = epoch_remain.saturating_sub(sleep_secs);

        let mut tee_tick_now = false;
        let mut epoch_boundary = false;

        if tee_remain == 0 {
            tee_tick_now = true;
            tee_remain = tee_every;
        }
        if epoch_remain == 0 {
            epoch_boundary = true;
            epoch_remain = epoch_every;
        }

        let tee_candidates = if tee_tick_now {
            sessions.tee_touch_candidates(config.tee_settle_tokens_threshold)
        } else {
            Vec::new()
        };

        if epoch_boundary {
            epoch_id = epoch_id.saturating_add(1);
            let pending = sessions.pending_settlement();
            let total_tokens = pending.iter().map(|s| s.tokens_output).sum();
            info!(
                epoch_id,
                peer_id = %identity.peer_id,
                sessions = pending.len(),
                total_tokens,
                "settlement epoch boundary"
            );
            let receipts_root = compute_receipts_root(&pending);
            let _ = store.save_epoch(&EpochBatch {
                epoch_id,
                provider_peer_id: identity.peer_id.clone(),
                session_count: pending.len() as u32,
                total_tokens_output: total_tokens,
                total_micro_usd: pending.iter().map(|s| s.amount_micro_usd).sum(),
                receipts_root,
                started_at: Utc::now(),
                ended_at: Utc::now(),
            });
        } else if tee_tick_now && !tee_candidates.is_empty() {
            info!(tee_candidates = tee_candidates.len(), "settlement tee tick");
        }

        #[cfg(feature = "evm-settlement")]
        {
            let skip_provider_record_usage =
                router.enabled && config.router_usage_metering;
            evm::process_settlement_tick(
                &config,
                sessions.clone(),
                tee_tick_now,
                epoch_boundary,
                &tee_candidates,
                &mut tee_last_eligible_block,
                skip_provider_record_usage,
            )
            .await;
        }
    }
}

fn compute_receipts_root(sessions: &[Session]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for session in sessions {
        for receipt in &session.receipts {
            let leaf = serde_json::to_vec(receipt).unwrap_or_default();
            hasher.update(Sha256::digest(&leaf));
        }
    }
    hasher.finalize().into()
}

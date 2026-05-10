// STUB: This module is intentionally incomplete.
// Safe to extend. Do not call from production paths without a feature flag.
// See AGENTS.md for the full stub list.
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::config::SettlementConfig;
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
) {
    if !config.enabled {
        warn!("settlement disabled; running log-only mode");
        return;
    }
    let mut epoch: u64 = 0;
    loop {
        epoch = epoch.saturating_add(1);
        let pending = sessions.pending_settlement();
        let total_tokens = pending.iter().map(|s| s.tokens_output).sum();
        info!(
            epoch_id = epoch,
            peer_id = %identity.peer_id,
            sessions = pending.len(),
            total_tokens,
            "settlement prototype tick"
        );
        let _ = store.save_epoch(&EpochBatch {
            epoch_id: epoch,
            provider_peer_id: identity.peer_id.clone(),
            session_count: pending.len() as u32,
            total_tokens_output: total_tokens,
            total_micro_usd: pending.iter().map(|s| s.amount_micro_usd).sum(),
            receipts_root: compute_receipts_root(&pending),
            started_at: Utc::now(),
            ended_at: Utc::now(),
        });
        sleep(Duration::from_secs(config.epoch_secs.max(1))).await;
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

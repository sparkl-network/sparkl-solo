use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::receipts::ChunkReceipt;
use crate::session::Session;
use crate::settlement::EpochBatch;

#[derive(Clone)]
pub struct Store {
    db: sled::Db,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).context("failed to create data dir for store")?;
        let db_path = data_dir.join("db");
        let db = sled::open(db_path).context("failed to open sled db")?;
        Ok(Self { db })
    }

    pub fn save_session(&self, session: &Session) -> Result<()> {
        let tree = self.db.open_tree("sessions")?;
        tree.insert(session.id.as_bytes(), serde_json::to_vec(session)?)?;
        tree.flush()?;
        Ok(())
    }

    pub fn load_session(&self, id: Uuid) -> Result<Option<Session>> {
        let tree = self.db.open_tree("sessions")?;
        let value = tree.get(id.as_bytes())?;
        let Some(bytes) = value else {
            return Ok(None);
        };
        let session = serde_json::from_slice(&bytes).context("invalid session payload")?;
        Ok(Some(session))
    }

    pub fn all_sessions(&self) -> Result<Vec<Session>> {
        let tree = self.db.open_tree("sessions")?;
        let mut out = Vec::new();
        for entry in tree.iter() {
            let (_, bytes) = entry?;
            out.push(serde_json::from_slice::<Session>(&bytes)?);
        }
        Ok(out)
    }

    pub fn save_receipt(&self, receipt: &ChunkReceipt) -> Result<()> {
        let tree = self.db.open_tree("receipts")?;
        let key = format!("{}:{}", receipt.session_id, receipt.seq);
        tree.insert(key.as_bytes(), serde_json::to_vec(receipt)?)?;
        tree.flush()?;
        Ok(())
    }

    pub fn receipts_for_session(&self, id: Uuid) -> Result<Vec<ChunkReceipt>> {
        let tree = self.db.open_tree("receipts")?;
        let prefix = format!("{id}:");
        let mut out = Vec::new();
        for item in tree.scan_prefix(prefix.as_bytes()) {
            let (_, bytes) = item?;
            out.push(serde_json::from_slice::<ChunkReceipt>(&bytes)?);
        }
        out.sort_by_key(|r| r.seq);
        Ok(out)
    }

    pub fn save_epoch(&self, epoch: &EpochBatch) -> Result<()> {
        let tree = self.db.open_tree("epochs")?;
        tree.insert(epoch.epoch_id.to_be_bytes(), serde_json::to_vec(epoch)?)?;
        tree.flush()?;
        Ok(())
    }

    pub fn prune_old_sessions(&self, older_than: Duration) -> Result<u64> {
        let tree = self.db.open_tree("sessions")?;
        let now = Utc::now();
        let mut removed = 0_u64;
        let mut to_remove = Vec::new();

        for item in tree.iter() {
            let (k, v) = item?;
            let session: Session = serde_json::from_slice(&v)?;
            if let Some(ended_at) = session.ended_at {
                if is_older_than(ended_at, now, older_than) {
                    to_remove.push(k);
                }
            }
        }

        for k in to_remove {
            tree.remove(k)?;
            removed += 1;
        }
        tree.flush()?;
        Ok(removed)
    }
}

fn is_older_than(ended_at: DateTime<Utc>, now: DateTime<Utc>, older_than: Duration) -> bool {
    now.signed_duration_since(ended_at)
        .to_std()
        .map(|age| age > older_than)
        .unwrap_or(false)
}

use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::receipts::ChunkReceipt;
use crate::store::Store;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionState {
    Opening,
    Active,
    Completed,
    ConsumerDisconnected,
    ProviderError,
    Disputed,
    Settled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub consumer_pubkey: Option<[u8; 32]>,
    pub model: String,
    pub state: SessionState,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub receipts: Vec<ChunkReceipt>,
    pub last_receipt_seq: u64,
    pub amount_micro_usd: u64,
}

#[derive(Clone)]
pub struct SessionManager {
    sessions: DashMap<Uuid, Arc<Mutex<Session>>>,
    store: Arc<Store>,
}

impl SessionManager {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            sessions: DashMap::new(),
            store,
        }
    }

    pub fn open(&self, model: &str, consumer_pubkey: Option<[u8; 32]>) -> Uuid {
        let id = Uuid::new_v4();
        let session = Session {
            id,
            consumer_pubkey,
            model: model.to_string(),
            state: SessionState::Active,
            started_at: Utc::now(),
            ended_at: None,
            tokens_input: 0,
            tokens_output: 0,
            receipts: Vec::new(),
            last_receipt_seq: 0,
            amount_micro_usd: 0,
        };
        let _ = self.store.save_session(&session);
        self.sessions.insert(id, Arc::new(Mutex::new(session)));
        id
    }

    pub fn record_chunk(
        &self,
        id: Uuid,
        tokens: u32,
        _content_hash: [u8; 32],
        price_per_m_output_tokens: u64,
    ) {
        if let Some(entry) = self.sessions.get(&id) {
            let mut guard = entry.lock().expect("session lock poisoned");
            guard.tokens_output = guard.tokens_output.saturating_add(tokens as u64);
            // micro_usd = tokens * (micro_usd per million tokens) / 1_000_000
            let chunk_cost = (tokens as u64)
                .saturating_mul(price_per_m_output_tokens)
                .saturating_div(1_000_000);
            guard.amount_micro_usd = guard.amount_micro_usd.saturating_add(chunk_cost);
            let _ = self.store.save_session(&guard);
        }
    }

    pub fn add_receipt(&self, id: Uuid, receipt: ChunkReceipt) {
        if let Some(entry) = self.sessions.get(&id) {
            let mut guard = entry.lock().expect("session lock poisoned");
            guard.last_receipt_seq = receipt.seq;
            guard.receipts.push(receipt.clone());
            let _ = self.store.save_receipt(&receipt);
            let _ = self.store.save_session(&guard);
        }
    }

    pub fn close(&self, id: Uuid, state: SessionState) {
        if let Some(entry) = self.sessions.get(&id) {
            let mut guard = entry.lock().expect("session lock poisoned");
            guard.state = state;
            guard.ended_at = Some(Utc::now());
            let _ = self.store.save_session(&guard);
        }
    }

    pub fn get(&self, id: Uuid) -> Option<Session> {
        let entry = self.sessions.get(&id)?;
        let guard = entry.lock().ok()?;
        Some(guard.clone())
    }

    pub fn pending_settlement(&self) -> Vec<Session> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                let guard = entry.lock().ok()?;
                if guard.state == SessionState::Completed {
                    Some(guard.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn recover_from_store(&self) -> Result<()> {
        for session in self.store.all_sessions()? {
            if matches!(session.state, SessionState::Opening | SessionState::Active) {
                self.sessions
                    .insert(session.id, Arc::new(Mutex::new(session)));
            }
        }
        Ok(())
    }

    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|entry| {
                entry
                    .lock()
                    .map(|s| matches!(s.state, SessionState::Opening | SessionState::Active))
                    .unwrap_or(false)
            })
            .count()
    }

    pub fn save_unicity_proof(&self, session_id: Uuid, seq: u64, proof_hex: &str) -> Result<()> {
        self.store.save_unicity_proof(session_id, seq, proof_hex)
    }

    pub fn get_unicity_proof(&self, session_id: Uuid, seq: u64) -> Result<Option<String>> {
        self.store.load_unicity_proof(session_id, seq)
    }
}

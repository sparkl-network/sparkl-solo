use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy)]
pub enum AcquireError {
    QueueFull {
        active: u32,
        concurrency: u32,
        queued: u32,
    },
    WaitTimeout {
        active: u32,
        concurrency: u32,
        queued: u32,
    },
}

struct Slot {
    active: AtomicU32,
    queued: AtomicU32,
    notify: Notify,
}

#[derive(Clone)]
pub struct ModelAdmission {
    slots: Arc<DashMap<String, Arc<Slot>>>,
}

impl Default for ModelAdmission {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelAdmission {
    pub fn new() -> Self {
        Self {
            slots: Arc::new(DashMap::new()),
        }
    }

    pub fn active_count_for_model(&self, model_id: &str) -> u32 {
        self.slots
            .get(model_id)
            .map(|s| s.active.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub async fn acquire(
        &self,
        model_id: &str,
        concurrency: u32,
        max_queue: u32,
        wait_timeout: Duration,
    ) -> Result<AdmissionGuard, AcquireError> {
        if concurrency == 0 {
            return Ok(AdmissionGuard {
                admission: self.clone(),
                model_id: model_id.to_string(),
                unlimited: true,
            });
        }

        let slot = self.slot_for(model_id);
        let deadline = tokio::time::Instant::now() + wait_timeout;

        loop {
            let active = slot.active.load(Ordering::Acquire);
            if active < concurrency {
                match slot.active.compare_exchange_weak(
                    active,
                    active + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        return Ok(AdmissionGuard {
                            admission: self.clone(),
                            model_id: model_id.to_string(),
                            unlimited: false,
                        });
                    }
                    Err(_) => continue,
                }
            }

            let queued = slot.queued.load(Ordering::Acquire);
            if max_queue == 0 || queued >= max_queue {
                return Err(AcquireError::QueueFull {
                    active,
                    concurrency,
                    queued,
                });
            }

            match slot.queued.compare_exchange_weak(
                queued,
                queued + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    let notified = tokio::time::timeout(remaining, slot.notify.notified()).await;
                    slot.queued.fetch_sub(1, Ordering::AcqRel);
                    if notified.is_err() {
                        return Err(AcquireError::WaitTimeout {
                            active: slot.active.load(Ordering::Relaxed),
                            concurrency,
                            queued: slot.queued.load(Ordering::Relaxed),
                        });
                    }
                    continue;
                }
                Err(_) => continue,
            }
        }
    }

    fn release_inner(&self, model_id: &str) {
        if let Some(slot) = self.slots.get(model_id) {
            slot.active.fetch_sub(1, Ordering::AcqRel);
            slot.notify.notify_one();
        }
    }

    fn slot_for(&self, model_id: &str) -> Arc<Slot> {
        if let Some(slot) = self.slots.get(model_id) {
            return Arc::clone(slot.value());
        }
        let slot = Arc::new(Slot {
            active: AtomicU32::new(0),
            queued: AtomicU32::new(0),
            notify: Notify::new(),
        });
        self.slots
            .entry(model_id.to_string())
            .or_insert_with(|| Arc::clone(&slot))
            .clone()
    }
}

pub struct AdmissionGuard {
    admission: ModelAdmission,
    model_id: String,
    unlimited: bool,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        if !self.unlimited {
            self.admission.release_inner(&self.model_id);
        }
    }
}

pub fn max_queue_depth(concurrency: u32, queue_depth_ratio: f64) -> u32 {
    if concurrency == 0 {
        return 0;
    }
    let depth = (concurrency as f64 * queue_depth_ratio).floor() as u32;
    depth.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admission_tracks_active_per_model() {
        let admission = ModelAdmission::new();
        let g1 = admission
            .acquire("m1", 2, 0, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(admission.active_count_for_model("m1"), 1);
        drop(g1);
        assert_eq!(admission.active_count_for_model("m1"), 0);
    }
}

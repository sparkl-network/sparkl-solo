//! Glue between NRAS HTTP client, ProviderRegistry **`setTEEProof`**, and HTTP observability.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::AttestationConfig;

use super::NrasClient;

/// Snapshot surfaced on **`GET /identity`** / **`GET /status/detail`**.
#[derive(Debug, Clone)]
pub struct NrasUiSnapshot {
    pub mode: &'static str,
    pub verified: Option<bool>,
    pub status: String,
    /// Normalized `0x` + 64-hex (**`bytes32`**) when NRAS succeeds.
    pub tee_report_hash: Option<String>,
    pub last_error: Option<String>,
    pub verified_at_unix: Option<u64>,
    pub expires_at_unix: Option<u64>,
}

impl Default for NrasUiSnapshot {
    fn default() -> Self {
        Self {
            mode: "disabled",
            verified: None,
            status: "nras_not_configured".into(),
            tee_report_hash: None,
            last_error: None,
            verified_at_unix: None,
            expires_at_unix: None,
        }
    }
}

/// Shared heartbeat + HTTP state for NRAS.
#[derive(Debug, Default)]
pub struct NrasRuntimeState {
    pub ui: NrasUiSnapshot,
    cached_good_hash: Option<String>,
    cached_verified_at_unix: Option<u64>,
    last_warn_key: Option<String>,
}

pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Normalize **`tee_report_hash`** from NRAS to `0x` + 64 lowercase hex for registry parsing.
pub fn normalize_tee_report_hash(raw: &str) -> Result<String> {
    let s = raw.trim().strip_prefix("0x").unwrap_or(raw.trim());
    let bytes = hex::decode(s).map_err(|e| anyhow!("invalid tee_report_hash hex: {e}"))?;
    if bytes.len() != 32 {
        bail!(
            "tee_report_hash must be 32 bytes, got `{}` len {} decoded",
            raw,
            bytes.len()
        );
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

pub fn ttl_secs(attestation: &AttestationConfig) -> u64 {
    attestation.cert_ttl_days.saturating_mul(86400)
}

pub fn truncate_hash_display(hex: &str, prefix_chars: usize) -> String {
    let t = hex.trim();
    let no0x = t.strip_prefix("0x").unwrap_or(t);
    if no0x.len() <= prefix_chars + 6 {
        return format!("0x{no0x}");
    }
    format!("0x{}…{}", &no0x[..prefix_chars], &no0x[no0x.len() - 4..])
}

fn missing_inputs_err() -> anyhow::Error {
    anyhow!(
        "missing attestation inputs: set `[attestation] nras_quote_hex` / `nras_signature_hex` or SPARKLE_ATTESTATION__NRAS_QUOTE_HEX / NRAS_SIGNATURE_HEX"
    )
}

fn log_once(slot: &mut Option<String>, key: &str, emit: impl FnOnce()) {
    if slot.as_deref() != Some(key) {
        emit();
        *slot = Some(key.to_string());
    }
}

fn maybe_use_cache_on_failure(
    guard: &mut NrasRuntimeState,
    now: u64,
    ttl: u64,
    err_msg: &str,
) -> Option<String> {
    if let (Some(h), Some(t)) = (
        guard.cached_good_hash.clone(),
        guard.cached_verified_at_unix,
    ) {
        if now.saturating_sub(t) < ttl {
            let exp = t.saturating_add(ttl);
            guard.ui = NrasUiSnapshot {
                mode: "nras",
                verified: Some(true),
                status: "cached_ttl_after_failure".into(),
                tee_report_hash: Some(h.clone()),
                last_error: Some(err_msg.to_string()),
                verified_at_unix: Some(t),
                expires_at_unix: Some(exp),
            };
            let key = "nras_fallback_cache";
            log_once(&mut guard.last_warn_key, key, || {
                warn!(
                    error = %err_msg,
                    "NRAS verification failed; using cached tee_report_hash within TTL"
                );
            });
            return Some(h);
        }
    }

    guard.ui = NrasUiSnapshot {
        mode: "nras",
        verified: Some(false),
        status: "nras_failed".into(),
        tee_report_hash: None,
        last_error: Some(err_msg.to_string()),
        verified_at_unix: guard.ui.verified_at_unix,
        expires_at_unix: guard.ui.expires_at_unix,
    };
    let key = format!("fail:{err_msg}");
    log_once(&mut guard.last_warn_key, &key, || {
        warn!(error = %err_msg, "NRAS verification failed");
    });
    None
}

/// Computes the hash used for **`ProviderState.attestation_hash`** / **`setTEEProof`** and updates **`state`** for HTTP surfaces.
///
/// When quote/signature are missing, returns the cached hash if **`cert_ttl_days`** has not expired.
pub async fn refresh_nras_tee_report_hash(
    attestation: &AttestationConfig,
    provider_id: Option<String>,
    state: Arc<RwLock<NrasRuntimeState>>,
) -> String {
    let mut guard = state.write().await;

    if !attestation.nras_enabled {
        guard.ui = NrasUiSnapshot {
            mode: "disabled",
            verified: None,
            status: "nras_disabled".into(),
            tee_report_hash: guard.ui.tee_report_hash.take(),
            last_error: None,
            verified_at_unix: None,
            expires_at_unix: None,
        };
        guard.cached_good_hash = None;
        guard.cached_verified_at_unix = None;
        return String::new();
    }

    let url = attestation.nras_url.trim().to_owned();
    if url.is_empty() {
        guard.ui = NrasUiSnapshot {
            mode: "nras",
            verified: Some(false),
            status: "nras_url_missing".into(),
            tee_report_hash: None,
            last_error: Some("attestation.nras_url is empty".into()),
            verified_at_unix: None,
            expires_at_unix: None,
        };
        log_once(&mut guard.last_warn_key, "nras_url_missing", || {
            warn!("NRAS enabled but attestation.nras_url is empty; skipping NRAS verification");
        });
        return String::new();
    }

    let ttl = ttl_secs(attestation);
    let now = unix_now_secs();

    let quote = attestation.nras_quote_hex.trim();
    let signature = attestation.nras_signature_hex.trim();
    let inputs_ready = !quote.is_empty() && !signature.is_empty();

    if !inputs_ready {
        let err = missing_inputs_err();

        if let (Some(h), Some(t)) = (
            guard.cached_good_hash.clone(),
            guard.cached_verified_at_unix,
        ) {
            if now.saturating_sub(t) < ttl {
                let exp = t.saturating_add(ttl);
                guard.ui = NrasUiSnapshot {
                    mode: "nras",
                    verified: Some(true),
                    status: "cached_ttl".into(),
                    tee_report_hash: Some(h.clone()),
                    last_error: Some(err.to_string()),
                    verified_at_unix: Some(t),
                    expires_at_unix: Some(exp),
                };
                info!(
                    ttl_secs = ttl,
                    remaining_secs = ttl.saturating_sub(now.saturating_sub(t)),
                    "using cached NRAS tee_report_hash within cert_ttl_days window"
                );
                return h;
            }
        }

        guard.ui = NrasUiSnapshot {
            mode: "nras",
            verified: Some(false),
            status: "inputs_missing_or_expired_cache".into(),
            tee_report_hash: None,
            last_error: Some(err.to_string()),
            verified_at_unix: guard.ui.verified_at_unix,
            expires_at_unix: guard.ui.expires_at_unix,
        };

        log_once(&mut guard.last_warn_key, "nras_inputs_missing", || {
            warn!(error = %err, "NRAS verification skipped");
        });

        return String::new();
    }

    guard.last_warn_key = None;

    let client = NrasClient::new(url);
    let flow = client
        .full_attestation_flow(quote, signature, provider_id.clone())
        .await;

    match flow {
        Ok((raw_hash, _chain)) => match normalize_tee_report_hash(&raw_hash) {
            Ok(normalized) => {
                let t = unix_now_secs();
                let exp = t.saturating_add(ttl);
                guard.cached_good_hash = Some(normalized.clone());
                guard.cached_verified_at_unix = Some(t);
                guard.ui = NrasUiSnapshot {
                    mode: "nras",
                    verified: Some(true),
                    status: "verified_ok".into(),
                    tee_report_hash: Some(normalized.clone()),
                    last_error: None,
                    verified_at_unix: Some(t),
                    expires_at_unix: Some(exp),
                };
                normalized
            }
            Err(e) => {
                let msg = e.to_string();
                maybe_use_cache_on_failure(&mut guard, now, ttl, &msg).unwrap_or_default()
            }
        },
        Err(e) => {
            let msg = e.to_string();
            maybe_use_cache_on_failure(&mut guard, now, ttl, &msg).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_0x_and_raw() {
        let raw = "a".repeat(64);
        let with = format!("0x{raw}");
        assert_eq!(normalize_tee_report_hash(&with).unwrap(), with);
        assert_eq!(normalize_tee_report_hash(&raw).unwrap(), with);
    }
}

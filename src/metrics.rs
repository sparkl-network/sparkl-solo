// Prometheus metrics for sparkl-solo provider node.
//
// Metrics are registered at startup and instrumented throughout the lifecycle:
// sessions, earnings, peer count, registry/settlement errors.
//
// Usage in other modules (after calling `init_registry()`):
//   use crate::metrics;
//   metrics::inc_sessions_opened("tee_verified");
//   metrics::observe_session_duration("best_effort", 2.5);

use std::sync::OnceLock;

use prometheus::{
    Counter, CounterVec, Encoder, Gauge, GaugeVec, HistogramOpts, HistogramVec, Opts, Registry,
    TextEncoder, DEFAULT_BUCKETS,
};

// ── Metric handles (global registration) ──────────────────────────────────────

/// Single point of initialization: creates registry, builds + registers all metrics, and caches
/// handle references.  Only one pair is ever created (OnceLock guarantees single-execution even
/// under concurrent access), which avoids the `AlreadyReg` panic that occurred when two separate
// OnceLocks (`REGISTRY` / `HANDLES`) could each spawn their own registry instance.
static REGISTRY_AND_HANDLES: OnceLock<(Registry, MetricHandles)> = OnceLock::new();

/// Register all metrics and return the registry. Call once at startup.
pub fn init_registry() -> &'static Registry {
    let (_reg, _handles) = REGISTRY_AND_HANDLES.get_or_init(|| {
        let reg = Registry::new();
        // build_handles creates every metric and registers it with `reg`, then returns the handle struct.
        let handles = build_handles(&reg);
        (reg, handles)
    });
    _reg
}

/// Return the encoder output (Prometheus text format) for an HTTP `/metrics` endpoint.
pub fn encode_prometheus() -> String {
    init_registry(); // ensure registry exists
    let mut buf = Vec::new();
    if let Some((ref reg, _)) = REGISTRY_AND_HANDLES.get() {
        let families = reg.gather();
        let encoder = TextEncoder::new();
        let _ = encoder.encode(&families, &mut buf);
    }
    String::from_utf8(buf).unwrap_or_default()
}

// ── Handle struct for convenient per-call access ─────────────────────────────

#[derive(Debug)]
pub struct MetricHandles {
    pub sessions_total: CounterVec,
    pub sessions_closed_total: CounterVec,
    pub sessions_active: Gauge,
    pub earnings_micro_usd_total: CounterVec,
    pub session_duration_seconds: HistogramVec,
    pub tokens_total: CounterVec,
    pub peers_known: GaugeVec,
    pub registry_ops_total: Counter,
    pub registry_errors_total: CounterVec,
    pub settlement_ops_total: CounterVec,
    pub settlement_errors_total: CounterVec,
    pub settled_micro_usd_total: Counter,
    pub uptime_seconds: Gauge,
    pub requests_total: Counter,
    pub request_errors_total: CounterVec,
}

fn build_handles(reg: &Registry) -> MetricHandles {
    // ── Counters with labels (CounterVec) ────────────────────────────────────
    let sessions_total = CounterVec::new(
        Opts::new("sparkl_sessions_total", "Total inference sessions opened"),
        &["tier"],
    )
    .unwrap();
    reg.register(Box::new(sessions_total.clone()))
        .expect("register counter vec");

    let sessions_closed_total = CounterVec::new(
        Opts::new(
            "sparkl_sessions_closed_total",
            "Total inference sessions closed",
        ),
        &["outcome"],
    )
    .unwrap();
    reg.register(Box::new(sessions_closed_total.clone()))
        .expect("register counter vec");

    let earnings_micro_usd_total = CounterVec::new(
        Opts::new(
            "sparkl_earnings_micro_usd_total",
            "Total earnings in micro-USDC accumulated from closed sessions",
        ),
        &["tier"],
    )
    .unwrap();
    reg.register(Box::new(earnings_micro_usd_total.clone()))
        .expect("register counter vec");

    let session_duration_seconds = HistogramVec::new(
        HistogramOpts::new(
            "sparkl_session_duration_seconds",
            "Inference session duration in seconds",
        )
        .buckets(DEFAULT_BUCKETS.to_vec()),
        &["tier"],
    )
    .unwrap();

    reg.register(Box::new(session_duration_seconds.clone()))
        .expect("register histogram vec");

    let tokens_total = CounterVec::new(
        Opts::new(
            "sparkl_tokens_total",
            "Total tokens consumed (input + output)",
        ),
        &["role"],
    )
    .unwrap();
    reg.register(Box::new(tokens_total.clone()))
        .expect("register counter vec");

    // ── Gauges with labels (GaugeVec) ────────────────────────────────────────
    let peers_known = GaugeVec::new(
        Opts::new("sparkl_peers_known", "Number of known P2P peers on the DHT"),
        &["role"],
    )
    .unwrap();
    reg.register(Box::new(peers_known.clone()))
        .expect("register gauge vec");

    // ── Single-value counters (Counter + register) ───────────────────────────
    let registry_ops_total = Counter::with_opts(Opts::new(
        "sparkl_registry_operations_total",
        "Total registry operations (register, heartbeat, chill, defunct)",
    ))
    .unwrap();
    reg.register(Box::new(registry_ops_total.clone()))
        .expect("register counter");

    let settled_micro_usd_total = Counter::with_opts(Opts::new(
        "sparkl_settled_micro_usd_total",
        "Total micro-USDC settled on-chain via SettlementEscrow",
    ))
    .unwrap();
    reg.register(Box::new(settled_micro_usd_total.clone()))
        .expect("register counter");

    let requests_total = Counter::with_opts(Opts::new(
        "sparkl_requests_total",
        "Total HTTP inference requests received",
    ))
    .unwrap();
    reg.register(Box::new(requests_total.clone()))
        .expect("register counter");

    // ── Single-value gauges (Gauge + register) ───────────────────────────────
    let sessions_active = Gauge::with_opts(Opts::new(
        "sparkl_sessions_active",
        "Number of currently active inference sessions",
    ))
    .unwrap();
    reg.register(Box::new(sessions_active.clone()))
        .expect("register gauge");

    let uptime_seconds = Gauge::with_opts(Opts::new(
        "sparkl_uptime_seconds",
        "Node uptime in seconds since start",
    ))
    .unwrap();
    reg.register(Box::new(uptime_seconds.clone()))
        .expect("register gauge");

    // ── CounterVecs for errors (CounterVec) ──────────────────────────────────
    let registry_errors_total = CounterVec::new(
        Opts::new(
            "sparkl_registry_errors_total",
            "Registry operation errors by operation type",
        ),
        &["op"],
    )
    .unwrap();

    let settlement_ops_total = CounterVec::new(
        Opts::new(
            "sparkl_settlement_operations_total",
            "Total settlement operations (deposit, withdraw, settle)",
        ),
        &["op"],
    )
    .unwrap();

    let settlement_errors_total = CounterVec::new(
        Opts::new(
            "sparkl_settlement_errors_total",
            "Settlement operation errors by operation type",
        ),
        &["op"],
    )
    .unwrap();

    let request_errors_total = CounterVec::new(
        Opts::new(
            "sparkl_request_errors_total",
            "Inference request errors by HTTP status code",
        ),
        &["status_code"],
    )
    .unwrap();

    MetricHandles {
        sessions_total,
        sessions_closed_total,
        sessions_active,
        earnings_micro_usd_total,
        session_duration_seconds,
        tokens_total,
        peers_known,
        registry_ops_total,
        registry_errors_total,
        settlement_ops_total,
        settlement_errors_total,
        settled_micro_usd_total,
        uptime_seconds,
        requests_total,
        request_errors_total,
    }
}

// ── Convenience functions (call from other modules) ───────────────────────────

/// Lazily initialize metrics and return the handle cache. Auto-initialization ensures
/// that metric counters work even when called before explicit `init_registry()` startup.
fn handles() -> &'static MetricHandles {
    let (reg, handles) = REGISTRY_AND_HANDLES.get_or_init(|| {
        let reg = Registry::new();
        let handles = build_handles(&reg);
        (reg, handles)
    });
    let _reg = reg; // suppress unused warning
    handles
}

/// Increment total sessions opened for a given tier.
pub fn inc_sessions_opened(tier: &str) {
    handles().sessions_total.with_label_values(&[tier]).inc();
}

/// Increment closed sessions by outcome (completed, failed, cancelled).
pub fn inc_sessions_closed(outcome: &str) {
    handles()
        .sessions_closed_total
        .with_label_values(&[outcome])
        .inc();
}

/// Set the current number of active sessions.
pub fn set_sessions_active(count: f64) {
    handles().sessions_active.set(count);
}

/// Add earnings in micro-USDC for a tier.
pub fn inc_earnings_micro_usd(tier: &str, amount: u64) {
    handles()
        .earnings_micro_usd_total
        .with_label_values(&[tier])
        .inc_by(amount as f64);
}

/// Observe a session duration in seconds.
pub fn observe_session_duration(tier: &str, duration_secs: f64) {
    handles()
        .session_duration_seconds
        .with_label_values(&[tier])
        .observe(duration_secs);
}

/// Increment total tokens consumed.
pub fn inc_tokens(role: &str, count: u64) {
    handles()
        .tokens_total
        .with_label_values(&[role])
        .inc_by(count as f64);
}

/// Set the known peer count by role (peer/admin).
pub fn set_peers_known(role: &str, count: f64) {
    handles().peers_known.with_label_values(&[role]).set(count);
}

/// Increment registry operations counter.
pub fn inc_registry_op() {
    handles().registry_ops_total.inc();
}

/// Increment a specific registry error.
pub fn inc_registry_error(op: &str) {
    handles()
        .registry_errors_total
        .with_label_values(&[op])
        .inc();
}

/// Increment settlement operations by type (deposit/withdraw/settle).
pub fn inc_settlement_op(op: &str) {
    handles()
        .settlement_ops_total
        .with_label_values(&[op])
        .inc();
}

/// Increment settlement errors by type.
pub fn inc_settlement_error(op: &str) {
    handles()
        .settlement_errors_total
        .with_label_values(&[op])
        .inc();
}

/// Add settled earnings in micro-USDC.
pub fn inc_settled_micro_usd(amount: u64) {
    handles().settled_micro_usd_total.inc_by(amount as f64);
}

/// Update uptime gauge with seconds since start.
pub fn set_uptime(seconds: f64) {
    handles().uptime_seconds.set(seconds);
}

/// Increment total HTTP inference requests received.
pub fn inc_request() {
    handles().requests_total.inc();
}

/// Increment request errors by status code label (e.g. "500").
pub fn inc_request_error(status_code: &str) {
    handles()
        .request_errors_total
        .with_label_values(&[status_code])
        .inc();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_initializes_without_panicking() {
        let reg = init_registry();
        // Verify metrics were registered by gathering and checking count > 0.
        assert!(!reg.gather().is_empty());
    }

    #[test]
    fn handles_can_increment_without_panicking() {
        // Ensure registry is initialized (init_registry does this idempotently).
        init_registry();

        inc_sessions_opened("tee_verified");
        inc_sessions_closed("completed");
        set_sessions_active(5.0);
        inc_earnings_micro_usd("tee_verified", 1000);
        observe_session_duration("best_effort", 2.5);
        inc_tokens("input", 128);
        set_peers_known("peer", 3.0);
        inc_registry_op();
        inc_settlement_op("deposit");
        inc_settled_micro_usd(5000);

        // Verify counters incremented via encode output.
        let out = encode_prometheus();
        assert!(out.contains(r#"sparkl_sessions_total{tier="tee_verified"}"#));
    }

    #[test]
    fn metrics_encode_to_prometheus_format() {
        init_registry();
        let output = encode_prometheus();
        assert!(output.contains("sparkl_sessions_total"));
        assert!(output.starts_with("# HELP "));
        assert!(output.contains("# TYPE sparkl_sessions_total counter\n"));
    }
}

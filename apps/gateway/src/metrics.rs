//! Prometheus metrics.
//!
//! Hand-rolled rather than pulled from a crate: the surface we need is a few counters, a
//! gauge, and two histograms, and the hot path must not allocate to record a measurement.
//! Every recording is a relaxed atomic add on a pre-registered series.
//!
//! Exposed at `GET /metrics` in the Prometheus text exposition format.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// Histogram buckets for **gateway overhead**, in milliseconds.
///
/// Dense below 1ms because that is the number we are held to: Principle 1 sets a P99
/// budget of 1ms, so we need resolution to see it degrade, not just to see it breached.
const OVERHEAD_BUCKETS_MS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 5.0, 10.0, 50.0, 100.0];

/// Histogram buckets for **total request latency**, in milliseconds. Dominated by the
/// upstream provider, so the range runs to a minute.
const LATENCY_BUCKETS_MS: &[f64] = &[
    10.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0, 30_000.0, 60_000.0,
];

/// A labelled counter series.
#[derive(Default)]
struct CounterVec {
    series: RwLock<BTreeMap<String, AtomicU64>>,
}

impl CounterVec {
    fn inc(&self, labels: &str, by: u64) {
        // Fast path: the series almost always already exists.
        if let Ok(guard) = self.series.read() {
            if let Some(counter) = guard.get(labels) {
                counter.fetch_add(by, Ordering::Relaxed);
                return;
            }
        }
        if let Ok(mut guard) = self.series.write() {
            guard
                .entry(labels.to_string())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(by, Ordering::Relaxed);
        }
    }

    fn render(&self, name: &str, help: &str, out: &mut String) {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} counter");
        if let Ok(guard) = self.series.read() {
            for (labels, value) in guard.iter() {
                let v = value.load(Ordering::Relaxed);
                if labels.is_empty() {
                    let _ = writeln!(out, "{name} {v}");
                } else {
                    let _ = writeln!(out, "{name}{{{labels}}} {v}");
                }
            }
        }
    }

    fn total(&self) -> u64 {
        self.series
            .read()
            .map(|g| g.values().map(|v| v.load(Ordering::Relaxed)).sum())
            .unwrap_or(0)
    }
}

/// A fixed-bucket histogram. Bucket counts are cumulative at render time.
struct Histogram {
    buckets: &'static [f64],
    counts: Vec<AtomicU64>,
    sum_millis: AtomicU64, // fixed-point: microseconds, to stay integral
    count: AtomicU64,
}

impl Histogram {
    fn new(buckets: &'static [f64]) -> Histogram {
        Histogram {
            buckets,
            counts: (0..=buckets.len()).map(|_| AtomicU64::new(0)).collect(),
            sum_millis: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn observe(&self, value_ms: f64) {
        let index = self
            .buckets
            .iter()
            .position(|&edge| value_ms <= edge)
            .unwrap_or(self.buckets.len());
        if let Some(bucket) = self.counts.get(index) {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
        self.sum_millis
            .fetch_add((value_ms * 1_000.0).max(0.0) as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&self, name: &str, help: &str, out: &mut String) {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} histogram");
        let mut cumulative = 0u64;
        for (i, edge) in self.buckets.iter().enumerate() {
            cumulative += self.counts[i].load(Ordering::Relaxed);
            let _ = writeln!(out, "{name}_bucket{{le=\"{edge}\"}} {cumulative}");
        }
        cumulative += self.counts[self.buckets.len()].load(Ordering::Relaxed);
        let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {cumulative}");
        let sum_ms = self.sum_millis.load(Ordering::Relaxed) as f64 / 1_000.0;
        let _ = writeln!(out, "{name}_sum {sum_ms}");
        let _ = writeln!(out, "{name}_count {}", self.count.load(Ordering::Relaxed));
    }

    /// Approximate quantile from bucket counts. Good enough for alerting and for the
    /// admin dashboard; the exact figure lives in Prometheus itself.
    fn quantile(&self, q: f64) -> f64 {
        let total = self.count.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let target = (total as f64 * q).ceil() as u64;
        let mut cumulative = 0u64;
        for (i, edge) in self.buckets.iter().enumerate() {
            cumulative += self.counts[i].load(Ordering::Relaxed);
            if cumulative >= target {
                return *edge;
            }
        }
        f64::INFINITY
    }
}

/// A histogram broken out by label set.
///
/// Exists because the two unlabelled latency histograms could not answer the question the
/// Grafana dashboard's own panel description promised — "useful for spotting a slow
/// provider" — since nothing in the series said which provider a measurement came from.
/// Found in the enterprise readiness audit.
///
/// Cardinality is the risk with any labelled histogram. It is bounded here by construction:
/// the only labels used are provider id and model id, both drawn from the pricing table
/// rather than from anything a caller controls, so the series count is the size of the
/// model catalogue and cannot be inflated by traffic.
#[derive(Default)]
struct HistogramVec {
    series: RwLock<BTreeMap<String, Histogram>>,
}

impl HistogramVec {
    fn observe(&self, labels: &str, value_ms: f64, buckets: &'static [f64]) {
        if let Ok(guard) = self.series.read() {
            if let Some(histogram) = guard.get(labels) {
                histogram.observe(value_ms);
                return;
            }
        }
        if let Ok(mut guard) = self.series.write() {
            guard
                .entry(labels.to_string())
                .or_insert_with(|| Histogram::new(buckets))
                .observe(value_ms);
        }
    }

    fn render(&self, name: &str, help: &str, out: &mut String) {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} histogram");
        let Ok(guard) = self.series.read() else {
            return;
        };
        for (labels, histogram) in guard.iter() {
            let mut cumulative = 0u64;
            for (i, edge) in histogram.buckets.iter().enumerate() {
                cumulative += histogram.counts[i].load(Ordering::Relaxed);
                let _ = writeln!(out, "{name}_bucket{{{labels},le=\"{edge}\"}} {cumulative}");
            }
            cumulative += histogram.counts[histogram.buckets.len()].load(Ordering::Relaxed);
            let _ = writeln!(out, "{name}_bucket{{{labels},le=\"+Inf\"}} {cumulative}");
            let sum_ms = histogram.sum_millis.load(Ordering::Relaxed) as f64 / 1_000.0;
            let _ = writeln!(out, "{name}_sum{{{labels}}} {sum_ms}");
            let _ = writeln!(
                out,
                "{name}_count{{{labels}}} {}",
                histogram.count.load(Ordering::Relaxed)
            );
        }
    }

    /// Approximate quantile for one label set, for the admin console.
    fn quantile(&self, labels: &str, q: f64) -> Option<f64> {
        self.series
            .read()
            .ok()
            .and_then(|g| g.get(labels).map(|h| h.quantile(q)))
    }

    /// Every label set currently carrying observations.
    fn label_sets(&self) -> Vec<String> {
        self.series
            .read()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// A gauge: a value that goes up and down, unlike a counter.
#[derive(Default)]
struct GaugeVec {
    series: RwLock<BTreeMap<String, i64>>,
}

impl GaugeVec {
    fn set(&self, labels: &str, value: i64) {
        if let Ok(mut guard) = self.series.write() {
            guard.insert(labels.to_string(), value);
        }
    }

    fn render(&self, name: &str, help: &str, out: &mut String) {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} gauge");
        if let Ok(guard) = self.series.read() {
            for (labels, value) in guard.iter() {
                if labels.is_empty() {
                    let _ = writeln!(out, "{name} {value}");
                } else {
                    let _ = writeln!(out, "{name}{{{labels}}} {value}");
                }
            }
        }
    }
}

/// The gateway's metric registry. One instance, shared through `AppState`.
pub struct Metrics {
    requests_total: CounterVec,
    provider_errors_total: CounterVec,
    cache_events_total: CounterVec,
    routing_decisions_total: CounterVec,
    rate_limited_total: CounterVec,
    budget_blocked_total: CounterVec,
    usage_events_total: CounterVec,
    usage_events_lost_total: CounterVec,
    savings_micro_cents_total: CounterVec,
    circuit_state_changes_total: CounterVec,
    fallback_total: CounterVec,
    tokens_total: CounterVec,
    cache_bust_total: CounterVec,
    overhead: Histogram,
    latency: Histogram,
    provider_latency: HistogramVec,
    time_to_first_token: HistogramVec,
    reconciliation_drift: GaugeVec,
    store_health: GaugeVec,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Create an empty registry.
    pub fn new() -> Metrics {
        Metrics {
            requests_total: CounterVec::default(),
            provider_errors_total: CounterVec::default(),
            cache_events_total: CounterVec::default(),
            routing_decisions_total: CounterVec::default(),
            rate_limited_total: CounterVec::default(),
            budget_blocked_total: CounterVec::default(),
            usage_events_total: CounterVec::default(),
            usage_events_lost_total: CounterVec::default(),
            savings_micro_cents_total: CounterVec::default(),
            circuit_state_changes_total: CounterVec::default(),
            fallback_total: CounterVec::default(),
            tokens_total: CounterVec::default(),
            cache_bust_total: CounterVec::default(),
            overhead: Histogram::new(OVERHEAD_BUCKETS_MS),
            latency: Histogram::new(LATENCY_BUCKETS_MS),
            provider_latency: HistogramVec::default(),
            time_to_first_token: HistogramVec::default(),
            reconciliation_drift: GaugeVec::default(),
            store_health: GaugeVec::default(),
        }
    }

    /// Record a completed gateway request.
    pub fn record_request(&self, route: &str, status: u16) {
        self.requests_total
            .inc(&format!("route=\"{route}\",status=\"{status}\""), 1);
    }

    /// Record our own added latency, in milliseconds. Principle 1's scoreboard.
    pub fn record_overhead_ms(&self, ms: f64) {
        self.overhead.observe(ms);
    }

    /// Record end-to-end request latency, in milliseconds.
    pub fn record_latency_ms(&self, ms: f64) {
        self.latency.observe(ms);
    }

    /// Record a provider-level failure.
    pub fn record_provider_error(&self, provider: &str, kind: &str) {
        self.provider_errors_total
            .inc(&format!("provider=\"{provider}\",kind=\"{kind}\""), 1);
    }

    /// Record a cache lookup outcome: `exact`, `semantic`, `miss`, or `skipped`.
    pub fn record_cache(&self, outcome: &str) {
        self.cache_events_total
            .inc(&format!("outcome=\"{outcome}\""), 1);
    }

    /// Record why a request was routed the way it was.
    pub fn record_routing(&self, reason: &str, served_model: &str) {
        self.routing_decisions_total
            .inc(&format!("reason=\"{reason}\",model=\"{served_model}\""), 1);
    }

    /// Record a rate-limit rejection.
    pub fn record_rate_limited(&self, scope: &str) {
        self.rate_limited_total
            .inc(&format!("scope=\"{scope}\""), 1);
    }

    /// Record a budget rejection.
    pub fn record_budget_blocked(&self, scope: &str) {
        self.budget_blocked_total
            .inc(&format!("scope=\"{scope}\""), 1);
    }

    /// Record that a usage event was emitted. Principle 2 says this must equal the
    /// request count — the reconciliation job alerts when it does not.
    pub fn record_usage_event(&self) {
        self.usage_events_total.inc("", 1);
    }

    /// Accumulate gross savings delivered, in micro-cents.
    pub fn record_savings(&self, micro_cents: i64) {
        if micro_cents > 0 {
            self.savings_micro_cents_total.inc("", micro_cents as u64);
        }
    }

    /// Record a system prompt found to contain a cache-busting pattern (a fresh timestamp,
    /// UUID, or nonce that invalidates the provider's prefix cache on every turn). `kind`
    /// is the pattern that matched — see [`crate::engine::cache_bust`].
    pub fn record_cache_bust(&self, kind: &str) {
        self.cache_bust_total.inc(&format!("kind=\"{kind}\""), 1);
    }

    /// Record a circuit breaker transition.
    pub fn record_circuit_change(&self, provider: &str, state: &str) {
        self.circuit_state_changes_total
            .inc(&format!("provider=\"{provider}\",state=\"{state}\""), 1);
    }

    /// Record a usage event that could not be persisted.
    ///
    /// The counterpart to [`Metrics::record_usage_event`]. Together they make silent
    /// billing loss visible as a rate rather than only as a discrepancy between two other
    /// series — which is what an alert needs to fire on.
    pub fn record_usage_event_lost(&self, reason: &str) {
        self.usage_events_lost_total
            .inc(&format!("reason=\"{reason}\""), 1);
    }

    /// Record that a request was served by a fallback rather than the routed model.
    ///
    /// Distinct from `routing_decisions_total{reason="fallback"}`: this carries which
    /// provider was abandoned and which one answered, which is what an operator needs to
    /// tell "one provider is degraded" from "our routing is thrashing".
    pub fn record_fallback(&self, from_provider: &str, to_provider: &str) {
        self.fallback_total
            .inc(&format!("from=\"{from_provider}\",to=\"{to_provider}\""), 1);
    }

    /// Accumulate tokens served, split by direction and model.
    ///
    /// The denominator for cost-per-token dashboards, and the fastest way to spot a
    /// customer whose prompt size changed underneath them.
    pub fn record_tokens(&self, model: &str, input: u64, output: u64) {
        if input > 0 {
            self.tokens_total
                .inc(&format!("model=\"{model}\",direction=\"input\""), input);
        }
        if output > 0 {
            self.tokens_total
                .inc(&format!("model=\"{model}\",direction=\"output\""), output);
        }
    }

    /// Record how long one provider took to answer, by provider and model.
    ///
    /// This is the series that makes "which provider is slow right now" answerable. The
    /// unlabelled `aegis_request_latency_ms` cannot: it blends every provider together.
    pub fn record_provider_latency_ms(&self, provider: &str, model: &str, ms: f64) {
        self.provider_latency.observe(
            &format!("provider=\"{provider}\",model=\"{model}\""),
            ms,
            LATENCY_BUCKETS_MS,
        );
    }

    /// Record time to first token for a streaming request.
    ///
    /// The number a streaming client actually experiences as "responsiveness" — total
    /// latency says nothing about it, because a long answer and a slow start look
    /// identical in an end-to-end measurement.
    pub fn record_ttft_ms(&self, provider: &str, model: &str, ms: f64) {
        self.time_to_first_token.observe(
            &format!("provider=\"{provider}\",model=\"{model}\""),
            ms,
            LATENCY_BUCKETS_MS,
        );
    }

    /// Publish the number of organisations whose billing drift exceeded the threshold.
    pub fn record_reconciliation(&self, organisations_over_threshold: usize) {
        self.reconciliation_drift
            .set("", organisations_over_threshold as i64);
    }

    /// Publish a dependency's reachability: 1 up, 0 down.
    ///
    /// Scraped rather than only surfaced on `/health`, so an alert can fire on Redis being
    /// unreachable without anything having to poll a JSON endpoint and parse it.
    pub fn record_dependency_up(&self, dependency: &str, up: bool) {
        self.store_health
            .set(&format!("dependency=\"{dependency}\""), i64::from(up));
    }

    /// Approximate P99 latency for one provider and model, for the admin console.
    pub fn provider_latency_p99_ms(&self, provider: &str, model: &str) -> Option<f64> {
        self.provider_latency
            .quantile(&format!("provider=\"{provider}\",model=\"{model}\""), 0.99)
    }

    /// Every provider/model pair currently carrying latency observations.
    pub fn observed_provider_models(&self) -> Vec<String> {
        self.provider_latency.label_sets()
    }

    /// Total requests recorded. Used by the reconciliation self-check.
    pub fn total_requests(&self) -> u64 {
        self.requests_total.total()
    }

    /// Total usage events emitted.
    pub fn total_usage_events(&self) -> u64 {
        self.usage_events_total.total()
    }

    /// Approximate P99 gateway overhead, in milliseconds.
    pub fn overhead_p99_ms(&self) -> f64 {
        self.overhead.quantile(0.99)
    }

    /// Approximate P50 gateway overhead, in milliseconds.
    pub fn overhead_p50_ms(&self) -> f64 {
        self.overhead.quantile(0.50)
    }

    /// Render the whole registry in Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4096);
        self.requests_total.render(
            "aegis_requests_total",
            "Requests handled by the gateway",
            &mut out,
        );
        self.provider_errors_total.render(
            "aegis_provider_errors_total",
            "Upstream provider failures by provider and kind",
            &mut out,
        );
        self.cache_events_total.render(
            "aegis_cache_events_total",
            "Cache lookups by outcome",
            &mut out,
        );
        self.routing_decisions_total.render(
            "aegis_routing_decisions_total",
            "Routing decisions by reason and served model",
            &mut out,
        );
        self.rate_limited_total.render(
            "aegis_rate_limited_total",
            "Requests rejected by rate limiting",
            &mut out,
        );
        self.budget_blocked_total.render(
            "aegis_budget_blocked_total",
            "Requests rejected by budget enforcement",
            &mut out,
        );
        self.usage_events_total.render(
            "aegis_usage_events_total",
            "Usage events emitted (must track requests: Principle 2)",
            &mut out,
        );
        self.savings_micro_cents_total.render(
            "aegis_savings_micro_cents_total",
            "Gross savings delivered to customers, in micro-cents",
            &mut out,
        );
        self.circuit_state_changes_total.render(
            "aegis_circuit_state_changes_total",
            "Circuit breaker transitions by provider",
            &mut out,
        );
        self.overhead.render(
            "aegis_gateway_overhead_ms",
            "Latency added by Aegis itself, excluding the provider call",
            &mut out,
        );
        self.latency.render(
            "aegis_request_latency_ms",
            "End-to-end request latency",
            &mut out,
        );
        self.usage_events_lost_total.render(
            "aegis_usage_events_lost_total",
            "Usage events that could not be persisted — any non-zero rate is billable \
             traffic being served without a record",
            &mut out,
        );
        self.fallback_total.render(
            "aegis_fallback_total",
            "Requests served by a fallback provider, by provider abandoned and provider used",
            &mut out,
        );
        self.tokens_total.render(
            "aegis_tokens_total",
            "Tokens served by model and direction",
            &mut out,
        );
        self.cache_bust_total.render(
            "aegis_cache_bust_total",
            "System prompts found to contain a provider-prefix-cache-busting pattern, by kind",
            &mut out,
        );
        self.provider_latency.render(
            "aegis_provider_latency_ms",
            "Upstream provider latency by provider and model",
            &mut out,
        );
        self.time_to_first_token.render(
            "aegis_time_to_first_token_ms",
            "Time to first streamed token by provider and model",
            &mut out,
        );
        self.reconciliation_drift.render(
            "aegis_reconciliation_orgs_over_threshold",
            "Organisations whose billing drift exceeded the alert threshold on the last run",
            &mut out,
        );
        self.store_health.render(
            "aegis_dependency_up",
            "Dependency reachability: 1 up, 0 down",
            &mut out,
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_per_label_set() {
        let m = Metrics::new();
        m.record_request("/v1/chat/completions", 200);
        m.record_request("/v1/chat/completions", 200);
        m.record_request("/v1/chat/completions", 429);
        let out = m.render();
        assert!(
            out.contains(r#"aegis_requests_total{route="/v1/chat/completions",status="200"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"aegis_requests_total{route="/v1/chat/completions",status="429"} 1"#),
            "{out}"
        );
    }

    #[test]
    fn histogram_buckets_are_cumulative() {
        let m = Metrics::new();
        m.record_overhead_ms(0.04);
        m.record_overhead_ms(0.4);
        m.record_overhead_ms(20.0);
        let out = m.render();
        // 0.04 falls in le=0.05; cumulative at le=0.5 must include both sub-ms samples.
        assert!(
            out.contains(r#"aegis_gateway_overhead_ms_bucket{le="0.05"} 1"#),
            "{out}"
        );
        assert!(
            out.contains(r#"aegis_gateway_overhead_ms_bucket{le="0.5"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"aegis_gateway_overhead_ms_bucket{le="+Inf"} 3"#),
            "{out}"
        );
        assert!(out.contains("aegis_gateway_overhead_ms_count 3"), "{out}");
    }

    #[test]
    fn quantiles_track_the_latency_budget() {
        let m = Metrics::new();
        // 99 fast requests and one slow one: P99 must reflect the fast bulk, P100 the tail.
        for _ in 0..99 {
            m.record_overhead_ms(0.2);
        }
        m.record_overhead_ms(50.0);
        assert!(
            m.overhead_p50_ms() <= 0.25,
            "p50 was {}",
            m.overhead_p50_ms()
        );
        assert!(
            m.overhead_p99_ms() <= 1.0,
            "p99 was {}",
            m.overhead_p99_ms()
        );
    }

    #[test]
    fn empty_histogram_reports_zero_not_nan() {
        let m = Metrics::new();
        assert_eq!(m.overhead_p99_ms(), 0.0);
    }

    #[test]
    fn savings_counter_ignores_negative_values() {
        let m = Metrics::new();
        m.record_savings(1_000);
        m.record_savings(-500); // a routing decision that cost more must not subtract
        let out = m.render();
        assert!(
            out.contains("aegis_savings_micro_cents_total 1000"),
            "{out}"
        );
    }

    #[test]
    fn usage_events_can_be_compared_against_requests() {
        // Principle 2: every request must produce exactly one usage event.
        let m = Metrics::new();
        for _ in 0..5 {
            m.record_request("/v1/chat/completions", 200);
            m.record_usage_event();
        }
        assert_eq!(m.total_requests(), m.total_usage_events());
    }

    #[test]
    fn exposition_format_is_well_formed() {
        let m = Metrics::new();
        m.record_request("/health", 200);
        m.record_cache("exact");
        m.record_routing("complexity", "gpt-4o-mini");
        let out = m.render();
        for line in out.lines() {
            assert!(
                line.starts_with('#') || line.split(' ').count() == 2,
                "malformed exposition line: {line}"
            );
        }
        assert!(out.contains("# TYPE aegis_requests_total counter"));
        assert!(out.contains("# TYPE aegis_gateway_overhead_ms histogram"));
    }
}

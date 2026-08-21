//! Billing reconciliation.
//!
//! `MASTER_BUILD.md` Part 13 item 1 requires a daily job comparing the Redis counters
//! against the PostgreSQL rollups, alerting on drift over $0.01.
//!
//! # Why drift happens, and why it must be watched
//!
//! The counters are updated on the hot path and the records are written asynchronously, so
//! the two are *expected* to differ momentarily. Persistent drift is different: it means
//! either events are being emitted but not persisted (we are under-billing and losing
//! revenue) or counters are being incremented without a matching record (we are
//! over-billing, which is far worse). Both are invisible without this job — every
//! individual request looks fine.
//!
//! The counters are never "fixed" to match. PostgreSQL is authoritative; a mismatch is
//! reported, not papered over, because silently rewriting a billing figure to match
//! whichever source we trusted last is how a reconciliation job becomes the bug.

use crate::db::repo;
use crate::error::Result;
use crate::metering::usage;
use crate::money::MicroCents;
use crate::AppState;
use chrono::{Datelike, TimeZone, Utc};
use serde::Serialize;
use std::time::Duration;
use uuid::Uuid;

/// Drift beyond this is reported. One cent, per Part 13.
pub const DRIFT_THRESHOLD: MicroCents = MicroCents(10_000);

/// How often reconciliation runs.
pub const INTERVAL: Duration = Duration::from_secs(24 * 3_600);

/// The result of reconciling one organisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Reconciliation {
    pub org_id: Uuid,
    /// Month-to-date spend according to the Redis counter.
    pub counter_spend_mc: i64,
    /// Month-to-date spend according to `usage_records`.
    pub recorded_spend_mc: i64,
    /// Signed difference: positive means the counter is ahead.
    pub drift_mc: i64,
    pub requests_recorded: i64,
}

impl Reconciliation {
    /// True when the drift exceeds the alert threshold.
    pub fn is_significant(&self) -> bool {
        self.drift_mc.abs() > DRIFT_THRESHOLD.as_i64()
    }

    /// Which direction the drift runs, in business terms.
    ///
    /// Naming this explicitly matters: an operator reading an alert at 3am needs to know
    /// immediately whether customers are being over-billed.
    pub fn direction(&self) -> &'static str {
        if self.drift_mc > 0 {
            // Counter ahead of records: budget enforcement is stricter than reality.
            "counter_ahead"
        } else if self.drift_mc < 0 {
            // Records ahead of counter: requests were served that budgets did not see.
            "records_ahead"
        } else {
            "exact"
        }
    }
}

/// Reconcile one organisation's month-to-date figures.
pub async fn reconcile_org(state: &AppState, org_id: Uuid) -> Result<Reconciliation> {
    let counter_spend = usage::current_spend(state.store.as_ref(), org_id).await;

    let now = Utc::now();
    let month_start = Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now);

    let summary = repo::usage_summary(state.db()?, org_id, month_start, now).await?;

    Ok(Reconciliation {
        org_id,
        counter_spend_mc: counter_spend.as_i64(),
        recorded_spend_mc: summary.actual_cost_mc,
        drift_mc: counter_spend.as_i64() - summary.actual_cost_mc,
        requests_recorded: summary.requests,
    })
}

/// Compare emitted requests against persisted usage events.
///
/// Principle 2 says every request produces exactly one usage record. This is the
/// self-check on that claim, using the process-local metric counters.
pub fn check_metering_completeness(state: &AppState) -> Option<String> {
    let requests = state.metrics.total_requests();
    let events = state.metrics.total_usage_events();

    if requests == events {
        return None;
    }
    Some(format!(
        "metering gap: {requests} requests handled but {events} usage events emitted \
         (difference {})",
        requests.abs_diff(events)
    ))
}

/// Run reconciliation on a schedule.
pub async fn run(state: AppState, org_ids: Vec<Uuid>) {
    let mut ticker = tokio::time::interval(INTERVAL);
    loop {
        ticker.tick().await;

        if let Some(gap) = check_metering_completeness(&state) {
            tracing::error!(gap, "metering completeness check failed");
        }

        for org_id in &org_ids {
            match reconcile_org(&state, *org_id).await {
                Ok(result) if result.is_significant() => {
                    // Error level, not warn: this is a billing-accuracy incident.
                    tracing::error!(
                        org_id = %result.org_id,
                        drift_mc = result.drift_mc,
                        direction = result.direction(),
                        counter = result.counter_spend_mc,
                        recorded = result.recorded_spend_mc,
                        "billing drift exceeds threshold"
                    );
                }
                Ok(result) => tracing::debug!(
                    org_id = %result.org_id,
                    drift_mc = result.drift_mc,
                    "reconciliation clean"
                ),
                Err(e) => tracing::error!(org_id = %org_id, error = %e, "reconciliation failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reconciliation(drift: i64) -> Reconciliation {
        Reconciliation {
            org_id: Uuid::new_v4(),
            counter_spend_mc: 1_000_000 + drift,
            recorded_spend_mc: 1_000_000,
            drift_mc: drift,
            requests_recorded: 100,
        }
    }

    #[test]
    fn exact_agreement_is_not_significant() {
        let result = reconciliation(0);
        assert!(!result.is_significant());
        assert_eq!(result.direction(), "exact");
    }

    #[test]
    fn drift_under_a_cent_is_tolerated() {
        // Per-request rounding and in-flight events legitimately produce small drift.
        assert!(!reconciliation(9_999).is_significant());
        assert!(!reconciliation(-9_999).is_significant());
        assert!(!reconciliation(10_000).is_significant());
    }

    #[test]
    fn drift_over_a_cent_is_significant_in_both_directions() {
        assert!(reconciliation(10_001).is_significant());
        assert!(reconciliation(-10_001).is_significant());
    }

    #[test]
    fn drift_direction_is_named_for_the_operator() {
        // An alert at 3am must say which way the error runs.
        assert_eq!(reconciliation(50_000).direction(), "counter_ahead");
        assert_eq!(reconciliation(-50_000).direction(), "records_ahead");
    }

    #[test]
    fn the_threshold_matches_the_specification() {
        // Part 13 item 1: alert on drift over $0.01.
        assert_eq!(DRIFT_THRESHOLD, MicroCents(10_000));
        assert_eq!(DRIFT_THRESHOLD.to_usd_string(), "$0.0100");
    }

    #[test]
    fn metering_completeness_passes_when_counts_agree() {
        let state = AppState::for_tests();
        for _ in 0..10 {
            state.metrics.record_request("/v1/chat/completions", 200);
            state.metrics.record_usage_event();
        }
        assert!(check_metering_completeness(&state).is_none());
    }

    #[test]
    fn metering_completeness_detects_a_missing_usage_event() {
        // The exact failure Principle 2 forbids: a request that produced no record.
        let state = AppState::for_tests();
        for _ in 0..10 {
            state.metrics.record_request("/v1/chat/completions", 200);
        }
        for _ in 0..9 {
            state.metrics.record_usage_event();
        }

        let gap = check_metering_completeness(&state).expect("a gap should be reported");
        assert!(gap.contains("10 requests"), "{gap}");
        assert!(gap.contains("9 usage events"), "{gap}");
        assert!(gap.contains("difference 1"), "{gap}");
    }

    #[test]
    fn metering_completeness_detects_a_surplus_event() {
        // Double-emitting is over-billing, which is worse than under-billing.
        let state = AppState::for_tests();
        state.metrics.record_request("/v1/chat/completions", 200);
        state.metrics.record_usage_event();
        state.metrics.record_usage_event();
        assert!(check_metering_completeness(&state).is_some());
    }

    #[test]
    fn a_fresh_process_reports_no_gap() {
        assert!(check_metering_completeness(&AppState::for_tests()).is_none());
    }

    #[test]
    fn reconciliation_serializes_for_the_admin_console() {
        let json = serde_json::to_value(reconciliation(12_345)).unwrap();
        assert_eq!(json["drift_mc"], 12_345);
        assert!(json["org_id"].is_string());
    }
}

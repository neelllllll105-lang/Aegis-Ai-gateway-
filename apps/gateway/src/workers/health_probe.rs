//! Dependency reachability, published as a scrapeable metric.
//!
//! `/health` already reports per-dependency status, but only to whoever asks it. Alerting
//! on "Redis is unreachable" through that endpoint means something has to poll it, parse
//! the JSON, and decide — which is a second monitoring system to build and keep working.
//!
//! This worker probes the same dependencies on a timer and writes the result into
//! `aegis_dependency_up`, so the existing Prometheus scrape already carries it and an
//! alerting rule is one expression rather than a new component. Found in the enterprise
//! readiness audit: no alerting pipeline existed at all, and the metrics that did exist
//! could not express dependency health.

use crate::AppState;
use std::time::Duration;

/// How often each dependency is probed.
///
/// Fifteen seconds: fast enough that a Prometheus rule with a one-minute `for` clause has
/// four observations to work with, slow enough that the probe is not itself load.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(15);

/// Probe dependencies forever.
pub async fn run(state: AppState) {
    let mut ticker = tokio::time::interval(PROBE_INTERVAL);
    loop {
        ticker.tick().await;
        probe_once(&state).await;
    }
}

/// One probe of every dependency. Separated so it can be tested and triggered directly.
pub async fn probe_once(state: &AppState) {
    let store_up = state.store.ping().await.is_ok();
    state.metrics.record_dependency_up("store", store_up);
    if !store_up {
        // Warn, not error: the request path fails open on a store outage by design, so
        // this is degraded service rather than an outage. The alert rule decides urgency.
        tracing::warn!(
            backend = state.store.backend_name(),
            "store unreachable; rate limits and budgets are failing open"
        );
    }

    if let Some(pool) = state.db.as_ref() {
        let db_up = sqlx::query("SELECT 1").execute(pool).await.is_ok();
        state.metrics.record_dependency_up("database", db_up);
        if !db_up {
            // Error, not warn: without the database, usage events cannot be persisted, and
            // persisted usage is what invoices are built from.
            tracing::error!("database unreachable; usage events cannot be persisted");
        }
    }

    if let Some(replica) = state.db_replica.as_ref() {
        let replica_up = sqlx::query("SELECT 1").execute(replica).await.is_ok();
        state
            .metrics
            .record_dependency_up("database_replica", replica_up);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;

    #[tokio::test]
    async fn a_reachable_store_is_reported_up() {
        let state = AppState::for_tests();
        probe_once(&state).await;
        assert!(
            state.metrics.render().contains("dependency=\"store\"} 1"),
            "an in-memory store always pings successfully, so it must report up"
        );
    }

    #[tokio::test]
    async fn no_database_configured_publishes_no_database_series() {
        // Absence must not read as "down". A self-hosted instance running without a
        // replica should not page anyone about a replica it never had.
        let state = AppState::for_tests();
        probe_once(&state).await;
        let rendered = state.metrics.render();
        assert!(!rendered.contains("dependency=\"database\""));
        assert!(!rendered.contains("dependency=\"database_replica\""));
    }
}

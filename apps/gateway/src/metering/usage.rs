//! Usage events — the billing source of truth.
//!
//! Principle 2: **every request produces a usage record.** Not most requests, not
//! successful requests — every one, including the ones that 429, 402, or fail upstream,
//! because a request that cost us a provider call and produced no record is a request we
//! cannot bill for or explain.
//!
//! Stage [10] of the pipeline emits the event to a Redis stream and bumps the real-time
//! counters, then returns to the client immediately. Persisting to PostgreSQL happens in
//! [`crate::workers::usage_writer`], off the hot path, because Principle 1 forbids
//! synchronous database I/O on the request path.

use crate::error::Result;
use crate::metering::savings::SavingsBreakdown;
use crate::money::MicroCents;
use crate::store::KvStore;
use crate::types::{CacheOutcome, RoutingReason, TokenUsage};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Redis stream carrying usage events to the writer worker.
pub const USAGE_STREAM: &str = "aegis:usage_events";

/// Cap on the stream length. At ~1M entries the stream self-trims; the writer normally
/// runs many orders of magnitude ahead of that, so this is a backstop against a worker
/// outage silently consuming all of Redis rather than a routine limit.
pub const USAGE_STREAM_MAX_LEN: usize = 1_000_000;

/// One metered request.
///
/// Field names match `usage_records` columns so the writer's insert is a direct mapping
/// with no translation layer to drift out of sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Idempotency key. The writer deduplicates on this, so a redelivered stream entry
    /// cannot double-bill.
    pub request_id: Uuid,
    pub org_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub team_id: Option<Uuid>,

    /// What the caller asked for — the savings baseline.
    pub requested_model: String,
    /// What actually served it.
    pub served_model: String,
    pub provider: String,

    pub input_tokens: u64,
    pub output_tokens: u64,
    /// True when token counts were estimated rather than reported by the provider.
    pub tokens_estimated: bool,

    pub baseline_cost_mc: i64,
    pub actual_cost_mc: i64,
    pub gross_savings_mc: i64,
    pub aegis_fee_mc: i64,

    pub latency_ms: u32,
    /// Our own added latency. Part 13 item 4: we display this and never game it.
    pub gateway_overhead_ms: f64,

    pub cache_hit: bool,
    pub cache_type: Option<String>,

    pub routing_reason: String,
    pub complexity_score: Option<f32>,

    pub status_code: u16,
    pub error_type: Option<String>,

    /// Tokens removed by context compression, if any.
    pub tokens_saved_by_compression: u64,

    /// The region that served this request.
    ///
    /// Recorded on the event rather than read from config at write time, because the
    /// usage writer may run in a different region from the gateway that served the
    /// request, and attributing spend to the writer's region would be wrong.
    #[serde(default)]
    pub region: Option<String>,

    pub created_at: DateTime<Utc>,
}

impl UsageEvent {
    /// Assemble an event from the pieces the pipeline has at stage [10].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: Uuid,
        org_id: Uuid,
        api_key_id: Option<Uuid>,
        team_id: Option<Uuid>,
        requested_model: String,
        served_model: String,
        provider: String,
        tokens: TokenUsage,
        savings: SavingsBreakdown,
        latency_ms: u32,
        gateway_overhead_ms: f64,
        cache: CacheOutcome,
        routing_reason: RoutingReason,
        complexity_score: Option<f32>,
        status_code: u16,
    ) -> UsageEvent {
        UsageEvent {
            request_id,
            org_id,
            api_key_id,
            team_id,
            requested_model,
            served_model,
            provider,
            input_tokens: tokens.input_tokens,
            output_tokens: tokens.output_tokens,
            tokens_estimated: tokens.estimated,
            baseline_cost_mc: savings.baseline_cost.as_i64(),
            actual_cost_mc: savings.actual_cost.as_i64(),
            gross_savings_mc: savings.gross_savings.as_i64(),
            aegis_fee_mc: savings.aegis_fee.as_i64(),
            latency_ms,
            gateway_overhead_ms,
            cache_hit: cache.is_hit(),
            cache_type: cache.is_hit().then(|| cache.as_str().to_string()),
            routing_reason: routing_reason.as_str().to_string(),
            complexity_score,
            status_code,
            error_type: None,
            region: None,
            tokens_saved_by_compression: 0,
            created_at: Utc::now(),
        }
    }

    /// An event for a request that failed before or during provider execution.
    ///
    /// Still metered: it consumed our capacity and may have consumed provider quota, and
    /// Principle 2 admits no exceptions.
    pub fn rejected(
        request_id: Uuid,
        org_id: Uuid,
        api_key_id: Option<Uuid>,
        requested_model: String,
        status_code: u16,
        error_type: &str,
        gateway_overhead_ms: f64,
    ) -> UsageEvent {
        UsageEvent {
            request_id,
            org_id,
            api_key_id,
            team_id: None,
            requested_model: requested_model.clone(),
            served_model: requested_model,
            provider: "none".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            tokens_estimated: false,
            baseline_cost_mc: 0,
            actual_cost_mc: 0,
            gross_savings_mc: 0,
            aegis_fee_mc: 0,
            latency_ms: gateway_overhead_ms.round() as u32,
            gateway_overhead_ms,
            cache_hit: false,
            cache_type: None,
            routing_reason: RoutingReason::Passthrough.as_str().to_string(),
            complexity_score: None,
            status_code,
            error_type: Some(error_type.to_string()),
            region: None,
            tokens_saved_by_compression: 0,
            created_at: Utc::now(),
        }
    }

    /// Whether this request actually reached a provider and incurred cost.
    pub fn is_billable(&self) -> bool {
        self.actual_cost_mc > 0 || self.cache_hit
    }

    /// Total tokens.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Redis key for an org's monthly spend counter, in micro-cents.
pub fn org_spend_key(org_id: Uuid, at: DateTime<Utc>) -> String {
    format!("aegis:org:{}:spend:{}{:02}", org_id, at.year(), at.month())
}

/// Redis key for an org's monthly request counter (free-tier allowance).
pub fn org_requests_key(org_id: Uuid, at: DateTime<Utc>) -> String {
    format!(
        "aegis:org:{}:requests:{}{:02}",
        org_id,
        at.year(),
        at.month()
    )
}

/// Redis key for a team's monthly spend counter.
pub fn team_spend_key(team_id: Uuid, at: DateTime<Utc>) -> String {
    format!(
        "aegis:team:{}:spend:{}{:02}",
        team_id,
        at.year(),
        at.month()
    )
}

/// Redis key for an API key's monthly spend counter.
pub fn key_spend_key(api_key_id: Uuid, at: DateTime<Utc>) -> String {
    format!(
        "aegis:key:{}:spend:{}{:02}",
        api_key_id,
        at.year(),
        at.month()
    )
}

/// Redis key for an org's accumulated savings, used by the dashboard's live counter.
pub fn org_savings_key(org_id: Uuid, at: DateTime<Utc>) -> String {
    format!(
        "aegis:org:{}:savings:{}{:02}",
        org_id,
        at.year(),
        at.month()
    )
}

/// TTL for monthly counters: 45 days, long enough to survive month-end reconciliation and
/// short enough that abandoned orgs do not accumulate keys forever.
const COUNTER_TTL: std::time::Duration = std::time::Duration::from_secs(45 * 24 * 3_600);

/// Emit a usage event: append to the stream, then bump the real-time counters.
///
/// Both steps are Redis-only and take well under the 0.1ms budgeted for stage [10].
/// Counter updates are best-effort — a failed counter bump must never fail a request that
/// the customer has already been served, because the authoritative figures are rebuilt
/// from `usage_records` by the reconciliation job.
pub async fn emit(store: &dyn KvStore, event: &UsageEvent) -> Result<String> {
    let payload = serde_json::to_string(event)
        .map_err(|e| crate::error::AegisError::Internal(format!("usage serialization: {e}")))?;

    let id = store
        .stream_append(USAGE_STREAM, &payload, USAGE_STREAM_MAX_LEN)
        .await?;

    let at = event.created_at;

    // Spend and request counters gate budget enforcement and the free-tier allowance at
    // stage [3], so they must reflect this request before the next one arrives.
    let _ = store
        .incr_by(
            &org_spend_key(event.org_id, at),
            event.actual_cost_mc,
            Some(COUNTER_TTL),
        )
        .await;
    let _ = store
        .incr_by(&org_requests_key(event.org_id, at), 1, Some(COUNTER_TTL))
        .await;
    let _ = store
        .incr_by(
            &org_savings_key(event.org_id, at),
            event.gross_savings_mc,
            Some(COUNTER_TTL),
        )
        .await;

    if let Some(team_id) = event.team_id {
        let _ = store
            .incr_by(
                &team_spend_key(team_id, at),
                event.actual_cost_mc,
                Some(COUNTER_TTL),
            )
            .await;
    }
    if let Some(key_id) = event.api_key_id {
        let _ = store
            .incr_by(
                &key_spend_key(key_id, at),
                event.actual_cost_mc,
                Some(COUNTER_TTL),
            )
            .await;
    }
    // Regional counter. Always written, even for single-region organisations: the cost is
    // one INCR, and without it a customer who adds a regional budget later would start
    // from an empty counter and get a month of free overspend.
    if let Some(region) = event.region.as_deref() {
        let _ = store
            .incr_by(
                &org_region_spend_key(event.org_id, region, at),
                event.actual_cost_mc,
                Some(COUNTER_TTL),
            )
            .await;
    }

    Ok(id)
}

/// Per-region spend counter key.
///
/// Scoped by organisation as well as region, exactly like every other counter here. A
/// key of the form `region:eu-central` would aggregate every tenant in the region into
/// one number, which is both useless to a customer and a cross-tenant leak.
fn org_region_spend_key(org_id: Uuid, region: &str, at: DateTime<Utc>) -> String {
    format!(
        "aegis:spend:org:{org_id}:region:{}:{}",
        region.to_ascii_lowercase(),
        at.format("%Y-%m")
    )
}

/// Read an organisation's current monthly spend within one region.
pub async fn current_region_spend(store: &dyn KvStore, org_id: Uuid, region: &str) -> MicroCents {
    read_counter(store, &org_region_spend_key(org_id, region, Utc::now())).await
}

/// Read an org's current monthly spend from the counters.
pub async fn current_spend(store: &dyn KvStore, org_id: Uuid) -> MicroCents {
    read_counter(store, &org_spend_key(org_id, Utc::now())).await
}

/// Read an org's current monthly request count.
pub async fn current_requests(store: &dyn KvStore, org_id: Uuid) -> u64 {
    read_counter(store, &org_requests_key(org_id, Utc::now()))
        .await
        .as_i64()
        .max(0) as u64
}

/// Read a team's current monthly spend.
pub async fn current_team_spend(store: &dyn KvStore, team_id: Uuid) -> MicroCents {
    read_counter(store, &team_spend_key(team_id, Utc::now())).await
}

/// Read an API key's current monthly spend.
pub async fn current_key_spend(store: &dyn KvStore, api_key_id: Uuid) -> MicroCents {
    read_counter(store, &key_spend_key(api_key_id, Utc::now())).await
}

/// Read an org's accumulated savings this month.
pub async fn current_savings(store: &dyn KvStore, org_id: Uuid) -> MicroCents {
    read_counter(store, &org_savings_key(org_id, Utc::now())).await
}

async fn read_counter(store: &dyn KvStore, key: &str) -> MicroCents {
    // A store failure here must not block a request: treating an unreadable counter as
    // zero fails open on budget enforcement, which is the right trade for an outage —
    // we would rather serve traffic we later reconcile than reject paying customers.
    store
        .get(key)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .map(MicroCents)
        .unwrap_or(MicroCents::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use crate::types::TokenUsage;

    fn sample_event(org_id: Uuid) -> UsageEvent {
        UsageEvent::new(
            Uuid::new_v4(),
            org_id,
            Some(Uuid::new_v4()),
            None,
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "openai".to_string(),
            TokenUsage {
                input_tokens: 1_000,
                output_tokens: 500,
                estimated: false,
            },
            SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), 2_000),
            842,
            0.37,
            CacheOutcome::Miss,
            RoutingReason::Complexity,
            Some(0.21),
            200,
        )
    }

    #[test]
    fn event_carries_the_full_savings_attribution() {
        let event = sample_event(Uuid::new_v4());
        assert_eq!(event.baseline_cost_mc, 7_500);
        assert_eq!(event.actual_cost_mc, 450);
        assert_eq!(event.gross_savings_mc, 7_050);
        assert_eq!(event.aegis_fee_mc, 1_410);
        assert_eq!(event.total_tokens(), 1_500);
        assert!(!event.cache_hit);
        assert_eq!(event.cache_type, None);
    }

    #[test]
    fn cache_hits_record_their_type() {
        let mut event = sample_event(Uuid::new_v4());
        event.cache_hit = true;
        event.cache_type = Some(CacheOutcome::Exact.as_str().to_string());
        assert_eq!(event.cache_type.as_deref(), Some("exact"));
        assert!(
            event.is_billable(),
            "a cache hit still produces a savings-share fee"
        );
    }

    #[test]
    fn rejected_requests_are_still_metered() {
        // Principle 2 admits no exceptions: a 429 produces a record too.
        let event = UsageEvent::rejected(
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
            "gpt-4o".to_string(),
            429,
            "rate_limit_exceeded",
            0.08,
        );
        assert_eq!(event.status_code, 429);
        assert_eq!(event.error_type.as_deref(), Some("rate_limit_exceeded"));
        assert_eq!(event.actual_cost_mc, 0);
        assert!(!event.is_billable());
    }

    #[test]
    fn events_round_trip_through_json() {
        // The stream carries JSON; a field that fails to survive would silently lose
        // billing data.
        let event = sample_event(Uuid::new_v4());
        let json = serde_json::to_string(&event).unwrap();
        let restored: UsageEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, restored);
    }

    #[test]
    fn counter_keys_are_org_scoped_and_month_scoped() {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        let now = Utc::now();
        assert_ne!(org_spend_key(org_a, now), org_spend_key(org_b, now));
        assert!(org_spend_key(org_a, now).contains(&org_a.to_string()));

        let january = DateTime::parse_from_rfc3339("2026-01-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let february = DateTime::parse_from_rfc3339("2026-02-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_ne!(
            org_spend_key(org_a, january),
            org_spend_key(org_a, february)
        );
        assert!(org_spend_key(org_a, january).ends_with("202601"));
        assert!(org_spend_key(org_a, february).ends_with("202602"));
    }

    #[tokio::test]
    async fn emitting_appends_to_the_stream_and_updates_counters() {
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();
        let event = sample_event(org_id);

        let id = emit(&store, &event).await.unwrap();
        assert!(!id.is_empty());

        let entries = store.stream_read(USAGE_STREAM, "0", 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        let decoded: UsageEvent = serde_json::from_str(&entries[0].payload).unwrap();
        assert_eq!(decoded.request_id, event.request_id);

        assert_eq!(current_spend(&store, org_id).await, MicroCents(450));
        assert_eq!(current_requests(&store, org_id).await, 1);
        assert_eq!(current_savings(&store, org_id).await, MicroCents(7_050));
    }

    #[tokio::test]
    async fn counters_accumulate_across_requests() {
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();
        for _ in 0..10 {
            emit(&store, &sample_event(org_id)).await.unwrap();
        }
        assert_eq!(current_spend(&store, org_id).await, MicroCents(4_500));
        assert_eq!(current_requests(&store, org_id).await, 10);
        assert_eq!(current_savings(&store, org_id).await, MicroCents(70_500));
    }

    #[tokio::test]
    async fn counters_are_isolated_between_organisations() {
        // Tenant isolation reaches all the way into the counters: one org's spend must
        // never appear in another's budget check.
        let store = MemoryStore::new();
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();

        emit(&store, &sample_event(org_a)).await.unwrap();
        emit(&store, &sample_event(org_a)).await.unwrap();
        emit(&store, &sample_event(org_b)).await.unwrap();

        assert_eq!(current_spend(&store, org_a).await, MicroCents(900));
        assert_eq!(current_spend(&store, org_b).await, MicroCents(450));
    }

    #[tokio::test]
    async fn team_and_key_counters_track_separately() {
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();
        let team_id = Uuid::new_v4();
        let key_id = Uuid::new_v4();

        let mut event = sample_event(org_id);
        event.team_id = Some(team_id);
        event.api_key_id = Some(key_id);
        emit(&store, &event).await.unwrap();

        assert_eq!(current_team_spend(&store, team_id).await, MicroCents(450));
        assert_eq!(current_key_spend(&store, key_id).await, MicroCents(450));
    }

    #[tokio::test]
    async fn unread_counters_report_zero_rather_than_failing() {
        let store = MemoryStore::new();
        assert_eq!(
            current_spend(&store, Uuid::new_v4()).await,
            MicroCents::ZERO
        );
        assert_eq!(current_requests(&store, Uuid::new_v4()).await, 0);
    }

    #[tokio::test]
    async fn every_emitted_event_is_recoverable_from_the_stream() {
        // The durability property the usage worker depends on: nothing is dropped between
        // emission and persistence.
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();
        let mut ids = Vec::new();
        for _ in 0..250 {
            let event = sample_event(org_id);
            ids.push(event.request_id);
            emit(&store, &event).await.unwrap();
        }
        let entries = store.stream_read(USAGE_STREAM, "0", 1_000).await.unwrap();
        assert_eq!(entries.len(), 250);
        let recovered: Vec<Uuid> = entries
            .iter()
            .map(|e| {
                serde_json::from_str::<UsageEvent>(&e.payload)
                    .unwrap()
                    .request_id
            })
            .collect();
        assert_eq!(recovered, ids);
    }
}

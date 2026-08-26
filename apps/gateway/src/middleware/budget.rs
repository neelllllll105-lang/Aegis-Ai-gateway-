//! Budget enforcement — pipeline stage [3].
//!
//! Rejects with **402 Payment Required** when a hard limit is exceeded. Budget: 0.1ms, so
//! this is counter operations only — the authoritative figures are rebuilt from
//! `usage_records` by the reconciliation worker.
//!
//! # Reserve, then true up
//!
//! A budget check that only *reads* a counter is not enforcement. The cost of a request is
//! not known until after the provider answers, so a read-compare-then-charge-later design
//! leaves a window in which every concurrent request sees the same pre-request spend and
//! every one of them is admitted. That was measured, not theorised: 20 simultaneous
//! requests against a $1.00 hard limit with $0.05 of headroom admitted 2-5 of them, 8 runs
//! out of 8 (enterprise readiness audit, 2026-08-25).
//!
//! So the check does not read. It **reserves**:
//!
//! 1. [`check_and_reserve`] atomically adds a *projected* cost to every applicable spend
//!    counter and inspects the value that increment returned. One round trip, one atomic
//!    operation per scope — the same primitive the rate limiter's proven-atomic Lua script
//!    relies on.
//! 2. If any scope would breach its limit, every increment already applied is rolled back
//!    and the request is refused. A refused request leaves the counters exactly as it
//!    found them.
//! 3. When the request finishes, [`Reservation::commit`] hands the held projection to
//!    [`crate::metering::usage::emit`], which applies the *difference* between the real
//!    cost and the projection so the counter ends up holding the true figure. Exactly one
//!    of the two moves the counter — see [`Reservation::commit`] for why that matters.
//!
//! The reservation is therefore live for exactly the duration of the request, which is the
//! window a competing request needs to see it in.
//!
//! # Hard limits block; soft limits alert
//!
//! `budgets.hard_limit` is honoured rather than ignored. A hard limit refuses a request
//! that *would* take the org past the line — that is what "hard" has to mean, or the
//! number is advisory. A soft limit never blocks; it exists to fire the threshold alerts
//! in [`crate::workers::budget_alerts`].
//!
//! # Fail open, deliberately
//!
//! If the store is unreachable, a counter reads as zero and the request proceeds. That is
//! a considered trade: during an outage of *our* infrastructure we would rather serve
//! traffic we reconcile afterwards than reject paying customers. The exposure is bounded
//! by outage duration, and the reconciliation worker rebuilds the counters from
//! `usage_records` afterwards — which is also what recovers a reservation orphaned by a
//! process that died mid-request.
//!
//! The free tier's *request* allowance is enforced the same way, which is the one place
//! where failing open costs us real money — so free-tier overage is capped by the pooled
//! provider keys themselves as a second line of defence.

use crate::error::{AegisError, Result};
use crate::metering::usage;
use crate::middleware::auth::AuthContext;
use crate::money::MicroCents;
use crate::store::KvStore;
use crate::types::NormalizedRequest;
use chrono::Utc;
use uuid::Uuid;

/// A spend limit that applies to one region only (`P7.2`).
///
/// Separate from the organisation budget because they answer different questions. The org
/// budget bounds the total bill. This bounds how much of that total any single region may
/// consume, which is what a multinational on one contract actually needs: "the EU burned
/// the whole month by the 9th" is a real operational failure the org-level number cannot
/// express.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionBudget<'a> {
    pub region: &'a str,
    pub limit_mc: i64,
}

/// The outcome of a budget check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetDecision {
    pub allowed: bool,
    pub spend: MicroCents,
    pub limit: Option<MicroCents>,
    pub scope: &'static str,
}

impl BudgetDecision {
    /// Convert a rejection into the 402 the client receives.
    pub fn into_error(self) -> AegisError {
        AegisError::BudgetExceeded {
            spend_micro_cents: self.spend.as_i64(),
            limit_micro_cents: self.limit.unwrap_or(MicroCents::ZERO).as_i64(),
        }
    }

    /// Proportion of the budget consumed, for the dashboard and alert thresholds.
    pub fn utilization_percent(&self) -> f64 {
        match self.limit {
            Some(limit) if limit.as_i64() > 0 => {
                (self.spend.as_i64() as f64 / limit.as_i64() as f64) * 100.0
            }
            _ => 0.0,
        }
    }
}

/// One budget counter this request touches, and the ceiling that applies to it.
///
/// `limit` is `None` when the counter is tracked but unconstrained. Those still take part
/// in a reservation: [`crate::metering::usage::emit`] subtracts the reserved amount from
/// every counter it bumps, so a counter that skipped the reservation would end the request
/// under-counted by exactly the projection.
#[derive(Debug, Clone)]
struct Scope {
    name: &'static str,
    key: String,
    limit: Option<i64>,
    /// Soft limits never refuse a request — they exist to fire threshold alerts.
    hard: bool,
}

/// Every budget applying to one organisation, resolved from the database once and cached.
///
/// Before this existed, `budgets` rows were writable through the management API, listed on
/// the dashboard, and **never consulted by the request path** — every call site passed
/// `None` for the org, team, and region limits, so the only ceiling with any effect was
/// the one stored directly on the API key. Found in the enterprise readiness audit.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BudgetLimits {
    /// Organisation-wide monthly ceiling.
    pub org: Option<Limit>,
    /// Ceiling for the calling key's team.
    pub team: Option<Limit>,
    /// Ceiling for the calling key itself, from the `budgets` table.
    ///
    /// Separate from [`AuthContext::monthly_budget_mc`], which is the limit stored on the
    /// `api_keys` row. Both are enforced; the tighter one wins naturally, because whichever
    /// is breached first refuses the request.
    pub key: Option<Limit>,
    /// Ceiling for the region serving this request.
    pub region: Option<Limit>,
}

/// A ceiling and whether breaching it refuses the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Limit {
    pub limit_mc: i64,
    pub hard: bool,
}

impl Limit {
    pub fn hard(limit_mc: i64) -> Limit {
        Limit {
            limit_mc,
            hard: true,
        }
    }
}

impl BudgetLimits {
    /// Whether any ceiling at all applies. Lets the caller skip work entirely.
    pub fn is_empty(&self) -> bool {
        self.org.is_none() && self.team.is_none() && self.key.is_none() && self.region.is_none()
    }

    /// Fold `budgets` rows into the four scopes this request could breach.
    ///
    /// Only monthly budgets are honoured, because the counters they are checked against are
    /// monthly — enforcing a `daily` row against a month-to-date counter would refuse
    /// traffic for the rest of the month the moment one day went heavy. The management API
    /// refuses to create one; this is the second line of defence for rows that predate it.
    ///
    /// Rows scoped to a team, key, or region other than this request's are skipped rather
    /// than applied — the most common way a budget system goes wrong is capping the wrong
    /// caller.
    pub fn from_rows(
        rows: &[crate::db::repo::Budget],
        team_id: Option<Uuid>,
        api_key_id: Option<Uuid>,
        region: Option<&str>,
    ) -> BudgetLimits {
        let mut limits = BudgetLimits::default();
        for row in rows
            .iter()
            .filter(|r| r.period.eq_ignore_ascii_case("monthly"))
        {
            let limit = Limit {
                limit_mc: row.limit_mc,
                hard: row.hard_limit,
            };
            match (row.team_id, row.api_key_id, row.region.as_deref()) {
                (None, None, None) => limits.org = Some(tighter(limits.org, limit)),
                (Some(t), _, _) if Some(t) == team_id => {
                    limits.team = Some(tighter(limits.team, limit))
                }
                (_, Some(k), _) if Some(k) == api_key_id => {
                    limits.key = Some(tighter(limits.key, limit))
                }
                (_, _, Some(r))
                    if region.is_some_and(|serving| serving.eq_ignore_ascii_case(r)) =>
                {
                    limits.region = Some(tighter(limits.region, limit))
                }
                // Scoped to a different team, key, or region than this caller's. Not ours.
                _ => {}
            }
        }
        limits
    }
}

/// How long a resolved budget set is cached before the database is consulted again.
///
/// Sixty seconds, matching the API-key cache: long enough to keep PostgreSQL off a hot
/// path budgeted at 0.1ms, short enough that raising a limit for a blocked customer takes
/// effect while the operator is still on the call.
const LIMITS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Load every budget applying to this request, cached.
///
/// Returns an empty set — meaning "no ceiling" — when there is no database, when the query
/// fails, or when the organisation has configured nothing. Failing open here is the same
/// trade the rest of this module makes: an unavailable budget table must not stop a paying
/// customer's traffic.
pub async fn load_limits(
    store: &dyn KvStore,
    db: Option<&sqlx::PgPool>,
    auth: &AuthContext,
    region: Option<&str>,
) -> BudgetLimits {
    // Cached per key and region, not per org: two keys in the same org can sit in
    // different teams and therefore resolve to different ceilings.
    let cache_key = format!(
        "aegis:budget_limits:{}:{}:{}",
        auth.org_id,
        auth.api_key_id.map(|id| id.to_string()).unwrap_or_default(),
        region.unwrap_or("-")
    );

    if let Ok(Some(raw)) = store.get(&cache_key).await {
        if let Ok(limits) = serde_json::from_str::<BudgetLimits>(&raw) {
            return limits;
        }
    }

    let Some(pool) = db else {
        return BudgetLimits::default();
    };
    let Ok(rows) = crate::db::repo::list_budgets(pool, auth.org_id).await else {
        tracing::warn!(
            org_id = %auth.org_id,
            "could not load budgets; proceeding without a ceiling for this request"
        );
        return BudgetLimits::default();
    };

    let limits = BudgetLimits::from_rows(&rows, auth.team_id, auth.api_key_id, region);
    if let Ok(encoded) = serde_json::to_string(&limits) {
        let _ = store.set_ex(&cache_key, &encoded, LIMITS_CACHE_TTL).await;
    }
    limits
}

/// Drop every cached budget set for an organisation.
///
/// Called when a budget is created or deleted, so a change takes effect on the next
/// request rather than up to [`LIMITS_CACHE_TTL`] later. Without this, raising a limit for
/// a customer who is currently blocked appears not to work.
pub async fn invalidate_limits(store: &dyn KvStore, org_id: Uuid) {
    let _ = store
        .del_prefix(&format!("aegis:budget_limits:{org_id}:"))
        .await;
}

/// The lower of two ceilings, preferring the stricter enforcement mode on a tie.
fn tighter(existing: Option<Limit>, candidate: Limit) -> Limit {
    match existing {
        Some(current) if current.limit_mc < candidate.limit_mc => current,
        Some(current) if current.limit_mc == candidate.limit_mc => Limit {
            limit_mc: current.limit_mc,
            hard: current.hard || candidate.hard,
        },
        _ => candidate,
    }
}

/// Spend held against every applicable counter for the lifetime of one request.
///
/// Created by [`check_and_reserve`], consumed by [`Reservation::settle`] (the request ran)
/// or [`Reservation::release`] (it did not). Dropping one without doing either logs, and
/// leaves the reconciliation worker to correct the counter from `usage_records` — the same
/// mechanism that recovers a reservation orphaned by a process that died mid-request.
#[derive(Debug)]
#[must_use = "a reservation must be settled or released, or it holds budget until reconciliation"]
pub struct Reservation {
    keys: Vec<String>,
    amount_mc: i64,
    resolved: bool,
}

impl Reservation {
    /// The projected amount currently held.
    pub fn amount_mc(&self) -> i64 {
        self.amount_mc
    }

    /// Hand the held projection over to [`crate::metering::usage::emit`], which applies
    /// the correction to the real cost.
    ///
    /// Returns the amount to put on [`crate::metering::usage::UsageEvent::reserved_mc`].
    ///
    /// # Why this does no I/O
    ///
    /// The obvious design has `settle` apply `actual - projected` itself. That is wrong,
    /// and wrong in a way that is easy to miss and expensive to ship: `emit` *also*
    /// applies `actual - reserved` to the very same counters, so both running leaves each
    /// counter one full correction too high — every customer's metered spend inflated by
    /// the difference between estimate and reality, on every request. Caught by
    /// `emit_does_not_double_count_a_reserved_request`, which is the reason it exists.
    ///
    /// So exactly one place moves the counter after a reservation, and it is `emit` —
    /// which already owns the org, team, key, and region counters and knows the real cost.
    /// This method exists to make the handover explicit and to mark the reservation
    /// resolved so [`Drop`] does not report it as leaked.
    ///
    /// If `emit` then fails, the projection stays held rather than being released. That is
    /// the safe direction: the request *was* served and *is* billable, so holding budget
    /// against it is closer to the truth than giving it back. The loss is logged, counted
    /// by `aegis_usage_events_lost_total`, and repaired by the reconciliation worker.
    #[must_use = "the returned amount must be put on UsageEvent::reserved_mc, or emit will \
                  double-count this request"]
    pub fn commit(mut self) -> i64 {
        self.resolved = true;
        self.amount_mc
    }

    /// Give the projection back untouched, for a request that never reached a provider.
    pub async fn release(mut self, store: &dyn KvStore) {
        self.resolved = true;
        if self.amount_mc == 0 {
            return;
        }
        for key in &self.keys {
            let _ = store.incr_by(key, -self.amount_mc, Some(COUNTER_TTL)).await;
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.resolved && self.amount_mc != 0 {
            // Cannot do async work here, so this is a report rather than a repair. The
            // counter self-corrects when the reconciliation worker next runs.
            tracing::warn!(
                amount_mc = self.amount_mc,
                scopes = self.keys.len(),
                "budget reservation dropped without settle or release; the counter holds \
                 this amount until reconciliation rebuilds it from usage_records"
            );
        }
    }
}

/// Either a granted reservation or the decision that refused it.
#[derive(Debug)]
pub enum BudgetOutcome {
    /// The request may proceed; the projection is held until settled.
    Allowed(Reservation),
    /// The request is refused. Nothing was left held.
    Denied(Box<BudgetDecision>),
}

/// Read-only budget check. Reports where an organisation stands without reserving.
///
/// This is what the dashboard and a pre-flight estimate want. **It is not enforcement** —
/// a request path that uses it admits concurrent requests past the limit, which is exactly
/// the bug that motivated [`check_and_reserve`]. The request path must use that instead.
pub async fn check(
    store: &dyn KvStore,
    auth: &AuthContext,
    org_budget_mc: Option<i64>,
    team_budget_mc: Option<i64>,
    region_budget: Option<RegionBudget<'_>>,
) -> Result<BudgetDecision> {
    let limits = BudgetLimits {
        org: org_budget_mc.map(Limit::hard),
        team: team_budget_mc.map(Limit::hard),
        key: None,
        region: region_budget.map(|r| Limit::hard(r.limit_mc)),
    };
    let region_name = region_budget.map(|r| r.region);

    for scope in scopes(auth, &limits, region_name) {
        let Some(limit) = scope.limit else { continue };
        let spend = read_counter(store, &scope.key).await;
        if spend >= limit {
            return Ok(BudgetDecision {
                allowed: false,
                spend: MicroCents(spend),
                limit: Some(MicroCents(limit)),
                scope: scope.name,
            });
        }
    }

    let org_spend = usage::current_spend(store, auth.org_id).await;
    Ok(BudgetDecision {
        allowed: true,
        spend: org_spend,
        limit: org_budget_mc.map(MicroCents),
        scope: "organization",
    })
}

/// Atomically reserve `projected_mc` against every budget this request touches.
///
/// The increment *is* the check: `incr_by` returns the post-increment total, so a scope's
/// ceiling is evaluated against a figure that already includes this request and every other
/// request currently in flight. Two callers racing at the same headroom get two different
/// totals back, and at most one of them fits.
///
/// On refusal, every increment already applied is rolled back before returning, so a
/// rejected request leaves the counters exactly as it found them.
pub async fn check_and_reserve(
    store: &dyn KvStore,
    auth: &AuthContext,
    limits: &BudgetLimits,
    region: Option<&str>,
    projected_mc: i64,
) -> Result<BudgetOutcome> {
    let projected_mc = projected_mc.max(0);
    let scopes = scopes(auth, limits, region);

    let mut held: Vec<String> = Vec::with_capacity(scopes.len());
    let mut breach: Option<BudgetDecision> = None;

    for scope in &scopes {
        // Fail open on a store error: an unreachable counter must not reject a paying
        // customer. The scope is skipped rather than reserved, so nothing is left held.
        let Ok(total) = store
            .incr_by(&scope.key, projected_mc, Some(COUNTER_TTL))
            .await
        else {
            continue;
        };
        held.push(scope.key.clone());

        let Some(limit) = scope.limit else { continue };
        if !scope.hard {
            // Soft limits are for alerting, not refusal.
            continue;
        }

        // Two ways to be over. `total > limit` catches the request that would cross the
        // line — the case a hard limit exists to prevent. `committed >= limit` catches an
        // organisation already at or past it, including when the projection is zero
        // because the model has no price.
        let committed = total - projected_mc;
        if total > limit || committed >= limit {
            breach = Some(BudgetDecision {
                allowed: false,
                spend: MicroCents(committed.max(0)),
                limit: Some(MicroCents(limit)),
                scope: scope.name,
            });
            break;
        }
    }

    if let Some(decision) = breach {
        // Roll back everything, including the scope that tripped: a refused request must
        // not consume budget it was never allowed to spend.
        if projected_mc != 0 {
            for key in &held {
                let _ = store.incr_by(key, -projected_mc, Some(COUNTER_TTL)).await;
            }
        }
        return Ok(BudgetOutcome::Denied(Box::new(decision)));
    }

    Ok(BudgetOutcome::Allowed(Reservation {
        keys: held,
        amount_mc: projected_mc,
        resolved: false,
    }))
}

/// Every counter this request touches, innermost scope first.
///
/// Order matters for the error message: a rejection should name the most specific limit
/// that was actually hit, because that is the one the customer can act on.
fn scopes(auth: &AuthContext, limits: &BudgetLimits, region: Option<&str>) -> Vec<Scope> {
    let at = Utc::now();
    let mut scopes = Vec::with_capacity(4);

    if let Some(api_key_id) = auth.api_key_id {
        // The key can be capped in two places: on the `api_keys` row and by a `budgets`
        // row. Both point at the same counter, so take the tighter of the two.
        let from_key_row = auth.monthly_budget_mc.map(Limit::hard);
        let limit = match (from_key_row, limits.key) {
            (Some(a), Some(b)) => Some(tighter(Some(a), b)),
            (a, b) => a.or(b),
        };
        scopes.push(Scope {
            name: "key",
            key: usage::key_spend_key(api_key_id, at),
            limit: limit.map(|l| l.limit_mc),
            hard: limit.is_none_or(|l| l.hard),
        });
    }

    if let Some(team_id) = auth.team_id {
        scopes.push(Scope {
            name: "team",
            key: usage::team_spend_key(team_id, at),
            limit: limits.team.map(|l| l.limit_mc),
            hard: limits.team.is_none_or(|l| l.hard),
        });
    }

    // Regional limit. An organisation running in several regions has one spend figure but
    // several exposures: the org budget stops the total and does nothing to stop one
    // region consuming the whole allowance before the others wake up.
    if let Some(region) = region {
        scopes.push(Scope {
            name: "region",
            key: usage::org_region_spend_key(auth.org_id, region, at),
            limit: limits.region.map(|l| l.limit_mc),
            hard: limits.region.is_none_or(|l| l.hard),
        });
    }

    scopes.push(Scope {
        name: "organization",
        key: usage::org_spend_key(auth.org_id, at),
        limit: limits.org.map(|l| l.limit_mc),
        hard: limits.org.is_none_or(|l| l.hard),
    });

    scopes
}

/// Read one counter, treating an unreadable one as zero. See "fail open" above.
async fn read_counter(store: &dyn KvStore, key: &str) -> i64 {
    store
        .get(key)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
}

/// TTL applied to a counter created by a reservation.
///
/// Matches [`crate::metering::usage`]'s own counter TTL so a counter first touched by a
/// reservation expires on the same schedule as one first touched by an emit.
const COUNTER_TTL: std::time::Duration = std::time::Duration::from_secs(45 * 24 * 3_600);

/// Check the free tier's monthly request allowance.
///
/// Free-tier traffic runs on our own pooled provider keys, so this is the only limit that
/// directly protects our margin rather than a customer's.
pub async fn check_free_tier_allowance(
    store: &dyn KvStore,
    auth: &AuthContext,
    monthly_allowance: u64,
) -> Result<bool> {
    if auth.plan != "free" {
        return Ok(true);
    }
    Ok(usage::current_requests(store, auth.org_id).await < monthly_allowance)
}

/// Project what this request will cost, before it runs.
///
/// Input tokens are known; output tokens are not, so this assumes an output roughly a
/// third the size of the input — the ratio typical of chat traffic. It is an estimate used
/// only to decide whether a request would clearly breach a budget, never for billing.
pub fn project_cost(
    request: &NormalizedRequest,
    pricing: &crate::metering::pricing::PricingTable,
) -> MicroCents {
    let input_tokens = request.estimated_input_tokens();
    let projected_output = request
        .max_tokens
        .map(u64::from)
        .unwrap_or_else(|| (input_tokens / 3).max(256));

    pricing
        .cost(&request.model, input_tokens, projected_output)
        .unwrap_or(MicroCents::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::KeyContext;
    use crate::metering::pricing::PricingTable;
    use crate::metering::savings::SavingsBreakdown;
    use crate::metering::usage::{self, UsageEvent};
    use crate::store::MemoryStore;
    use crate::types::{CacheOutcome, RoutingReason, TokenUsage};
    use uuid::Uuid;

    fn auth(plan: &str) -> AuthContext {
        AuthContext::from_key(KeyContext {
            api_key_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            rate_limit_per_minute: 60,
            monthly_budget_mc: None,
            allowed_models: None,
            plan: plan.to_string(),
            savings_share_bp: 2_000,
            zero_retention: false,
            org_region: "eu-central".into(),
        })
    }

    /// A billable usage event for `cost` micro-cents, without emitting it.
    fn billed_event(auth: &AuthContext, cost: i64) -> UsageEvent {
        let mut event = UsageEvent::new(
            Uuid::new_v4(),
            auth.org_id,
            auth.api_key_id,
            auth.team_id,
            "gpt-4o".into(),
            "gpt-4o".into(),
            "openai".into(),
            TokenUsage {
                input_tokens: 10,
                output_tokens: 10,
                estimated: false,
                ..Default::default()
            },
            SavingsBreakdown::compute(MicroCents(cost), MicroCents(cost), 2_000),
            10,
            0.1,
            CacheOutcome::Miss,
            RoutingReason::Passthrough,
            None,
            200,
        );
        event.actual_cost_mc = cost;
        event
    }

    /// Record `cost` micro-cents of spend for this caller.
    async fn spend(store: &MemoryStore, auth: &AuthContext, cost: i64) {
        let event = UsageEvent {
            team_id: auth.team_id,
            ..UsageEvent::new(
                Uuid::new_v4(),
                auth.org_id,
                auth.api_key_id,
                auth.team_id,
                "gpt-4o".into(),
                "gpt-4o".into(),
                "openai".into(),
                TokenUsage {
                    input_tokens: 10,
                    output_tokens: 10,
                    estimated: false,
                    ..Default::default()
                },
                SavingsBreakdown::compute(MicroCents(cost), MicroCents(cost), 2_000),
                10,
                0.1,
                CacheOutcome::Miss,
                RoutingReason::Passthrough,
                None,
                200,
            )
        };
        let mut event = event;
        event.actual_cost_mc = cost;
        usage::emit(store, &event).await.unwrap();
    }

    #[tokio::test]
    async fn requests_under_budget_are_allowed() {
        let store = MemoryStore::new();
        let context = auth("pro");
        let decision = check(&store, &context, Some(1_000_000), None, None)
            .await
            .unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.spend, MicroCents::ZERO);
    }

    #[tokio::test]
    async fn an_org_over_its_budget_is_rejected_with_402() {
        let store = MemoryStore::new();
        let context = auth("pro");
        spend(&store, &context, 1_500_000).await;

        let decision = check(&store, &context, Some(1_000_000), None, None)
            .await
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.scope, "organization");

        let error = decision.into_error();
        assert_eq!(error.status().as_u16(), 402);
        assert_eq!(error.error_type(), "budget_exceeded");
    }

    #[tokio::test]
    async fn the_most_specific_budget_is_the_one_reported() {
        // A key over its own limit should be told about the key, not the organisation.
        let store = MemoryStore::new();
        let mut context = auth("pro");
        context.monthly_budget_mc = Some(500_000);
        spend(&store, &context, 600_000).await;

        let decision = check(&store, &context, Some(10_000_000), None, None)
            .await
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.scope, "key");
        assert_eq!(decision.limit, Some(MicroCents(500_000)));
    }

    #[tokio::test]
    async fn team_budgets_are_enforced() {
        let store = MemoryStore::new();
        let mut context = auth("team");
        context.team_id = Some(Uuid::new_v4());
        spend(&store, &context, 2_000_000).await;

        let decision = check(&store, &context, None, Some(1_000_000), None)
            .await
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.scope, "team");
    }

    #[tokio::test]
    async fn no_configured_budget_means_no_limit() {
        let store = MemoryStore::new();
        let context = auth("enterprise");
        spend(&store, &context, 999_999_999).await;

        let decision = check(&store, &context, None, None, None).await.unwrap();
        assert!(
            decision.allowed,
            "an org with no budget must not be blocked"
        );
    }

    #[tokio::test]
    async fn spending_exactly_the_limit_blocks_the_next_request() {
        // The boundary matters: a customer who sets a $10 cap should not be able to spend
        // $10.05 because the comparison was strict.
        let store = MemoryStore::new();
        let context = auth("pro");
        spend(&store, &context, 1_000_000).await;

        let decision = check(&store, &context, Some(1_000_000), None, None)
            .await
            .unwrap();
        assert!(!decision.allowed);
    }

    #[tokio::test]
    async fn budgets_are_isolated_between_organisations() {
        let store = MemoryStore::new();
        let heavy = auth("pro");
        let light = auth("pro");
        spend(&store, &heavy, 5_000_000).await;

        assert!(
            !check(&store, &heavy, Some(1_000_000), None, None)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            check(&store, &light, Some(1_000_000), None, None)
                .await
                .unwrap()
                .allowed
        );
    }

    #[tokio::test]
    async fn free_tier_request_allowance_is_enforced() {
        let store = MemoryStore::new();
        let context = auth("free");

        assert!(check_free_tier_allowance(&store, &context, 3)
            .await
            .unwrap());
        for _ in 0..3 {
            spend(&store, &context, 0).await;
        }
        assert!(
            !check_free_tier_allowance(&store, &context, 3)
                .await
                .unwrap(),
            "the free allowance must stop at the cap"
        );
    }

    #[tokio::test]
    async fn paid_plans_have_no_request_allowance() {
        let store = MemoryStore::new();
        let context = auth("pro");
        for _ in 0..100 {
            spend(&store, &context, 0).await;
        }
        assert!(check_free_tier_allowance(&store, &context, 3)
            .await
            .unwrap());
    }

    #[test]
    fn utilization_is_reported_for_alert_thresholds() {
        let decision = BudgetDecision {
            allowed: true,
            spend: MicroCents(800_000),
            limit: Some(MicroCents(1_000_000)),
            scope: "organization",
        };
        assert!((decision.utilization_percent() - 80.0).abs() < 0.001);
    }

    #[test]
    fn utilization_with_no_limit_is_zero_not_nan() {
        let decision = BudgetDecision {
            allowed: true,
            spend: MicroCents(100),
            limit: None,
            scope: "organization",
        };
        assert_eq!(decision.utilization_percent(), 0.0);

        let zero_limit = BudgetDecision {
            limit: Some(MicroCents::ZERO),
            ..decision
        };
        assert_eq!(zero_limit.utilization_percent(), 0.0);
    }

    #[test]
    fn cost_projection_scales_with_the_request() {
        let pricing = PricingTable::with_seed_data();
        let small = NormalizedRequest::simple("gpt-4o", "hi");
        let large = NormalizedRequest::simple("gpt-4o", &"word ".repeat(5_000));

        assert!(project_cost(&large, &pricing) > project_cost(&small, &pricing));
        assert!(project_cost(&small, &pricing) > MicroCents::ZERO);
    }

    #[test]
    fn cost_projection_uses_max_tokens_when_supplied() {
        let pricing = PricingTable::with_seed_data();
        let base = NormalizedRequest::simple("gpt-4o", "hi");
        let capped = NormalizedRequest {
            max_tokens: Some(10),
            ..base.clone()
        };
        let generous = NormalizedRequest {
            max_tokens: Some(4_000),
            ..base
        };

        assert!(project_cost(&generous, &pricing) > project_cost(&capped, &pricing));
    }

    #[test]
    fn an_unpriced_model_projects_zero_rather_than_guessing() {
        let pricing = PricingTable::with_seed_data();
        let request = NormalizedRequest::simple("unknown-private-model", "hi");
        assert_eq!(project_cost(&request, &pricing), MicroCents::ZERO);
    }

    /// Record `cost` micro-cents of spend attributed to a region.
    ///
    /// Goes through the real `usage::emit` rather than writing the counter directly, so
    /// the test fails if the regional counter is ever dropped from the emit path — which
    /// is the failure that would silently disable every regional budget.
    async fn spend_in_region(store: &MemoryStore, auth: &AuthContext, region: &str, cost: i64) {
        let mut event = UsageEvent::new(
            Uuid::new_v4(),
            auth.org_id,
            auth.api_key_id,
            auth.team_id,
            "gpt-4o".into(),
            "gpt-4o".into(),
            "openai".into(),
            TokenUsage {
                input_tokens: 10,
                output_tokens: 10,
                estimated: false,
                ..Default::default()
            },
            SavingsBreakdown::compute(MicroCents(cost), MicroCents(cost), 2_000),
            10,
            0.1,
            CacheOutcome::Miss,
            RoutingReason::Passthrough,
            None,
            200,
        );
        event.actual_cost_mc = cost;
        event.region = Some(region.to_string());
        usage::emit(store, &event).await.unwrap();
    }

    #[tokio::test]
    async fn a_regional_limit_blocks_only_the_region_that_breached_it() {
        let store = MemoryStore::new();
        let context = auth("pro");

        spend_in_region(&store, &context, "eu-central", 2_000_000).await;

        let breached = RegionBudget {
            region: "eu-central",
            limit_mc: 1_000_000,
        };
        let decision = check(&store, &context, None, None, Some(breached))
            .await
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.scope, "region");

        // The other region is untouched. That is the whole point: one region running hot
        // must not take the rest of the organisation offline.
        let other = RegionBudget {
            region: "us-east",
            limit_mc: 1_000_000,
        };
        let decision = check(&store, &context, None, None, Some(other))
            .await
            .unwrap();
        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn regional_spend_does_not_leak_between_organisations() {
        // A counter keyed by region alone would aggregate every tenant in the region into
        // one number — useless to a customer, and a cross-tenant leak.
        let store = MemoryStore::new();
        let noisy = auth("pro");
        let quiet = auth("pro");

        spend_in_region(&store, &noisy, "eu-central", 9_000_000).await;

        let limit = RegionBudget {
            region: "eu-central",
            limit_mc: 1_000_000,
        };

        assert!(
            !check(&store, &noisy, None, None, Some(limit))
                .await
                .unwrap()
                .allowed
        );
        assert!(
            check(&store, &quiet, None, None, Some(limit))
                .await
                .unwrap()
                .allowed,
            "one organisation's regional spend must not block another's"
        );
    }

    #[tokio::test]
    async fn regions_are_matched_case_insensitively() {
        // The region reaches the counter from config on one side and from a budget row on
        // the other. Those two are typed by different people at different times.
        let store = MemoryStore::new();
        let context = auth("pro");

        spend_in_region(&store, &context, "EU-Central", 2_000_000).await;

        let limit = RegionBudget {
            region: "eu-central",
            limit_mc: 1_000_000,
        };
        assert!(
            !check(&store, &context, None, None, Some(limit))
                .await
                .unwrap()
                .allowed
        );
    }

    #[tokio::test]
    async fn no_regional_budget_means_no_regional_check() {
        let store = MemoryStore::new();
        let context = auth("pro");

        spend_in_region(&store, &context, "eu-central", 500_000_000).await;

        // Enormous regional spend, no regional limit configured: the request proceeds.
        // Adding a scope must never start rejecting traffic for customers who did not
        // opt in to it.
        let decision = check(&store, &context, None, None, None).await.unwrap();
        assert!(decision.allowed);
    }

    // -----------------------------------------------------------------------------
    // Adversarial: is budget enforcement actually race-safe under concurrency?
    //
    // The rate limiter (store.rs) is proven atomic — 50 racing callers against a limit
    // of 10 admits exactly 10, via a single atomic Redis Lua script. Budget::check reads
    // the current spend and compares it to the limit, but the spend counter itself is
    // only incremented later, asynchronously, when usage::emit() runs after each
    // request's own provider call finishes. That is a classic check-then-act gap: if
    // many requests arrive while spend is just under the limit, every one of them can
    // read "allowed" before any of them has recorded its own cost.
    // -----------------------------------------------------------------------------

    // `flavor = "multi_thread"` matters here, not just as a style choice: the default
    // single-threaded `#[tokio::test]` runtime schedules spawned tasks cooperatively on
    // one OS thread, and this test's own first attempt under that default did not
    // reproduce the race even once in several runs — not because the check-then-act gap
    // was not real (it plainly was, reading the function), but because cooperative
    // single-threaded scheduling happened to let each task's check-then-spend sequence
    // complete before the next task got a turn. Real production concurrency runs on
    // genuinely parallel OS threads, so the test needs to as well or it is proving a
    // weaker property than the one that actually matters.
    //
    // History, because it is the point of this test: written during the enterprise
    // readiness audit (2026-08-25) to *demonstrate* a bypass, and it did — 2-5 of 20
    // concurrent requests admitted past a hard budget with $0.05 of headroom, final spend
    // $1.15-$1.45 against a $1.00 limit, 8 runs out of 8. It was committed `#[ignore]`d so
    // the proof stayed runnable without permanently reddening CI. `check_and_reserve` then
    // replaced read-compare with an atomic reserve-and-check, and this test is what proves
    // it worked: same barrier, same headroom, same concurrency, now passing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_requests_cannot_overshoot_a_hard_budget() {
        let store = std::sync::Arc::new(MemoryStore::new());
        let context = auth("pro");
        let limit = 1_000_000; // $1.00 hard budget
        let cost_per_request = 100_000; // $0.10 per request
        let limits = BudgetLimits {
            org: Some(Limit::hard(limit)),
            ..BudgetLimits::default()
        };

        // Prime spend to $0.95 — five cents of headroom, half a request's worth.
        spend(&store, &context, 950_000).await;

        // A barrier forces all 20 tasks to reserve at effectively the same instant, rather
        // than relying on tokio::spawn's scheduling to happen to overlap them — the whole
        // point is to remove luck from whether the race window is hit.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(20));

        // Each task follows the real pipeline's order exactly: reserve a projection before
        // the (here, simulated) provider call, then settle the real cost afterwards.
        let mut handles = Vec::new();
        for _ in 0..20 {
            let store = std::sync::Arc::clone(&store);
            let context = context.clone();
            let limits = limits.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                let outcome =
                    check_and_reserve(store.as_ref(), &context, &limits, None, cost_per_request)
                        .await
                        .unwrap();
                match outcome {
                    BudgetOutcome::Allowed(reservation) => {
                        // The request runs and costs exactly what was projected, so the
                        // correction `emit` would apply is zero and the held amount is
                        // already the right figure.
                        let _ = reservation.commit();
                        true
                    }
                    BudgetOutcome::Denied(_) => false,
                }
            }));
        }

        let mut admitted = 0;
        for handle in handles {
            if handle.await.unwrap() {
                admitted += 1;
            }
        }

        let final_spend = usage::current_spend(store.as_ref(), context.org_id).await;
        println!(
            "admitted: {admitted}, final spend: {} micro-cents (limit was {limit})",
            final_spend.as_i64()
        );

        // A hard limit means the line is not crossed. With $0.05 of headroom and requests
        // costing $0.10, no request fits, so spend must not move at all.
        assert!(
            final_spend.as_i64() <= limit,
            "budget bypassed: {admitted} of 20 concurrent requests were admitted with only \
             $0.05 of headroom against a $1.00 hard limit. Final spend {} micro-cents, \
             {} over.",
            final_spend.as_i64(),
            final_spend.as_i64() - limit
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_requests_fill_a_budget_exactly_to_the_line() {
        // The complement of the test above, and the one that proves the fix is not simply
        // "refuse everything": with room for exactly four requests, exactly four are
        // admitted — not three (too strict, revenue left on the table) and not five.
        let store = std::sync::Arc::new(MemoryStore::new());
        let context = auth("pro");
        let limit = 1_000_000;
        let cost_per_request = 250_000; // $0.25 — exactly four fit
        let limits = BudgetLimits {
            org: Some(Limit::hard(limit)),
            ..BudgetLimits::default()
        };

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(20));
        let mut handles = Vec::new();
        for _ in 0..20 {
            let store = std::sync::Arc::clone(&store);
            let context = context.clone();
            let limits = limits.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                match check_and_reserve(store.as_ref(), &context, &limits, None, cost_per_request)
                    .await
                    .unwrap()
                {
                    BudgetOutcome::Allowed(reservation) => {
                        let _ = reservation.commit();
                        true
                    }
                    BudgetOutcome::Denied(_) => false,
                }
            }));
        }

        let mut admitted = 0;
        for handle in handles {
            if handle.await.unwrap() {
                admitted += 1;
            }
        }

        assert_eq!(
            admitted, 4,
            "exactly four requests fit a $1.00 budget at $0.25 each"
        );
        assert_eq!(
            usage::current_spend(store.as_ref(), context.org_id)
                .await
                .as_i64(),
            limit
        );
    }

    #[tokio::test]
    async fn a_refused_request_leaves_the_counter_untouched() {
        // The rollback path. Without it, every refused request would still consume its own
        // projection, so a customer sitting at their limit would watch their spend keep
        // climbing from requests that were never served.
        let store = MemoryStore::new();
        let context = auth("pro");
        let limits = BudgetLimits {
            org: Some(Limit::hard(1_000_000)),
            ..BudgetLimits::default()
        };
        spend(&store, &context, 1_000_000).await;

        for _ in 0..5 {
            let outcome = check_and_reserve(&store, &context, &limits, None, 100_000)
                .await
                .unwrap();
            assert!(matches!(outcome, BudgetOutcome::Denied(_)));
        }

        assert_eq!(
            usage::current_spend(&store, context.org_id).await.as_i64(),
            1_000_000,
            "refused requests must not accumulate spend"
        );
    }

    #[tokio::test]
    async fn committing_replaces_the_projection_with_the_real_cost() {
        // The projection is an estimate. If the true-up were additive rather than a
        // difference, every request would be billed twice against its own budget.
        let store = MemoryStore::new();
        let context = auth("pro");
        let limits = BudgetLimits::default();

        let BudgetOutcome::Allowed(reservation) =
            check_and_reserve(&store, &context, &limits, None, 500_000)
                .await
                .unwrap()
        else {
            panic!("no limit configured, so nothing should refuse this");
        };
        assert_eq!(
            usage::current_spend(&store, context.org_id).await.as_i64(),
            500_000,
            "the projection is held while the request runs"
        );

        // The request turned out cheaper than projected. `commit` hands the projection
        // over; `emit` is what applies the correction, so the counter only moves there.
        let reserved = reservation.commit();
        assert_eq!(reserved, 500_000);

        let mut event = billed_event(&context, 120_000);
        event.reserved_mc = reserved;
        usage::emit(&store, &event).await.unwrap();

        assert_eq!(
            usage::current_spend(&store, context.org_id).await.as_i64(),
            120_000,
            "the counter ends at the real cost, not projection plus cost"
        );
    }

    #[tokio::test]
    async fn releasing_gives_the_whole_projection_back() {
        let store = MemoryStore::new();
        let context = auth("pro");
        let limits = BudgetLimits::default();

        let BudgetOutcome::Allowed(reservation) =
            check_and_reserve(&store, &context, &limits, None, 500_000)
                .await
                .unwrap()
        else {
            panic!("no limit configured");
        };
        reservation.release(&store).await;

        assert_eq!(
            usage::current_spend(&store, context.org_id).await.as_i64(),
            0,
            "a request that never ran must cost nothing"
        );
    }

    #[tokio::test]
    async fn emit_does_not_double_count_a_reserved_request() {
        // The two halves have to agree: `check_and_reserve` adds the projection, `emit`
        // adds only the difference. This is the seam where a mistake would silently
        // double every customer's metered spend.
        let store = MemoryStore::new();
        let context = auth("pro");
        let limits = BudgetLimits::default();

        let BudgetOutcome::Allowed(reservation) =
            check_and_reserve(&store, &context, &limits, None, 300_000)
                .await
                .unwrap()
        else {
            panic!("no limit configured");
        };
        let reserved = reservation.commit();

        let mut event = UsageEvent::new(
            Uuid::new_v4(),
            context.org_id,
            context.api_key_id,
            None,
            "gpt-4o".into(),
            "gpt-4o".into(),
            "openai".into(),
            TokenUsage {
                input_tokens: 10,
                output_tokens: 10,
                estimated: false,
                ..Default::default()
            },
            SavingsBreakdown::compute(MicroCents(400_000), MicroCents(400_000), 2_000),
            10,
            0.1,
            CacheOutcome::Miss,
            RoutingReason::Passthrough,
            None,
            200,
        );
        event.actual_cost_mc = 400_000;
        event.reserved_mc = reserved;
        usage::emit(&store, &event).await.unwrap();

        assert_eq!(
            usage::current_spend(&store, context.org_id).await.as_i64(),
            400_000,
            "a reserved-then-settled-then-emitted request must count exactly once"
        );
    }

    #[tokio::test]
    async fn a_soft_limit_never_refuses() {
        // `hard_limit = false` exists in the schema and was previously ignored entirely.
        // A soft limit is for alerting; blocking on one would be a surprise a customer
        // explicitly opted out of.
        let store = MemoryStore::new();
        let context = auth("pro");
        let limits = BudgetLimits {
            org: Some(Limit {
                limit_mc: 1_000,
                hard: false,
            }),
            ..BudgetLimits::default()
        };
        spend(&store, &context, 900_000).await;

        let outcome = check_and_reserve(&store, &context, &limits, None, 100_000)
            .await
            .unwrap();
        assert!(
            matches!(outcome, BudgetOutcome::Allowed(_)),
            "a soft limit must not block"
        );
        if let BudgetOutcome::Allowed(r) = outcome {
            r.release(&store).await;
        }
    }

    #[test]
    fn budget_rows_resolve_onto_the_scope_they_name() {
        let team = Uuid::new_v4();
        let key = Uuid::new_v4();
        let other_team = Uuid::new_v4();

        let rows = vec![
            budget_row(None, None, None, 10_000, true),
            budget_row(Some(team), None, None, 5_000, true),
            budget_row(None, Some(key), None, 2_000, true),
            budget_row(None, None, Some("eu-central"), 7_000, true),
            // Belongs to someone else. Applying it would cap the wrong caller, which is
            // the single most damaging way a budget system can be wrong.
            budget_row(Some(other_team), None, None, 1, true),
            // Wrong region for this request.
            budget_row(None, None, Some("us-east"), 1, true),
        ];

        let limits = BudgetLimits::from_rows(&rows, Some(team), Some(key), Some("eu-central"));
        assert_eq!(limits.org.unwrap().limit_mc, 10_000);
        assert_eq!(limits.team.unwrap().limit_mc, 5_000);
        assert_eq!(limits.key.unwrap().limit_mc, 2_000);
        assert_eq!(limits.region.unwrap().limit_mc, 7_000);
    }

    #[test]
    fn a_daily_budget_is_ignored_rather_than_misapplied() {
        // The counters are month-to-date. Enforcing a daily figure against them would
        // refuse traffic for the rest of the month after one heavy day — worse than not
        // enforcing it, because it looks like it works.
        let mut row = budget_row(None, None, None, 10_000, true);
        row.period = "daily".to_string();
        let limits = BudgetLimits::from_rows(&[row], None, None, None);
        assert!(limits.is_empty());
    }

    #[test]
    fn the_tighter_of_two_budgets_on_one_scope_wins() {
        let rows = vec![
            budget_row(None, None, None, 10_000, true),
            budget_row(None, None, None, 3_000, true),
        ];
        let limits = BudgetLimits::from_rows(&rows, None, None, None);
        assert_eq!(limits.org.unwrap().limit_mc, 3_000);
    }

    #[test]
    fn a_hard_and_soft_budget_at_the_same_figure_resolve_to_hard() {
        let rows = vec![
            budget_row(None, None, None, 5_000, false),
            budget_row(None, None, None, 5_000, true),
        ];
        let limits = BudgetLimits::from_rows(&rows, None, None, None);
        assert!(limits.org.unwrap().hard, "the stricter mode must win a tie");
    }

    fn budget_row(
        team_id: Option<Uuid>,
        api_key_id: Option<Uuid>,
        region: Option<&str>,
        limit_mc: i64,
        hard_limit: bool,
    ) -> crate::db::repo::Budget {
        crate::db::repo::Budget {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id,
            api_key_id,
            region: region.map(|r| r.to_string()),
            period: "monthly".to_string(),
            limit_mc,
            hard_limit,
            created_at: Utc::now(),
        }
    }
}

//! Budget enforcement — pipeline stage [3].
//!
//! Reads Redis spend counters and rejects with **402 Payment Required** when a hard limit
//! is exceeded. Budget: 0.1ms, so this is counter reads only — the authoritative figures
//! are rebuilt from `usage_records` by the reconciliation worker.
//!
//! # Fail open, deliberately
//!
//! If Redis is unreachable, [`crate::metering::usage::current_spend`] reports zero and the
//! request proceeds. That is a considered trade: during a cache outage we would rather
//! serve traffic we reconcile afterwards than reject paying customers because our own
//! infrastructure is unwell. The exposure is bounded by how long an outage lasts, and the
//! reconciliation job surfaces any overspend.
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

/// Check every budget that applies to this request.
///
/// Checked innermost first — key, then team, then organisation — so the rejection names
/// the most specific limit that was actually hit, which is the one the customer can act
/// on.
pub async fn check(
    store: &dyn KvStore,
    auth: &AuthContext,
    org_budget_mc: Option<i64>,
    team_budget_mc: Option<i64>,
    region_budget: Option<RegionBudget<'_>>,
) -> Result<BudgetDecision> {
    if let (Some(api_key_id), Some(limit)) = (auth.api_key_id, auth.monthly_budget_mc) {
        let spend = usage::current_key_spend(store, api_key_id).await;
        if spend.as_i64() >= limit {
            return Ok(BudgetDecision {
                allowed: false,
                spend,
                limit: Some(MicroCents(limit)),
                scope: "key",
            });
        }
    }

    if let (Some(team_id), Some(limit)) = (auth.team_id, team_budget_mc) {
        let spend = usage::current_team_spend(store, team_id).await;
        if spend.as_i64() >= limit {
            return Ok(BudgetDecision {
                allowed: false,
                spend,
                limit: Some(MicroCents(limit)),
                scope: "team",
            });
        }
    }

    // Regional limit. An organisation running in several regions has one spend figure but
    // several exposures: the org budget stops the total and does nothing to stop one
    // region consuming the whole allowance before the others wake up.
    if let Some(RegionBudget { region, limit_mc }) = region_budget {
        let spend = usage::current_region_spend(store, auth.org_id, region).await;
        if spend.as_i64() >= limit_mc {
            return Ok(BudgetDecision {
                allowed: false,
                spend,
                limit: Some(MicroCents(limit_mc)),
                scope: "region",
            });
        }
    }

    let org_spend = usage::current_spend(store, auth.org_id).await;
    if let Some(limit) = org_budget_mc {
        if org_spend.as_i64() >= limit {
            return Ok(BudgetDecision {
                allowed: false,
                spend: org_spend,
                limit: Some(MicroCents(limit)),
                scope: "organization",
            });
        }
    }

    Ok(BudgetDecision {
        allowed: true,
        spend: org_spend,
        limit: org_budget_mc.map(MicroCents),
        scope: "organization",
    })
}

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
}

//! Rate limiting — pipeline stage [2].
//!
//! Two independent windows are checked, and both must admit the request:
//!
//! * **Per key** — the customer's own configured limit, protecting them from a runaway
//!   loop in their own code.
//! * **Per organisation** — a plan-derived ceiling, protecting *us* from an organisation
//!   minting a hundred keys to route around a per-key limit.
//!
//! Budget: 0.1ms, which is one Redis round trip per window against
//! [`crate::store::RATE_LIMIT_LUA`]. The Lua script makes trim-count-admit atomic, so
//! concurrent requests cannot both be admitted into the last remaining slot.

use crate::error::{AegisError, Result};
use crate::middleware::auth::AuthContext;
use crate::store::{KvStore, RateLimitOutcome};
use std::time::Duration;
use uuid::Uuid;

/// The window every limit is measured over.
pub const WINDOW: Duration = Duration::from_secs(60);

/// Requests per minute an organisation may make, by plan.
///
/// The free tier's 10/minute is deliberately low: it is enough to evaluate the product
/// and far too little to run production traffic on our own pooled provider keys, which
/// are the only real cost a free user imposes on us.
pub fn org_limit_for_plan(plan: &str) -> u32 {
    match plan {
        "pro" => 600,
        "team" => 3_000,
        "enterprise" => 10_000,
        "api" => 10_000,
        _ => 10,
    }
}

/// Redis key for a per-key window.
fn key_window(api_key_id: Uuid) -> String {
    format!("aegis:rl:key:{api_key_id}")
}

/// Redis key for a per-organisation window.
fn org_window(org_id: Uuid) -> String {
    format!("aegis:rl:org:{org_id}")
}

/// Redis key for a per-IP window (abuse control on unauthenticated routes).
fn ip_window(ip: &str) -> String {
    format!("aegis:rl:ip:{ip}")
}

/// What the limiter decided, including the headers to echo back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub retry_after_secs: u64,
    /// Which window rejected the request.
    pub scope: &'static str,
}

impl RateLimitDecision {
    /// Convert a rejection into the error the client receives.
    pub fn into_error(self) -> AegisError {
        AegisError::RateLimited {
            retry_after_secs: self.retry_after_secs.max(1),
            limit: self.limit,
            scope: self.scope,
        }
    }

    /// `X-RateLimit-*` headers for a successful response.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("x-ratelimit-limit", self.limit.to_string()),
            ("x-ratelimit-remaining", self.remaining.to_string()),
            ("x-ratelimit-scope", self.scope.to_string()),
        ]
    }
}

/// Check both windows for an authenticated request.
///
/// The per-key window is checked first because it is the one a customer configured and
/// therefore the one they expect to hit. Reporting "your key's limit" rather than an
/// opaque org-wide number is the difference between an actionable 429 and a support
/// ticket.
pub async fn check(store: &dyn KvStore, auth: &AuthContext) -> Result<RateLimitDecision> {
    if let Some(api_key_id) = auth.api_key_id {
        let limit = auth.rate_limit_per_minute.max(1);
        let outcome = store
            .rate_limit(&key_window(api_key_id), limit, WINDOW)
            .await?;
        if !outcome.allowed {
            return Ok(rejected(outcome, limit, "key"));
        }
    }

    let org_limit = org_limit_for_plan(&auth.plan);
    let outcome = store
        .rate_limit(&org_window(auth.org_id), org_limit, WINDOW)
        .await?;
    if !outcome.allowed {
        return Ok(rejected(outcome, org_limit, "organization"));
    }

    Ok(RateLimitDecision {
        allowed: true,
        limit: org_limit,
        remaining: outcome.remaining,
        retry_after_secs: 0,
        scope: "organization",
    })
}

/// Per-IP limit for unauthenticated endpoints (signup, login, password reset).
///
/// Cloudflare handles volumetric abuse; this stops the credential-stuffing rate that
/// stays under a WAF threshold but would still let someone grind through a password list.
pub async fn check_ip(store: &dyn KvStore, ip: &str, limit: u32) -> Result<RateLimitDecision> {
    let outcome = store.rate_limit(&ip_window(ip), limit, WINDOW).await?;
    if !outcome.allowed {
        return Ok(rejected(outcome, limit, "ip"));
    }
    Ok(RateLimitDecision {
        allowed: true,
        limit,
        remaining: outcome.remaining,
        retry_after_secs: 0,
        scope: "ip",
    })
}

fn rejected(outcome: RateLimitOutcome, limit: u32, scope: &'static str) -> RateLimitDecision {
    RateLimitDecision {
        allowed: false,
        limit,
        remaining: 0,
        retry_after_secs: outcome.retry_after_secs.max(1),
        scope,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::KeyContext;
    use crate::store::MemoryStore;

    fn auth(plan: &str, key_limit: u32) -> AuthContext {
        AuthContext::from_key(KeyContext {
            api_key_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            rate_limit_per_minute: key_limit as i32,
            monthly_budget_mc: None,
            allowed_models: None,
            plan: plan.to_string(),
            savings_share_bp: 2_000,
            zero_retention: false,
            org_region: "eu-central".into(),
        })
    }

    #[test]
    fn plan_limits_match_the_business_model() {
        assert_eq!(org_limit_for_plan("free"), 10);
        assert_eq!(org_limit_for_plan("pro"), 600);
        assert_eq!(org_limit_for_plan("team"), 3_000);
        assert_eq!(org_limit_for_plan("enterprise"), 10_000);
        // An unrecognised plan must get the most restrictive limit, not the loosest.
        assert_eq!(org_limit_for_plan("something-new"), 10);
    }

    #[tokio::test]
    async fn requests_under_the_limit_are_admitted() {
        let store = MemoryStore::new();
        let context = auth("pro", 5);
        for i in 0..5 {
            let decision = check(&store, &context).await.unwrap();
            assert!(decision.allowed, "request {i} rejected");
        }
    }

    #[tokio::test]
    async fn the_key_limit_rejects_and_reports_its_scope() {
        let store = MemoryStore::new();
        let context = auth("pro", 3);
        for _ in 0..3 {
            assert!(check(&store, &context).await.unwrap().allowed);
        }

        let decision = check(&store, &context).await.unwrap();
        assert!(!decision.allowed);
        // Naming the key, not the org, is what makes the 429 actionable.
        assert_eq!(decision.scope, "key");
        assert_eq!(decision.limit, 3);
        assert!(decision.retry_after_secs >= 1);
    }

    #[tokio::test]
    async fn the_org_limit_catches_what_the_key_limit_misses() {
        // A free-tier org with a generous per-key limit still cannot exceed its plan:
        // this is what stops someone minting keys to route around the limit.
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();

        let mut rejected_scope = None;
        for _ in 0..20 {
            let mut context = auth("free", 10_000);
            context.org_id = org_id;
            let decision = check(&store, &context).await.unwrap();
            if !decision.allowed {
                rejected_scope = Some(decision.scope);
                break;
            }
        }
        assert_eq!(rejected_scope, Some("organization"));
    }

    #[tokio::test]
    async fn separate_keys_have_separate_windows() {
        let store = MemoryStore::new();
        let first = auth("pro", 2);
        let second = auth("pro", 2);

        for _ in 0..2 {
            assert!(check(&store, &first).await.unwrap().allowed);
        }
        assert!(!check(&store, &first).await.unwrap().allowed);
        // A different key in a different org is unaffected.
        assert!(check(&store, &second).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn rejection_converts_to_a_429_with_usable_headers() {
        let store = MemoryStore::new();
        let context = auth("pro", 1);
        assert!(check(&store, &context).await.unwrap().allowed);

        let decision = check(&store, &context).await.unwrap();
        let error = decision.into_error();
        assert_eq!(error.status().as_u16(), 429);
        assert_eq!(error.error_type(), "rate_limit_exceeded");
        // The message must tell the caller when to come back.
        assert!(error.to_string().contains("rate limit"));
    }

    #[tokio::test]
    async fn successful_decisions_expose_remaining_capacity() {
        let store = MemoryStore::new();
        let decision = check(&store, &auth("pro", 10)).await.unwrap();
        let headers = decision.headers();
        assert!(headers.iter().any(|(k, _)| *k == "x-ratelimit-limit"));
        assert!(headers.iter().any(|(k, _)| *k == "x-ratelimit-remaining"));
    }

    #[tokio::test]
    async fn ip_limits_protect_unauthenticated_routes() {
        let store = MemoryStore::new();
        for _ in 0..5 {
            assert!(check_ip(&store, "203.0.113.7", 5).await.unwrap().allowed);
        }
        let decision = check_ip(&store, "203.0.113.7", 5).await.unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.scope, "ip");

        // A different address is unaffected.
        assert!(check_ip(&store, "203.0.113.8", 5).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn a_zero_configured_limit_never_blocks_everything() {
        // A corrupt or zero rate limit must not brick a customer's key entirely; it is
        // floored to 1 so at least some traffic flows while they fix the setting.
        let store = MemoryStore::new();
        let mut context = auth("pro", 0);
        context.rate_limit_per_minute = 0;
        assert!(check(&store, &context).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn session_requests_are_limited_by_organisation_only() {
        // Dashboard sessions have no API key, so only the org window applies.
        let store = MemoryStore::new();
        let mut context = auth("free", 100);
        context.api_key_id = None;

        let mut admitted = 0;
        for _ in 0..20 {
            if check(&store, &context).await.unwrap().allowed {
                admitted += 1;
            }
        }
        assert_eq!(admitted, org_limit_for_plan("free"));
    }
}

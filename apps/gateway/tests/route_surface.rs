//! Every advertised route is actually wired.
//!
//! # Why this test exists
//!
//! A handler can be written, reviewed, tested at the unit level, and still be unreachable
//! because nobody added a `.route(...)` line for it. That failure is silent: the code
//! compiles, the unit tests pass, and the endpoint returns 404 in production. It has
//! happened in this repository more than once.
//!
//! So this asserts the routing table itself. It does not need a database — a 404 means
//! *no route matched*, whereas 401/500/503 all mean a route matched and the handler ran.
//! Distinguishing those two is the entire point, and it works with no infrastructure.

use aegis_gateway::build_router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// Send a bare request and return the status. No auth, no body — we only care whether
/// the router recognised the path and method.
async fn probe(method: &str, path: &str) -> StatusCode {
    let router = build_router(aegis_gateway::AppState::for_tests());

    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .expect("request builds");

    router
        .oneshot(request)
        .await
        .expect("router responds")
        .status()
}

/// The full advertised surface: everything a customer, a dashboard, or an identity
/// provider is told to call. Adding a route to `docs/API.md` without adding it here (and
/// to the router) is the mistake this list is designed to catch.
fn advertised_routes() -> Vec<(&'static str, &'static str)> {
    vec![
        // Operational.
        ("GET", "/health"),
        ("GET", "/ready"),
        ("GET", "/metrics"),
        ("GET", "/status"),
        // The provider-compatible surface.
        ("POST", "/v1/chat/completions"),
        ("GET", "/v1/models"),
        ("POST", "/v1/embeddings"),
        ("POST", "/v1/messages"),
        // Auth.
        ("POST", "/api/auth/signup"),
        ("POST", "/api/auth/login"),
        ("POST", "/api/auth/logout"),
        ("GET", "/api/auth/me"),
        ("POST", "/api/auth/totp/enroll"),
        ("POST", "/api/auth/totp/confirm"),
        ("POST", "/api/auth/totp/disable"),
        // Keys.
        ("GET", "/api/keys"),
        ("POST", "/api/keys"),
        ("GET", "/api/keys/00000000-0000-0000-0000-000000000000"),
        ("PATCH", "/api/keys/00000000-0000-0000-0000-000000000000"),
        ("DELETE", "/api/keys/00000000-0000-0000-0000-000000000000"),
        // Organisation.
        ("GET", "/api/org"),
        ("PATCH", "/api/org"),
        ("GET", "/api/org/members"),
        ("POST", "/api/org/members/invite"),
        (
            "DELETE",
            "/api/org/members/00000000-0000-0000-0000-000000000000",
        ),
        ("GET", "/api/org/teams"),
        ("POST", "/api/org/teams"),
        (
            "DELETE",
            "/api/org/teams/00000000-0000-0000-0000-000000000000",
        ),
        // Providers, policies, budgets.
        ("GET", "/api/providers"),
        ("POST", "/api/providers"),
        (
            "DELETE",
            "/api/providers/00000000-0000-0000-0000-000000000000",
        ),
        (
            "POST",
            "/api/providers/00000000-0000-0000-0000-000000000000/test",
        ),
        ("GET", "/api/scim-tokens"),
        ("POST", "/api/scim-tokens"),
        (
            "DELETE",
            "/api/scim-tokens/00000000-0000-0000-0000-000000000000",
        ),
        ("GET", "/api/policies"),
        ("POST", "/api/policies"),
        (
            "DELETE",
            "/api/policies/00000000-0000-0000-0000-000000000000",
        ),
        ("GET", "/api/budgets"),
        ("POST", "/api/budgets"),
        (
            "DELETE",
            "/api/budgets/00000000-0000-0000-0000-000000000000",
        ),
        // Reporting.
        ("GET", "/api/models"),
        ("GET", "/api/usage/summary"),
        ("GET", "/api/usage/anomalies"),
        ("GET", "/api/usage/chargeback"),
        ("GET", "/api/usage/chargeback.csv"),
        ("GET", "/api/requests"),
        ("GET", "/api/savings/report.csv"),
        // Billing.
        ("GET", "/api/billing/plan"),
        ("GET", "/api/billing/credits"),
        ("POST", "/api/billing/referral"),
        // Admin.
        ("GET", "/api/admin/metrics"),
        ("GET", "/api/admin/routing"),
        ("GET", "/api/admin/pricing"),
        ("GET", "/api/admin/audit"),
        ("POST", "/api/admin/providers/openai/reset"),
        // Enterprise: SCIM 2.0 and SSO.
        ("GET", "/scim/v2/ServiceProviderConfig"),
        ("GET", "/scim/v2/Users"),
        ("POST", "/scim/v2/Users"),
        ("GET", "/scim/v2/Users/00000000-0000-0000-0000-000000000000"),
        (
            "PATCH",
            "/scim/v2/Users/00000000-0000-0000-0000-000000000000",
        ),
        ("PUT", "/scim/v2/Users/00000000-0000-0000-0000-000000000000"),
        (
            "DELETE",
            "/scim/v2/Users/00000000-0000-0000-0000-000000000000",
        ),
        // The old paths here ("/api/sso/start", POST) matched neither the route table nor
        // what sso_start's own doc comment and redirect_uri construction assumed — this
        // test asserted the bug's shape rather than catching it. Corrected alongside
        // registering /api/auth/sso/callback, which did not exist at all.
        ("GET", "/api/auth/sso/start"),
        ("GET", "/api/auth/sso/callback"),
        ("GET", "/api/auth/sso/connections"),
    ]
}

#[tokio::test]
async fn every_advertised_route_is_reachable() {
    let mut unreachable = Vec::new();

    for (method, path) in advertised_routes() {
        let status = probe(method, path).await;
        // 404 means no route matched. 405 means the path matched but not the method —
        // equally a wiring bug. Anything else means the handler ran, which is all this
        // test claims to check.
        if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
            unreachable.push(format!("{method} {path} -> {status}"));
        }
    }

    assert!(
        unreachable.is_empty(),
        "these routes are advertised but not wired into the router:\n  {}",
        unreachable.join("\n  ")
    );
}

#[tokio::test]
async fn an_unwired_path_really_does_return_404() {
    // Without this, the test above could pass because every path returns 200 for some
    // unrelated reason (a catch-all fallback, say) and it would be proving nothing.
    assert_eq!(
        probe("GET", "/api/this-endpoint-does-not-exist").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_management_api_rejects_anonymous_callers_rather_than_serving_them() {
    // Reachable is not the same as open. Every route that touches tenant data must refuse
    // an unauthenticated caller — a 200 here would be a data leak, not a routing success.
    let must_be_guarded = [
        ("GET", "/api/keys"),
        ("GET", "/api/org"),
        ("GET", "/api/models"),
        ("GET", "/api/usage/summary"),
        ("GET", "/api/usage/chargeback"),
        ("GET", "/api/billing/credits"),
        ("GET", "/api/requests"),
        ("GET", "/api/admin/audit"),
        ("GET", "/scim/v2/Users"),
    ];

    for (method, path) in must_be_guarded {
        let status = probe(method, path).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path} returned {status} to an anonymous caller; \
             expected 401 (rejected) or 503 (no database configured)"
        );
    }
}

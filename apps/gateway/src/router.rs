//! Router assembly.
//!
//! # Why this lives in the library rather than in `main.rs`
//!
//! Which routes exist, what guards them, and what middleware wraps them is application
//! logic, not process bootstrap. While it lived in the binary no integration test could
//! reach it, which meant the one failure mode that is completely silent — a handler that
//! is written, unit-tested, and never wired — was the one thing nothing could catch. See
//! `tests/route_surface.rs`.

use crate::middleware::security_headers;
use crate::routes::{admin, anthropic_compat, enterprise, health, management, openai_compat};
use crate::AppState;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// Assemble the full router.
pub fn build_router(state: AppState) -> Router {
    let max_body = state.config.max_body_bytes;
    let is_production = state.config.environment.is_production_like();

    let public = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/metrics", get(health::metrics))
        .route("/status", get(health::public_status));

    // The OpenAI- and Anthropic-compatible surfaces. These are what a customer points
    // their SDK at.
    let gateway = Router::new()
        .route(
            "/v1/chat/completions",
            post(openai_compat::chat_completions),
        )
        .route("/v1/models", get(openai_compat::list_models))
        .route("/v1/embeddings", post(openai_compat::embeddings))
        .route("/v1/messages", post(anthropic_compat::messages));

    let api = Router::new()
        .route("/api/auth/signup", post(management::signup))
        .route("/api/auth/login", post(management::login))
        .route("/api/auth/logout", post(management::logout))
        .route("/api/auth/me", get(management::me))
        .route(
            "/api/keys",
            get(management::list_keys).post(management::create_key),
        )
        .route("/api/keys/{id}", get(management::get_key))
        .route("/api/keys/{id}", patch(management::update_key))
        .route("/api/keys/{id}", delete(management::revoke_key))
        .route(
            "/api/org",
            get(management::get_org).patch(management::update_org),
        )
        .route("/api/org/members", get(management::list_members))
        .route("/api/org/members/invite", post(management::invite_member))
        .route("/api/org/members/{id}", delete(management::remove_member))
        .route(
            "/api/org/teams",
            get(management::list_teams).post(management::create_team),
        )
        .route("/api/org/teams/{id}", delete(management::delete_team))
        .route(
            "/api/providers",
            get(management::list_providers).post(management::create_provider),
        )
        .route("/api/providers/{id}", delete(management::delete_provider))
        .route("/api/providers/{id}/test", post(management::test_provider))
        .route(
            "/api/policies",
            get(management::list_policies).post(management::create_policy),
        )
        .route("/api/policies/{id}", delete(management::delete_policy))
        .route(
            "/api/budgets",
            get(management::list_budgets).post(management::create_budget),
        )
        .route("/api/budgets/{id}", delete(management::delete_budget))
        .route("/api/usage/summary", get(management::usage_summary))
        .route("/api/requests", get(management::list_requests))
        .route(
            "/api/savings/report.csv",
            get(management::savings_report_csv),
        )
        .route("/api/billing/plan", get(management::billing_plan))
        .route("/api/billing/credits", get(management::list_credits))
        .route("/api/billing/referral", post(management::claim_referral))
        .route("/api/usage/anomalies", get(management::spend_anomalies))
        .route("/api/usage/chargeback", get(management::chargeback_report))
        .route("/api/usage/chargeback.csv", get(management::chargeback_csv));

    // SCIM 2.0 and SSO. Separate from `api` because SCIM authenticates with its own
    // bearer-token namespace (an identity provider has no session cookie) and speaks
    // `application/scim+json`, not our own error envelope.
    let enterprise_routes = Router::new()
        .route(
            "/scim/v2/ServiceProviderConfig",
            get(enterprise::scim_service_provider_config),
        )
        .route(
            "/scim/v2/Users",
            get(enterprise::scim_list_users).post(enterprise::scim_create_user),
        )
        .route("/scim/v2/Users/{id}", get(enterprise::scim_get_user))
        .route("/scim/v2/Users/{id}", patch(enterprise::scim_patch_user))
        .route("/scim/v2/Users/{id}", put(enterprise::scim_patch_user))
        .route("/scim/v2/Users/{id}", delete(enterprise::scim_delete_user))
        .route("/api/sso/start", post(enterprise::sso_start))
        .route("/api/sso/connections", get(enterprise::sso_connections));

    let admin_routes = Router::new()
        .route("/api/admin/metrics", get(admin::system_metrics))
        .route("/api/admin/routing", get(admin::routing_intelligence))
        .route("/api/admin/pricing", get(admin::pricing_table))
        .route("/api/admin/audit", get(admin::audit_log))
        .route(
            "/api/admin/providers/{provider}/reset",
            post(admin::reset_circuit),
        );

    Router::new()
        .merge(public)
        .merge(gateway)
        .merge(api)
        .merge(enterprise_routes)
        .merge(admin_routes)
        .layer(axum::middleware::from_fn(move |request, next| {
            security_headers_layer(request, next, is_production)
        }))
        // Part 9 item 7: a hard body cap, applied before any parsing.
        .layer(RequestBodyLimitLayer::new(max_body))
        .layer(TraceLayer::new_for_http())
        // The dashboard is served from a different origin, and credentials must be
        // allowed for the session cookie to travel.
        .layer(cors_layer(&state.config.app_url))
        .with_state(state)
}

/// Apply security headers to every response.
async fn security_headers_layer(
    request: axum::extract::Request,
    next: axum::middleware::Next,
    is_production: bool,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in security_headers::headers(is_production) {
        headers.insert(name, value);
    }
    response
}

/// CORS for the dashboard origin.
///
/// A specific origin rather than a wildcard: `Access-Control-Allow-Credentials` and `*`
/// are mutually exclusive, and the session cookie needs credentials.
fn cors_layer(app_url: &str) -> CorsLayer {
    match app_url.parse::<axum::http::HeaderValue>() {
        Ok(origin) => CorsLayer::new()
            .allow_origin(origin)
            .allow_credentials(true)
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderName::from_static("x-aegis-routing-hint"),
                axum::http::HeaderName::from_static("x-aegis-org"),
                axum::http::HeaderName::from_static("x-api-key"),
            ])
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ]),
        Err(_) => {
            tracing::warn!(app_url, "invalid AEGIS_APP_URL; CORS disabled");
            CorsLayer::new()
        }
    }
}

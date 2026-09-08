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
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
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
        .route("/api/auth/accept-invite", post(management::accept_invite))
        // TOTP two-factor. `totp_enabled` and `totp_secret_encrypted` have existed on
        // `users` since the initial schema and the RFC 6238 algorithm was always correct;
        // nothing previously read or wrote the column, and login never checked it, so
        // 2FA could not actually be turned on. Found in the enterprise readiness audit.
        .route("/api/auth/totp/enroll", post(management::totp_enroll))
        .route("/api/auth/totp/confirm", post(management::totp_confirm))
        .route("/api/auth/totp/disable", post(management::totp_disable))
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
        .route(
            "/api/org/members/{id}",
            delete(management::remove_member).patch(management::update_member_role),
        )
        .route(
            "/api/org/teams",
            get(management::list_teams).post(management::create_team),
        )
        .route(
            "/api/org/teams/{id}",
            patch(management::update_team).delete(management::delete_team),
        )
        // A project (team) lead's or org admin's view of one project's spend — org owner
        // or admin may reach any team, a team lead only their own, everyone else 404s.
        // See `management::assert_project_access`.
        .route("/api/org/teams/{id}/usage", get(management::project_usage))
        .route(
            "/api/org/teams/{id}/members",
            get(management::list_team_members).post(management::add_team_member),
        )
        .route(
            "/api/org/teams/{id}/members/{user_id}",
            delete(management::remove_team_member),
        )
        .route(
            "/api/providers",
            get(management::list_providers).post(management::create_provider),
        )
        .route("/api/providers/{id}", delete(management::delete_provider))
        .route("/api/providers/{id}/test", post(management::test_provider))
        // Self-service SCIM token issuance. repo::create_scim_token existed and was
        // tested; nothing in the API ever called it, so a customer could not turn on SCIM
        // without a direct database write on our side. Found in the enterprise readiness
        // audit.
        .route(
            "/api/scim-tokens",
            get(management::list_scim_tokens).post(management::create_scim_token),
        )
        .route(
            "/api/scim-tokens/{id}",
            delete(management::revoke_scim_token),
        )
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
        .route("/api/models", get(management::model_catalogue))
        // Dry-run compression: no provider call, no spend, no usage record. Exists so
        // the saving can be shown on a real prompt rather than asserted.
        .route(
            "/api/compression/preview",
            post(management::compression_preview),
        )
        .route("/api/usage/summary", get(management::usage_summary))
        // A member's own attributed spend — every request made with a key issued to them.
        .route("/api/me/usage", get(management::my_usage))
        .route(
            "/api/me/onboarding-complete",
            post(management::complete_onboarding),
        )
        .route("/api/requests", get(management::list_requests))
        .route(
            "/api/savings/report.csv",
            get(management::savings_report_csv),
        )
        // A customer's own audit trail. `/api/admin/audit` existed but is gated by
        // `is_admin` (platform staff), so no customer could ever reach it — the
        // compliance whitepaper's "audit log export (JSONL, SIEM-friendly)" described a
        // capability nothing in the router actually provided. Found in the enterprise
        // readiness audit.
        .route("/api/audit-log.jsonl", get(management::audit_log_export))
        .route("/api/billing/plan", get(management::billing_plan))
        .route("/api/billing/credits", get(management::list_credits))
        .route("/api/billing/referral", post(management::claim_referral))
        .route("/api/billing/checkout", post(management::create_checkout))
        // Stripe calls this directly — no session cookie, no API key. Authenticated by
        // Stripe-Signature alone, verified inside the handler itself; see its own doc
        // comment for why that is exactly as strong an authenticator as this endpoint needs.
        .route("/api/billing/webhook", post(management::stripe_webhook))
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
        // Registered at /api/auth/sso/..., matching what sso_start itself constructs as
        // the OAuth redirect_uri and what sso_connections' own doc comment always claimed.
        // Previously registered at /api/sso/start — a path nothing else in the codebase
        // referenced — so the route existed but not at the URL any client or identity
        // provider would actually reach. Compounding the callback route's total absence
        // (see sso_callback's doc comment): together, no SSO login could ever complete.
        // Found in the enterprise readiness audit.
        .route("/api/auth/sso/start", get(enterprise::sso_start))
        .route("/api/auth/sso/callback", get(enterprise::sso_callback))
        .route(
            "/api/auth/sso/connections",
            get(enterprise::sso_connections),
        );

    let admin_routes = Router::new()
        .route("/api/admin/metrics", get(admin::system_metrics))
        .route("/api/admin/routing", get(admin::routing_intelligence))
        .route("/api/admin/pricing", get(admin::pricing_table))
        .route("/api/admin/pricing/reload", post(admin::reload_pricing))
        .route(
            "/api/admin/pricing/openrouter/refresh",
            post(admin::refresh_openrouter_reference),
        )
        .route("/api/admin/audit", get(admin::audit_log))
        .route(
            "/api/admin/providers/{provider}/reset",
            post(admin::reset_circuit),
        );

    let deadline = state.config.request_deadline;

    Router::new()
        .merge(public)
        .merge(gateway)
        .merge(api)
        .merge(enterprise_routes)
        .merge(admin_routes)
        .layer(axum::middleware::from_fn(move |request, next| {
            security_headers_layer(request, next, is_production)
        }))
        // An outer deadline on every request.
        //
        // Without one, the worst case is unbounded in a way that is easy to miss: each
        // fallback-chain entry can spend up to `provider_timeout` (120s for a reasoning
        // model) per try, times three tries, times however many entries the chain has. A
        // client can therefore wait minutes to be told the request failed. This bounds it
        // at one number an operator can reason about and tune, and it is deliberately the
        // outermost timing layer so it covers auth, routing, and metering too — not just
        // the provider call.
        //
        // 504 rather than 500: the caller needs to know this was a timeout, because that
        // is the difference between "retry" and "do not retry".
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            deadline,
        ))
        .layer(axum::middleware::from_fn(move |request, next| {
            map_timeout_to_504(request, next, deadline.as_secs())
        }))
        // Part 9 item 7: a hard body cap, applied before any parsing.
        .layer(RequestBodyLimitLayer::new(max_body))
        // Correlation. `x-aegis-request-id` is echoed if the caller sent one and minted
        // otherwise, then attached to the tracing span that wraps the whole request — so
        // every log line a request produces carries the id a customer can quote, which was
        // the thing that made incident diagnosis effectively impossible before.
        .layer(axum::middleware::from_fn(correlation_layer))
        .layer(TraceLayer::new_for_http())
        // The dashboard is served from a different origin, and credentials must be
        // allowed for the session cookie to travel.
        .layer(cors_layer(
            &state.config.app_url,
            state.config.environment.is_production_like(),
        ))
        .with_state(state)
}

/// Header carrying the correlation id, on the way in and on the way out.
pub const REQUEST_ID_HEADER: &str = "x-aegis-request-id";

/// Attach a correlation id to the request, the tracing span, and the response.
///
/// A caller's own id is honoured when it looks sane, so a trace can be followed across a
/// customer's system and ours. Anything longer than 128 characters or containing something
/// other than ASCII alphanumerics, dashes, and underscores is replaced rather than trusted:
/// this value lands in log lines and response headers, and neither is a place to put
/// unvalidated caller input.
async fn correlation_layer(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let inbound = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map(|v| v.to_string());

    let request_id = inbound.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Available to handlers through the extensions, so a handler that mints its own
    // pipeline id can tie the two together.
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let span = tracing::info_span!(
        "http",
        request_id = %request_id,
        method = %method,
        path = %path,
    );

    let mut response = {
        use tracing::Instrument;
        next.run(request).instrument(span).await
    };

    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
            value,
        );
    }
    response
}

/// The correlation id for the request currently being served.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

/// Turn `tower_http`'s timeout, which surfaces as a bare 408, into a 504 with a body.
///
/// A gateway that times out waiting on an upstream is a 504 by definition, and a client
/// deciding whether to retry needs the distinction from a 408 (which blames the caller).
async fn map_timeout_to_504(
    request: axum::extract::Request,
    next: axum::middleware::Next,
    deadline_secs: u64,
) -> axum::response::Response {
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    if response.status() != axum::http::StatusCode::REQUEST_TIMEOUT {
        return response;
    }
    tracing::warn!(path = %path, "request exceeded the gateway deadline");
    crate::error::AegisError::ProviderTimeout(deadline_secs).into_response()
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
///
/// # Why this is not `mirror_request`
///
/// This layer used to mirror the caller's own `Origin` back whenever `app_url` mentioned
/// localhost, and *also* whenever `AEGIS_APP_URL` failed to parse — logging a warning and
/// carrying on. Mirroring combined with `allow_credentials(true)` tells every origin on
/// the internet that it may make credentialed requests and read the response: any page a
/// signed-in user visits could have read that org's keys, usage, and budgets with their
/// session cookie attached. The parse-failure branch made it worse by failing *open*, so a
/// typo in one production environment variable silently removed the boundary.
///
/// Now the allowed set is always explicit. Production permits exactly the configured
/// dashboard origin. Development additionally permits loopback and private-LAN origins on
/// any port — which is what makes `getApiUrl()`'s LAN mode work — but never an arbitrary
/// internet origin, and never as a consequence of misconfiguration.
fn cors_layer(app_url: &str, production_like: bool) -> CorsLayer {
    let configured = app_url.parse::<axum::http::HeaderValue>().ok();
    if configured.is_none() {
        // Not fatal here — `Config::validate` already refuses to start a production-like
        // process with an unparseable app_url, so reaching this in production is not
        // possible. In development it means the dev allowlist below is the only thing
        // granting access, which is the safe direction to fail.
        tracing::warn!(
            app_url,
            "AEGIS_APP_URL is not a valid origin header; it will not be granted CORS access"
        );
    }

    let origin_config = tower_http::cors::AllowOrigin::predicate(
        move |origin: &axum::http::HeaderValue, _parts: &axum::http::request::Parts| {
            if configured.as_ref().is_some_and(|allowed| allowed == origin) {
                return true;
            }
            !production_like && is_local_origin(origin)
        },
    );

    CorsLayer::new()
        .allow_origin(origin_config)
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
        ])
}

/// Whether an `Origin` header points at this machine or the local network.
///
/// Used only outside production, to let a developer reach the gateway from
/// `http://localhost:3000`, from `http://127.0.0.1:3000`, or from the LAN address a phone
/// or a second machine on the same network would use. Deliberately parsed rather than
/// substring-matched: `https://localhost.evil.com` contains "localhost" and must not pass,
/// which a `contains()` check would have allowed.
fn is_local_origin(origin: &axum::http::HeaderValue) -> bool {
    let Ok(text) = origin.to_str() else {
        return false;
    };
    let Ok(url) = url::Url::parse(text) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn origin(value: &str) -> HeaderValue {
        HeaderValue::from_str(value).expect("test origin must be a valid header value")
    }

    // -----------------------------------------------------------------------
    // CORS origin classification.
    //
    // These exist because the previous implementation mirrored the caller's own
    // `Origin` back while also setting `Access-Control-Allow-Credentials: true`,
    // which authorises every site on the internet to make credentialed requests
    // and read the response. The tests below are the boundary.
    // -----------------------------------------------------------------------

    #[test]
    fn loopback_and_lan_origins_are_recognised_as_local() {
        // The developer cases that must keep working, including the LAN address a
        // second device on the same network uses to reach a dev gateway.
        for value in [
            "http://localhost:3000",
            "http://localhost",
            "https://localhost:8443",
            "http://127.0.0.1:3000",
            "http://192.168.1.108:3000",
            "http://10.0.0.5:3000",
            "http://172.16.4.2:3000",
            "http://[::1]:3000",
        ] {
            assert!(
                is_local_origin(&origin(value)),
                "{value} should be treated as local"
            );
        }
    }

    #[test]
    fn a_hostname_merely_containing_localhost_is_not_local() {
        // The exact attack the old `app_url.contains("localhost")` check allowed
        // through: an attacker-controlled domain that contains the magic substring.
        for value in [
            "https://localhost.evil.com",
            "https://notlocalhost",
            "https://evil.com/?x=localhost",
            "https://127.0.0.1.evil.com",
        ] {
            assert!(
                !is_local_origin(&origin(value)),
                "{value} must NOT be treated as local"
            );
        }
    }

    #[test]
    fn public_origins_are_never_local() {
        for value in [
            "https://example.com",
            "https://8.8.8.8",
            "http://203.0.113.10:3000",
        ] {
            assert!(
                !is_local_origin(&origin(value)),
                "{value} must NOT be treated as local"
            );
        }
    }

    #[test]
    fn a_malformed_origin_is_not_local() {
        // A value that is a legal header but not a legal URL must fail closed.
        assert!(!is_local_origin(&origin("not-a-url")));
        assert!(!is_local_origin(&origin("null")));
    }

    // -----------------------------------------------------------------------
    // Configuration refuses to start production with an origin it cannot honour.
    // -----------------------------------------------------------------------

    #[test]
    fn production_refuses_an_unparseable_app_url() {
        // The fail-open path that used to exist: an invalid AEGIS_APP_URL logged a
        // warning and then mirrored every origin. Now it cannot start at all.
        let mut config = crate::config::Config::for_tests();
        config.environment = crate::config::Environment::Prod;
        config.database_url = Some("postgres://x/y".into());
        config.redis_url = Some("redis://x".into());
        config.base_url = "https://api.aegis.dev".into();
        config.app_url = "not a valid header\nvalue".into();

        let error = config
            .validate_for_tests()
            .expect_err("an unparseable app_url must refuse to start in production");
        assert!(
            format!("{error}").contains("AEGIS_APP_URL"),
            "the error must name the variable at fault: {error}"
        );
    }

    #[test]
    fn production_accepts_a_valid_app_url() {
        let mut config = crate::config::Config::for_tests();
        config.environment = crate::config::Environment::Prod;
        config.database_url = Some("postgres://x/y".into());
        config.redis_url = Some("redis://x".into());
        config.base_url = "https://api.aegis.dev".into();
        config.app_url = "https://app.aegis.dev".into();

        assert!(config.validate_for_tests().is_ok());
    }
}

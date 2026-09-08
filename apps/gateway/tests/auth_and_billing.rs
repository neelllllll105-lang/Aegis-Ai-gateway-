//! End-to-end integration tests for the Phase 1 and Phase 2 acceptance criteria.
//!
//! Phase 1 requires: signup, login, create key, list, revoke, and a revoked key being
//! rejected. Phase 2 requires a usage record with correct tokens and idempotent writes.
//! These run against a real database so the column mapping that
//! `docs/adr/0004-runtime-checked-sql.md` gives up at compile time is verified here
//! instead.

mod common;

use common::{cleanup, create_key, create_org, repo, setup, skip};

#[tokio::test]
async fn the_full_key_lifecycle_works() {
    // The Phase 1 acceptance criterion, end to end.
    let Some((state, pool)) = setup().await else {
        return skip("the_full_key_lifecycle_works");
    };

    use aegis_gateway::crypto;
    use aegis_gateway::middleware::auth;

    let fixture = create_org(&pool, "lifecycle").await;
    let (plaintext, key_id) = create_key(&pool, &fixture, "production").await;

    // The key authenticates.
    let context = auth::authenticate_api_key(&state, &plaintext)
        .await
        .expect("a freshly created key must authenticate");
    assert_eq!(context.org_id, fixture.org_id);
    assert_eq!(context.api_key_id, Some(key_id));

    // It appears in the listing, with only its prefix.
    let keys = repo::list_api_keys(&pool, fixture.org_id)
        .await
        .expect("query");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].name, "production");
    assert!(plaintext.starts_with(&keys[0].key_prefix));

    // The full key is not recoverable from the API surface.
    let serialized = serde_json::to_string(&keys[0]).expect("serialize");
    assert!(
        !serialized.contains(&plaintext),
        "the plaintext key leaked through serialization"
    );
    assert!(!serialized.contains("key_hash"), "the hash leaked");

    // Revoke it.
    assert!(repo::revoke_api_key(&pool, fixture.org_id, key_id)
        .await
        .expect("query"));

    // Caches must be invalidated, or the key keeps working for up to the TTL.
    auth::invalidate_key(
        state.store.as_ref(),
        &state.key_cache,
        &crypto::hash_token(&plaintext),
    )
    .await
    .expect("invalidate");

    // And now it is rejected.
    let err = auth::authenticate_api_key(&state, &plaintext)
        .await
        .expect_err("a revoked key must not authenticate");
    assert_eq!(err.error_type(), "unauthorized");

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn usage_records_are_written_idempotently() {
    // Phase 2: the usage worker redelivers on restart, so a repeated insert must not
    // double-bill. This is the property that makes at-least-once delivery safe.
    let Some((_, pool)) = setup().await else {
        return skip("usage_records_are_written_idempotently");
    };

    use aegis_gateway::metering::savings::SavingsBreakdown;
    use aegis_gateway::metering::usage::UsageEvent;
    use aegis_gateway::money::MicroCents;
    use aegis_gateway::types::{CacheOutcome, RoutingReason, TokenUsage};
    use chrono::{Duration, Utc};

    let fixture = create_org(&pool, "idempotency").await;

    let event = UsageEvent::new(
        uuid::Uuid::new_v4(),
        fixture.org_id,
        None,
        None,
        "gpt-4o".into(),
        "gpt-4o-mini".into(),
        "openai".into(),
        TokenUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            estimated: false,
            ..Default::default()
        },
        SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), 2_000),
        842,
        0.371,
        CacheOutcome::Miss,
        RoutingReason::Complexity,
        Some(0.21),
        200,
    );

    assert!(
        repo::insert_usage_record(&pool, &event)
            .await
            .expect("insert"),
        "the first insert should write a row"
    );

    // Five redeliveries of the identical event.
    for attempt in 0..5 {
        let written = repo::insert_usage_record(&pool, &event)
            .await
            .expect("insert");
        assert!(
            !written,
            "redelivery {attempt} wrote a duplicate row — this would double-bill"
        );
    }

    let summary = repo::usage_summary(
        &pool,
        fixture.org_id,
        Utc::now() - Duration::hours(1),
        Utc::now() + Duration::hours(1),
    )
    .await
    .expect("query");

    assert_eq!(summary.requests, 1, "exactly one record should exist");
    assert_eq!(summary.input_tokens, 1_000);
    assert_eq!(summary.output_tokens, 500);
    assert_eq!(summary.baseline_cost_mc, 7_500);
    assert_eq!(summary.actual_cost_mc, 450);
    assert_eq!(summary.gross_savings_mc, 7_050);
    assert_eq!(summary.aegis_fee_mc, 1_410);

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn the_database_rejects_a_fee_larger_than_the_saving() {
    // A defence in depth behind the arithmetic: even if application code were wrong, the
    // CHECK constraint stops an over-charge reaching the billing table.
    let Some((_, pool)) = setup().await else {
        return skip("the_database_rejects_a_fee_larger_than_the_saving");
    };

    let fixture = create_org(&pool, "constraint").await;

    let result = sqlx::query(
        "INSERT INTO usage_records
            (request_id, org_id, requested_model, served_model, provider,
             gross_savings_mc, aegis_fee_mc, status_code)
         VALUES ($1, $2, 'gpt-4o', 'gpt-4o-mini', 'openai', 1000, 5000, 200)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(fixture.org_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "the database accepted a fee larger than the saving it was charged on"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn the_database_rejects_a_cache_hit_that_cost_money() {
    let Some((_, pool)) = setup().await else {
        return skip("the_database_rejects_a_cache_hit_that_cost_money");
    };

    let fixture = create_org(&pool, "cache-constraint").await;

    let result = sqlx::query(
        "INSERT INTO usage_records
            (request_id, org_id, requested_model, served_model, provider,
             cache_hit, cache_type, actual_cost_mc, status_code)
         VALUES ($1, $2, 'gpt-4o', 'gpt-4o', 'openai', true, 'exact', 5000, 200)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(fixture.org_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "the database accepted a cache hit with a non-zero cost"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn an_organisation_cannot_be_both_zero_retention_and_capturing_content() {
    // Principle 4 made structurally impossible rather than merely enforced in code.
    let Some((_, pool)) = setup().await else {
        return skip("an_organisation_cannot_be_both_zero_retention_and_capturing_content");
    };

    let fixture = create_org(&pool, "retention").await;

    let result = sqlx::query(
        "UPDATE organizations SET zero_retention = true, content_capture = true WHERE id = $1",
    )
    .bind(fixture.org_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "an organisation was allowed to capture content while claiming zero retention"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn enabling_zero_retention_forces_content_capture_off() {
    // The repository handles the combination in one statement so a customer turning on
    // zero retention never hits a constraint error.
    let Some((_, pool)) = setup().await else {
        return skip("enabling_zero_retention_forces_content_capture_off");
    };

    let fixture = create_org(&pool, "retention-toggle").await;

    let with_capture =
        repo::update_org_settings(&pool, fixture.org_id, None, None, None, Some(true), None)
            .await
            .expect("enable capture");
    assert!(with_capture.content_capture);

    let private =
        repo::update_org_settings(&pool, fixture.org_id, None, None, Some(true), None, None)
            .await
            .expect("enable zero retention");

    assert!(private.zero_retention);
    assert!(
        !private.content_capture,
        "zero retention must force content capture off in the same statement"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn the_last_owner_cannot_be_removed() {
    // An organisation with no owner cannot be administered, and recovering one is a
    // manual database operation.
    let Some((_, pool)) = setup().await else {
        return skip("the_last_owner_cannot_be_removed");
    };

    let fixture = create_org(&pool, "last-owner").await;

    let err = repo::remove_member(&pool, fixture.org_id, fixture.user_id)
        .await
        .expect_err("removing the last owner must fail");
    assert!(format!("{err}").contains("last owner"), "{err}");

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn users_are_found_by_email_case_insensitively() {
    // People type their address with inconsistent capitalisation, and a login that fails
    // because of a capital letter is indistinguishable from a wrong password.
    let Some((_, pool)) = setup().await else {
        return skip("users_are_found_by_email_case_insensitively");
    };

    let fixture = create_org(&pool, "email-case").await;

    for variant in [
        fixture.email.clone(),
        fixture.email.to_uppercase(),
        format!("  {}  ", fixture.email),
    ] {
        let found = repo::find_user_by_email(&pool, variant.trim())
            .await
            .expect("query");
        assert!(
            found.is_some(),
            "the address {variant:?} did not resolve to the user"
        );
        assert_eq!(found.expect("user").id, fixture.user_id);
    }

    // A genuinely different address must not match.
    let other = repo::find_user_by_email(&pool, "nobody@test.invalid")
        .await
        .expect("query");
    assert!(other.is_none());

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn audit_entries_are_written_and_readable() {
    let Some((_, pool)) = setup().await else {
        return skip("audit_entries_are_written_and_readable");
    };

    let fixture = create_org(&pool, "audit").await;

    repo::write_audit_log(
        &pool,
        fixture.org_id,
        Some(fixture.user_id),
        "key.created",
        "api_key",
        None,
        Some(serde_json::json!({"name": "production"})),
    )
    .await
    .expect("audit write");

    let entries = repo::list_audit_logs(&pool, fixture.org_id, 10, 0)
        .await
        .expect("query");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "key.created");
    assert_eq!(entries[0].org_id, fixture.org_id);

    cleanup(&pool, &fixture).await;
}

/// A session, built directly against the database rather than through the login
/// endpoint. `create_provider` requires `require_writer`, and — this is deliberate,
/// verified directly in `middleware/auth.rs::can_write` — an API key can *never* satisfy
/// that check, only a session with an `owner`/`admin` role can. Exercising the actual
/// exploit chain from the enterprise readiness audit means authenticating the way that
/// chain does: as a freshly signed-up owner, not an API key.
async fn owner_session_headers(
    pool: &sqlx::PgPool,
    fixture: &common::Fixture,
) -> axum::http::HeaderMap {
    use aegis_gateway::crypto;
    use aegis_gateway::db::repo;

    let generated = crypto::generate_session_token();
    repo::create_session(
        pool,
        fixture.user_id,
        &generated.hash,
        chrono::Utc::now() + chrono::Duration::days(1),
    )
    .await
    .expect("session creation");

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        axum::http::HeaderValue::from_str(&format!("aegis_session={}", generated.plaintext))
            .unwrap(),
    );
    headers
}

/// The actual exploit chain from the enterprise readiness audit, run against the real
/// handler with a real database: an organisation on a plan that actually has BYOK — the
/// SSRF guard's job is to reject a malicious `base_url` regardless of who is asking, so
/// this deliberately uses a Pro-plan fixture to isolate that specific guard rather than
/// the separate plan-restriction check that now runs first (see
/// `a_free_plan_org_is_blocked_before_the_ssrf_check_ever_runs` below for that one). Before
/// `middleware::ssrf_guard` existed, this call succeeded and the credential was stored;
/// the very next step in the demonstrated chain (`POST /api/providers/{id}/test`) would
/// then have made the gateway itself issue a server-side request against it.
#[tokio::test]
async fn a_freshly_signed_up_org_cannot_register_a_provider_pointed_at_cloud_metadata() {
    use aegis_gateway::routes::management;
    use axum::extract::{Json, State};

    let Some((state, pool)) = setup().await else {
        return skip(
            "a_freshly_signed_up_org_cannot_register_a_provider_pointed_at_cloud_metadata",
        );
    };

    let fixture = create_org(&pool, "ssrf-attacker").await;
    repo::update_org_plan(&pool, fixture.org_id, "pro", 2_000)
        .await
        .expect("plan upgrade");
    let headers = owner_session_headers(&pool, &fixture).await;

    let request = management::CreateCredentialRequest {
        provider: "custom".to_string(),
        api_key: "irrelevant-for-this-test".to_string(),
        base_url: Some(
            "http://169.254.169.254/latest/meta-data/iam/security-credentials/".to_string(),
        ),
        label: None,
        is_default: Some(true),
    };

    let response = management::create_provider(State(state), headers, Json(request)).await;

    assert_eq!(
        response.status(),
        axum::http::StatusCode::BAD_REQUEST,
        "a base_url pointed at the cloud metadata service must be rejected, not stored"
    );

    // Confirm it truly was never persisted, not just that this one response looked right.
    let stored = repo::list_credentials(&pool, fixture.org_id)
        .await
        .expect("query");
    assert!(
        stored.is_empty(),
        "the SSRF-targeting credential must never reach storage: {stored:?}"
    );

    cleanup(&pool, &fixture).await;
}

/// The same chain with an ordinary, legitimate custom endpoint must still work — the
/// guard's job is to distinguish these two cases, not to break BYOK custom endpoints
/// generally.
#[tokio::test]
async fn a_legitimate_custom_endpoint_is_still_accepted() {
    use aegis_gateway::routes::management;
    use axum::extract::{Json, State};

    let Some((state, pool)) = setup().await else {
        return skip("a_legitimate_custom_endpoint_is_still_accepted");
    };

    let fixture = create_org(&pool, "ssrf-legitimate").await;
    repo::update_org_plan(&pool, fixture.org_id, "pro", 2_000)
        .await
        .expect("plan upgrade");
    let headers = owner_session_headers(&pool, &fixture).await;

    let request = management::CreateCredentialRequest {
        provider: "custom".to_string(),
        api_key: "sk-test-key".to_string(),
        base_url: Some("https://api.openai.com/v1".to_string()),
        label: Some("legitimate proxy".to_string()),
        is_default: Some(true),
    };

    let response = management::create_provider(State(state), headers, Json(request)).await;

    assert_eq!(
        response.status(),
        axum::http::StatusCode::CREATED,
        "a genuine public endpoint must not be rejected by the SSRF guard"
    );

    cleanup(&pool, &fixture).await;
}

// -----------------------------------------------------------------------------
// Plan-gated management writes — `billing::features`, enforced server-side.
//
// Before this, hiding a page in the dashboard was the *only* restriction on a Free-plan
// org's access to Pro/Team features: nothing on the backend checked plan, so a direct API
// call reached every one of these handlers regardless of what the caller was paying for.
// -----------------------------------------------------------------------------

/// A brand-new signup — Free plan, the default `common::create_org` gives every fixture —
/// is blocked from registering a BYOK credential at all, and blocked *before* the SSRF
/// guard even runs: the plan check is the very first thing after role authorisation, so an
/// attacker on a plan with no BYOK never reaches the URL-validation code path in the first
/// place. That URL is otherwise identical to the malicious one two tests above use, so a
/// CREATED here (or a BAD_REQUEST from the SSRF guard instead of a plan_restricted 403)
/// would mean the ordering regressed, not just the plan gate.
#[tokio::test]
async fn a_free_plan_org_is_blocked_before_the_ssrf_check_ever_runs() {
    use aegis_gateway::routes::management;
    use axum::extract::{Json, State};

    let Some((state, pool)) = setup().await else {
        return skip("a_free_plan_org_is_blocked_before_the_ssrf_check_ever_runs");
    };

    let fixture = create_org(&pool, "free-byok-attempt").await;
    let headers = owner_session_headers(&pool, &fixture).await;

    let request = management::CreateCredentialRequest {
        provider: "custom".to_string(),
        api_key: "irrelevant-for-this-test".to_string(),
        base_url: Some(
            "http://169.254.169.254/latest/meta-data/iam/security-credentials/".to_string(),
        ),
        label: None,
        is_default: Some(true),
    };

    let response = management::create_provider(State(state), headers, Json(request)).await;

    assert_eq!(
        response.status(),
        axum::http::StatusCode::FORBIDDEN,
        "a Free-plan org must be rejected for the plan, not reach the SSRF guard at all"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["error"]["type"], "plan_restricted");
    assert!(json["error"]["upgrade_url"].is_string());

    let stored = repo::list_credentials(&pool, fixture.org_id)
        .await
        .expect("query");
    assert!(stored.is_empty());

    cleanup(&pool, &fixture).await;
}

/// The same restriction, proven against `create_team` instead of `create_provider` — a
/// different gated feature (`TeamManagement`, Team plan, not `Byok`/Pro), and a different
/// handler, so this is not just re-testing the same guard twice under a new name.
#[tokio::test]
async fn a_free_plan_org_cannot_create_a_team() {
    use aegis_gateway::routes::management;
    use axum::extract::{Json, State};

    let Some((state, pool)) = setup().await else {
        return skip("a_free_plan_org_cannot_create_a_team");
    };

    let fixture = create_org(&pool, "free-team-attempt").await;
    let headers = owner_session_headers(&pool, &fixture).await;

    let request = management::CreateTeamRequest {
        name: "shadow-project".to_string(),
        monthly_budget_mc: None,
        default_routing_mode: None,
    };
    let response =
        management::create_team(State(state.clone()), headers.clone(), Json(request)).await;
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["error"]["type"], "plan_restricted");

    // And once upgraded to Team, the identical request succeeds — proving this is a plan
    // check, not a role or validation problem that happened to also return 403.
    repo::update_org_plan(&pool, fixture.org_id, "team", 1_500)
        .await
        .expect("plan upgrade");
    let request = management::CreateTeamRequest {
        name: "shadow-project".to_string(),
        monthly_budget_mc: None,
        default_routing_mode: None,
    };
    let response = management::create_team(State(state), headers, Json(request)).await;
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);

    cleanup(&pool, &fixture).await;
}

// -----------------------------------------------------------------------------
// Project (team) rename — `repo::update_team`.
// -----------------------------------------------------------------------------

/// A project's usage figures are keyed by `team_id`, never by name (confirmed by reading
/// `usage_summary_for_team`'s own `WHERE` clause) — renaming it must not touch a single row
/// in `usage_records`. This is the concrete proof behind that claim, not just a reading of
/// the query.
#[tokio::test]
async fn renaming_a_project_does_not_disturb_its_usage_history() {
    use aegis_gateway::metering::savings::SavingsBreakdown;
    use aegis_gateway::metering::usage::UsageEvent;
    use aegis_gateway::money::MicroCents;
    use aegis_gateway::types::{CacheOutcome, RoutingReason, TokenUsage};
    use chrono::{Duration, Utc};

    let Some((_, pool)) = setup().await else {
        return skip("renaming_a_project_does_not_disturb_its_usage_history");
    };

    let fixture = create_org(&pool, "rename-analytics").await;
    let team = repo::create_team(&pool, fixture.org_id, "genesis", None, None)
        .await
        .expect("team creation");

    let event = UsageEvent::new(
        uuid::Uuid::new_v4(),
        fixture.org_id,
        None,
        Some(team.id),
        "gpt-4o".into(),
        "gpt-4o-mini".into(),
        "openai".into(),
        TokenUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            estimated: false,
            ..Default::default()
        },
        SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), 2_000),
        200,
        0.4,
        CacheOutcome::Miss,
        RoutingReason::Complexity,
        Some(0.2),
        200,
    );
    repo::insert_usage_record(&pool, &event)
        .await
        .expect("insert");

    let from = Utc::now() - Duration::hours(1);
    let to = Utc::now() + Duration::hours(1);
    let before = repo::usage_summary_for_team(&pool, fixture.org_id, team.id, from, to)
        .await
        .expect("query");
    assert_eq!(before.requests, 1);

    let renamed = repo::update_team(&pool, fixture.org_id, team.id, "renamed-project")
        .await
        .expect("query")
        .expect("team exists");
    assert_eq!(
        renamed.id, team.id,
        "renaming must not change the project's identity"
    );
    assert_eq!(renamed.name, "renamed-project");

    let after = repo::usage_summary_for_team(&pool, fixture.org_id, team.id, from, to)
        .await
        .expect("query");
    assert_eq!(after.requests, before.requests, "requests");
    assert_eq!(
        after.baseline_cost_mc, before.baseline_cost_mc,
        "baseline_cost_mc"
    );
    assert_eq!(
        after.actual_cost_mc, before.actual_cost_mc,
        "actual_cost_mc"
    );
    assert_eq!(
        after.gross_savings_mc, before.gross_savings_mc,
        "gross_savings_mc"
    );

    cleanup(&pool, &fixture).await;
}

/// The table's real `UNIQUE (org_id, name)` constraint must surface as a clean, expected
/// error a client can act on, not a raw database error.
#[tokio::test]
async fn renaming_a_project_to_a_name_already_taken_is_a_clean_conflict_not_a_500() {
    let Some((_, pool)) = setup().await else {
        return skip("renaming_a_project_to_a_name_already_taken_is_a_clean_conflict_not_a_500");
    };

    let fixture = create_org(&pool, "rename-conflict").await;
    repo::create_team(&pool, fixture.org_id, "alpha", None, None)
        .await
        .expect("team creation");
    let bravo = repo::create_team(&pool, fixture.org_id, "bravo", None, None)
        .await
        .expect("team creation");

    match repo::update_team(&pool, fixture.org_id, bravo.id, "alpha").await {
        Err(aegis_gateway::error::AegisError::BadRequest(msg)) => {
            assert!(msg.contains("already exists"), "{msg}");
        }
        other => panic!("expected a clean BadRequest conflict, got {other:?}"),
    }

    cleanup(&pool, &fixture).await;
}

// -----------------------------------------------------------------------------
// Request log, filterable by project — `repo::list_requests`'s new `team_id` parameter.
// -----------------------------------------------------------------------------

/// Two projects in the same org, one request each: filtering by a project's id returns
/// only that project's row, and no filter still returns both — this is an org-wide log
/// narrowed by an optional filter, not two different code paths that could disagree.
#[tokio::test]
async fn the_request_log_can_be_narrowed_to_one_project() {
    use aegis_gateway::metering::savings::SavingsBreakdown;
    use aegis_gateway::metering::usage::UsageEvent;
    use aegis_gateway::money::MicroCents;
    use aegis_gateway::types::{CacheOutcome, RoutingReason, TokenUsage};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    let Some((_, pool)) = setup().await else {
        return skip("the_request_log_can_be_narrowed_to_one_project");
    };

    let fixture = create_org(&pool, "request-log-by-project").await;
    let alpha = repo::create_team(&pool, fixture.org_id, "alpha", None, None)
        .await
        .expect("team creation");
    let bravo = repo::create_team(&pool, fixture.org_id, "bravo", None, None)
        .await
        .expect("team creation");

    let make_event = |team_id: Uuid| {
        UsageEvent::new(
            uuid::Uuid::new_v4(),
            fixture.org_id,
            None,
            Some(team_id),
            "gpt-4o".into(),
            "gpt-4o-mini".into(),
            "openai".into(),
            TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                estimated: false,
                ..Default::default()
            },
            SavingsBreakdown::compute(MicroCents(750), MicroCents(45), 2_000),
            120,
            0.3,
            CacheOutcome::Miss,
            RoutingReason::Complexity,
            Some(0.1),
            200,
        )
    };
    repo::insert_usage_record(&pool, &make_event(alpha.id))
        .await
        .expect("insert");
    repo::insert_usage_record(&pool, &make_event(bravo.id))
        .await
        .expect("insert");

    let from = Utc::now() - Duration::hours(1);
    let to = Utc::now() + Duration::hours(1);

    let alpha_only = repo::list_requests(&pool, fixture.org_id, from, to, 100, 0, Some(alpha.id))
        .await
        .expect("query");
    assert_eq!(alpha_only.len(), 1);
    assert_eq!(alpha_only[0].team_id, Some(alpha.id));

    let bravo_only = repo::list_requests(&pool, fixture.org_id, from, to, 100, 0, Some(bravo.id))
        .await
        .expect("query");
    assert_eq!(bravo_only.len(), 1);
    assert_eq!(bravo_only[0].team_id, Some(bravo.id));

    let unfiltered = repo::list_requests(&pool, fixture.org_id, from, to, 100, 0, None)
        .await
        .expect("query");
    assert_eq!(
        unfiltered.len(),
        2,
        "no team_id filter must return the whole org's log, same as before this filter existed"
    );

    cleanup(&pool, &fixture).await;
}

// -----------------------------------------------------------------------------
// TOTP two-factor authentication, end to end against real handlers and a real database.
//
// The algorithm was always correct in isolation (`enterprise::totp`, unit-tested with the
// RFC 6238 test vector); what did not exist was everything connecting it to a real login —
// no repo function read or wrote the secret column, no enrollment endpoint, and login
// never checked `totp_enabled`. This exercises the whole path a real account would go
// through: enroll, confirm with a real generated code, get blocked at login without one,
// log in with one, then disable and confirm login reverts to password-only. Found in the
// enterprise readiness audit.
// -----------------------------------------------------------------------------

/// Give a fixture a real, known password. `common::create_org` seeds an unusable
/// placeholder hash — fine for tests that never call `login`, wrong for these.
async fn set_known_password(pool: &sqlx::PgPool, fixture: &common::Fixture, password: &str) {
    use aegis_gateway::crypto;
    let hash = crypto::hash_password(password).expect("hash");
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(hash)
        .bind(fixture.user_id)
        .execute(pool)
        .await
        .expect("set password");
}

/// Generate a currently-valid code for a secret, exactly as an authenticator app would.
fn code_for(secret: &str) -> String {
    use aegis_gateway::enterprise::totp;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    totp::generate_code(secret, now).expect("generate code")
}

#[tokio::test]
async fn totp_protects_login_end_to_end() {
    use aegis_gateway::routes::management;
    use axum::extract::{Json, State};
    use axum::http::StatusCode;

    let Some((state, pool)) = setup().await else {
        return skip("totp_protects_login_end_to_end");
    };

    let fixture = create_org(&pool, "totp-flow").await;
    let password = "correct horse battery staple 42";
    set_known_password(&pool, &fixture, password).await;
    let session_headers = owner_session_headers(&pool, &fixture).await;

    // Before enrollment: an ordinary password login succeeds with no code.
    let response = management::login(
        State(state.clone()),
        axum::http::HeaderMap::new(),
        Json(management::LoginRequest {
            email: fixture.email.clone(),
            password: password.to_string(),
            totp_code: None,
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a password login must succeed before 2FA is ever enabled"
    );

    // Enroll. This stores a secret but must not enable enforcement yet.
    let response = management::totp_enroll(State(state.clone()), session_headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = body_json(response).await;
    let secret = body["secret"]
        .as_str()
        .expect("secret in response")
        .to_string();
    assert!(
        body["provisioning_uri"]
            .as_str()
            .unwrap_or_default()
            .starts_with("otpauth://totp/"),
        "must return a scannable provisioning URI"
    );

    let user_after_enroll = repo::find_user_by_id(&pool, fixture.user_id)
        .await
        .expect("query")
        .expect("user exists");
    assert!(
        !user_after_enroll.totp_enabled,
        "enrolling alone must not enable enforcement — only a confirmed code does"
    );

    // Confirm with the wrong code: must not enable it.
    let response = management::totp_confirm(
        State(state.clone()),
        session_headers.clone(),
        Json(management::TotpConfirmRequest {
            code: "000000".to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Confirm with the real code: now it is enabled.
    let response = management::totp_confirm(
        State(state.clone()),
        session_headers.clone(),
        Json(management::TotpConfirmRequest {
            code: code_for(&secret),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let user_after_confirm = repo::find_user_by_id(&pool, fixture.user_id)
        .await
        .expect("query")
        .expect("user exists");
    assert!(user_after_confirm.totp_enabled);

    // Login with the correct password and no code: this is the actual enforcement check.
    // A stolen password must no longer be sufficient on its own.
    let response = management::login(
        State(state.clone()),
        axum::http::HeaderMap::new(),
        Json(management::LoginRequest {
            email: fixture.email.clone(),
            password: password.to_string(),
            totp_code: None,
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "correct password with no TOTP code must be refused once 2FA is enabled"
    );
    let body: serde_json::Value = body_json(response).await;
    assert_eq!(
        body["error"]["type"], "totp_required",
        "the client must be able to tell 'need a code' apart from 'wrong password': {body:?}"
    );

    // Login with the correct password and a stale/wrong code: still refused.
    let response = management::login(
        State(state.clone()),
        axum::http::HeaderMap::new(),
        Json(management::LoginRequest {
            email: fixture.email.clone(),
            password: password.to_string(),
            totp_code: Some("111111".to_string()),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Login with the correct password and a valid code: succeeds.
    let response = management::login(
        State(state.clone()),
        axum::http::HeaderMap::new(),
        Json(management::LoginRequest {
            email: fixture.email.clone(),
            password: password.to_string(),
            totp_code: Some(code_for(&secret)),
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "correct password + correct code must succeed"
    );
    assert!(
        response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .is_some(),
        "a successful TOTP login must still issue a session cookie"
    );

    // Disabling requires the password again, not just the session.
    let response = management::totp_disable(
        State(state.clone()),
        session_headers.clone(),
        Json(management::TotpDisableRequest {
            password: "definitely the wrong password".to_string(),
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a session alone must not be enough to turn 2FA off"
    );

    let response = management::totp_disable(
        State(state.clone()),
        session_headers.clone(),
        Json(management::TotpDisableRequest {
            password: password.to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Login now succeeds with no code again — enforcement genuinely turned off, and the
    // old secret cannot be reused: enrolling fresh would start over, not resurrect it.
    let response = management::login(
        State(state.clone()),
        axum::http::HeaderMap::new(),
        Json(management::LoginRequest {
            email: fixture.email.clone(),
            password: password.to_string(),
            totp_code: None,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let user_after_disable = repo::find_user_by_id(&pool, fixture.user_id)
        .await
        .expect("query")
        .expect("user exists");
    assert!(!user_after_disable.totp_enabled);

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn totp_confirm_without_enrolling_first_is_rejected() {
    let Some((state, pool)) = setup().await else {
        return skip("totp_confirm_without_enrolling_first_is_rejected");
    };
    use aegis_gateway::routes::management;
    use axum::extract::{Json, State};

    let fixture = create_org(&pool, "totp-no-enroll").await;
    let headers = owner_session_headers(&pool, &fixture).await;

    let response = management::totp_confirm(
        State(state),
        headers,
        Json(management::TotpConfirmRequest {
            code: "123456".to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn an_api_key_cannot_manage_totp() {
    // TOTP protects an account login, which an API key was never part of issuing.
    // Accepting one here would let a leaked, lower-privilege credential manage the very
    // control meant to protect against a leaked credential.
    let Some((state, pool)) = setup().await else {
        return skip("an_api_key_cannot_manage_totp");
    };
    use aegis_gateway::routes::management;
    use axum::extract::State;

    let fixture = create_org(&pool, "totp-api-key").await;
    let (plaintext, _key_id) = create_key(&pool, &fixture, "no-totp-access").await;

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_str(&format!("Bearer {plaintext}")).unwrap(),
    );

    let response = management::totp_enroll(State(state), headers).await;
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);

    cleanup(&pool, &fixture).await;
}

/// Parse a response body as JSON, for assertions that need to look inside it.
async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

// -----------------------------------------------------------------------------
// SCIM token self-service, end to end against real handlers and a real database.
//
// repo::create_scim_token existed and was tested in isolation; nothing in the management
// API ever called it, so a customer wanting SCIM provisioning had no way to get a token
// without a direct database write on our side. Found in the enterprise readiness audit.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn scim_tokens_can_be_issued_listed_and_revoked() {
    use aegis_gateway::routes::management;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;

    let Some((state, pool)) = setup().await else {
        return skip("scim_tokens_can_be_issued_listed_and_revoked");
    };

    let fixture = create_org(&pool, "scim-self-service").await;
    let headers = owner_session_headers(&pool, &fixture).await;

    // Issue one. The plaintext is only ever in this one response.
    let response = management::create_scim_token(State(state.clone()), headers.clone()).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    let token = body["token"].as_str().expect("plaintext token in response");
    assert!(
        token.starts_with("aegis_scim_"),
        "a SCIM token must be visually distinguishable from an ordinary API key: {token}"
    );
    let token_id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // It actually authenticates a SCIM call.
    let scim_headers = {
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        h
    };
    let response = aegis_gateway::routes::enterprise::scim_list_users(
        State(state.clone()),
        scim_headers.clone(),
        axum::extract::Query(aegis_gateway::routes::enterprise::ScimQuery {
            filter: None,
            start_index: None,
            count: None,
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a freshly issued token must authenticate a real SCIM call"
    );

    // It shows up in the listing, without the hash.
    let response = management::list_scim_tokens(State(state.clone()), headers.clone()).await;
    let listed = body_json(response).await;
    let tokens = listed.as_array().expect("array response");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["id"], token_id.to_string());
    assert!(tokens[0]["revoked_at"].is_null());
    assert!(
        serde_json::to_string(&tokens[0]).unwrap().len() < 200,
        "the listing must not carry the token hash or plaintext"
    );

    // Revoke it.
    let response =
        management::revoke_scim_token(State(state.clone()), headers.clone(), Path(token_id)).await;
    assert_eq!(response.status(), StatusCode::OK);

    // It no longer authenticates.
    let response = aegis_gateway::routes::enterprise::scim_list_users(
        State(state.clone()),
        scim_headers,
        axum::extract::Query(aegis_gateway::routes::enterprise::ScimQuery {
            filter: None,
            start_index: None,
            count: None,
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a revoked SCIM token must stop authenticating immediately"
    );

    // Revoking it again is a clean 404, not a silent no-op or a 500.
    let response = management::revoke_scim_token(State(state), headers, Path(token_id)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn a_scim_token_is_scoped_to_the_organisation_that_issued_it() {
    use aegis_gateway::routes::management;
    use axum::extract::{Path, State};

    let Some((state, pool)) = setup().await else {
        return skip("a_scim_token_is_scoped_to_the_organisation_that_issued_it");
    };

    let owner = create_org(&pool, "scim-scope-owner").await;
    let other = create_org(&pool, "scim-scope-other").await;

    let response = management::create_scim_token(
        State(state.clone()),
        owner_session_headers(&pool, &owner).await,
    )
    .await;
    let body = body_json(response).await;
    let token_id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // The *other* organisation cannot revoke a token it does not own, even knowing its id.
    let response = management::revoke_scim_token(
        State(state),
        owner_session_headers(&pool, &other).await,
        Path(token_id),
    )
    .await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::NOT_FOUND,
        "cross-tenant revocation must fail exactly like a nonexistent token, not leak that \
         a token with this id exists elsewhere"
    );

    cleanup(&pool, &owner).await;
    cleanup(&pool, &other).await;
}

// -----------------------------------------------------------------------------
// Audit log export and admin-scoped investigation, end to end against real handlers and
// a real database.
//
// /api/admin/audit existed but could only ever show the calling admin's own
// organisation's log -- useless for its stated purpose (staff investigating a customer).
// No customer-facing audit export existed at all, despite the compliance whitepaper
// describing one. Found in the enterprise readiness audit.
// -----------------------------------------------------------------------------

async fn make_platform_admin(pool: &sqlx::PgPool, fixture: &common::Fixture) {
    sqlx::query("UPDATE users SET is_admin = true WHERE id = $1")
        .bind(fixture.user_id)
        .execute(pool)
        .await
        .expect("grant admin");
}

#[tokio::test]
async fn a_customer_can_export_their_own_audit_log() {
    use aegis_gateway::routes::management;
    use axum::extract::{Query, State};

    let Some((state, pool)) = setup().await else {
        return skip("a_customer_can_export_their_own_audit_log");
    };

    let fixture = create_org(&pool, "audit-export").await;
    let headers = owner_session_headers(&pool, &fixture).await;

    // Do something that actually writes an audit entry.
    let _ = management::create_scim_token(State(state.clone()), headers.clone()).await;

    let response = management::audit_log_export(
        State(state),
        headers,
        Query(management::AuditLogQuery {
            limit: None,
            offset: None,
        }),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap(),
        "application/x-ndjson; charset=utf-8"
    );

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert!(!lines.is_empty(), "expected at least one audit entry");
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line was not valid JSON ({e}): {line}"));
        assert_eq!(
            parsed["org_id"],
            fixture.org_id.to_string(),
            "every line must belong to the exporting organisation"
        );
    }

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn an_ordinary_member_cannot_reach_the_admin_audit_endpoint() {
    let Some((state, pool)) = setup().await else {
        return skip("an_ordinary_member_cannot_reach_the_admin_audit_endpoint");
    };
    use aegis_gateway::routes::admin;
    use axum::extract::{Query, State};

    let fixture = create_org(&pool, "not-an-admin").await;
    let headers = owner_session_headers(&pool, &fixture).await;

    let response = admin::audit_log(
        State(state),
        headers,
        Query(admin::AdminAuditQuery {
            org_id: None,
            limit: None,
            offset: None,
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::NOT_FOUND,
        "an org owner who is not platform staff must not reach the admin surface"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn a_platform_admin_can_inspect_a_different_organisations_audit_log() {
    // The actual fix: an admin's own org membership must not determine which customer
    // they can investigate.
    use aegis_gateway::routes::{admin, management};
    use axum::extract::{Query, State};

    let Some((state, pool)) = setup().await else {
        return skip("a_platform_admin_can_inspect_a_different_organisations_audit_log");
    };

    let staff = create_org(&pool, "staff-account").await;
    make_platform_admin(&pool, &staff).await;
    let staff_headers = owner_session_headers(&pool, &staff).await;

    let customer = create_org(&pool, "customer-being-investigated").await;
    let customer_headers = owner_session_headers(&pool, &customer).await;
    // Give the customer's org a real audit entry to find.
    let _ = management::create_scim_token(State(state.clone()), customer_headers.clone()).await;

    // Without org_id: defaults to the admin's own org, which has no entries.
    let response = admin::audit_log(
        State(state.clone()),
        staff_headers.clone(),
        Query(admin::AdminAuditQuery {
            org_id: None,
            limit: None,
            offset: None,
        }),
    )
    .await;
    let body = body_json(response).await;
    assert_eq!(body["org_id"], staff.org_id.to_string());
    assert!(
        body["entries"].as_array().unwrap().is_empty(),
        "the staff account's own org has no activity"
    );

    // With org_id set to the customer: this is the fix. Before it, there was no way to
    // ever reach this data through the endpoint at all.
    let response = admin::audit_log(
        State(state),
        staff_headers,
        Query(admin::AdminAuditQuery {
            org_id: Some(customer.org_id),
            limit: None,
            offset: None,
        }),
    )
    .await;
    let body = body_json(response).await;
    assert_eq!(body["org_id"], customer.org_id.to_string());
    assert!(
        !body["entries"].as_array().unwrap().is_empty(),
        "a platform admin must be able to inspect a customer's audit log by org_id"
    );

    cleanup(&pool, &staff).await;
    cleanup(&pool, &customer).await;
}

#[tokio::test]
async fn invite_and_accept_lifecycle_works() {
    let Some((state, pool)) = setup().await else {
        return skip("invite_and_accept_lifecycle_works");
    };

    use aegis_gateway::routes::management;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::Json;

    let fixture = create_org(&pool, "invite_accept").await;
    // Member invitation is a Team-plan feature (MASTER_BUILD.md frames Free/Pro as
    // individual-developer plans) — this test is about the invite/accept lifecycle itself,
    // not the plan gate, so upgrade the fixture rather than let an unrelated 403 mask it.
    repo::update_org_plan(&pool, fixture.org_id, "team", 1_500)
        .await
        .expect("plan upgrade");
    let owner_headers = owner_session_headers(&pool, &fixture).await;

    // 1. Invite a new colleague
    let invite_email = format!("colleague_{}@aegis.test", uuid::Uuid::new_v4());
    let response = management::invite_member(
        State(state.clone()),
        owner_headers,
        Json(management::InviteRequest {
            email: invite_email.clone(),
            role: "admin".into(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    assert_eq!(body["invited"], invite_email);
    assert_eq!(body["role"], "admin");
    let invite_url = body["invite_url"].as_str().expect("must have invite_url");
    assert!(invite_url.contains("/join?token="));

    // Extract token from URL
    let token = invite_url
        .split("token=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();

    // 2. Colleague accepts invite and sets password (min 12 chars)
    let new_password = "secure_password_123456";
    let accept_res = management::accept_invite(
        State(state.clone()),
        Json(management::AcceptInviteRequest {
            token: token.to_string(),
            password: new_password.to_string(),
            name: Some("Invited Admin".into()),
        }),
    )
    .await;
    assert_eq!(accept_res.status(), StatusCode::OK);
    let accept_body = body_json(accept_res).await;
    assert_eq!(accept_body["user"]["email"], invite_email);
    assert_eq!(accept_body["user"]["name"], "Invited Admin");

    // 3. Colleague can now log in with their newly set password
    let login_res = management::login(
        State(state.clone()),
        HeaderMap::new(),
        Json(management::LoginRequest {
            email: invite_email.clone(),
            password: new_password.to_string(),
            totp_code: None,
        }),
    )
    .await;
    assert_eq!(login_res.status(), StatusCode::OK);

    // 4. Second redemption of the exact same token MUST be rejected
    let second_accept = management::accept_invite(
        State(state),
        Json(management::AcceptInviteRequest {
            token: token.to_string(),
            password: "another_password_123456".to_string(),
            name: None,
        }),
    )
    .await;
    assert_eq!(second_accept.status(), StatusCode::BAD_REQUEST);

    cleanup(&pool, &fixture).await;
}

// ---------------------------------------------------------------------------
// Per-person attribution, against a real database.
//
// The unit tests prove `AuthContext` carries the assignee. These prove the column
// mapping all the way to `usage_records` and back, which is exactly the class of
// bug `docs/adr/0004-runtime-checked-sql.md` accepts cannot be caught at compile
// time.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_key_issued_to_a_person_attributes_its_usage_to_them() {
    let Some((state, pool)) = setup().await else {
        return skip("a_key_issued_to_a_person_attributes_its_usage_to_them");
    };

    use aegis_gateway::middleware::auth;
    use common::create_key_assigned;

    let fixture = create_org(&pool, "attribution").await;
    let (plaintext, _key_id) =
        create_key_assigned(&pool, &fixture, "alice-laptop", Some(fixture.user_id)).await;

    // The assignee survives the round trip through the key-resolution query.
    let context = auth::authenticate_api_key(&state, &plaintext)
        .await
        .expect("an assigned key must still authenticate");
    assert_eq!(
        context.user_id,
        Some(fixture.user_id),
        "the key's assignee must reach AuthContext through the real resolve_key query"
    );

    // And it reaches the billing record.
    let request_id = uuid::Uuid::new_v4();
    let mut event = aegis_gateway::metering::usage::UsageEvent::rejected(
        request_id,
        fixture.org_id,
        context.api_key_id,
        "gpt-4o".into(),
        200,
        "none",
        0.4,
    );
    event.user_id = context.user_id;
    repo::insert_usage_record(&pool, &event)
        .await
        .expect("usage insert");

    let stored: Option<(Option<uuid::Uuid>,)> =
        sqlx::query_as("SELECT user_id FROM usage_records WHERE request_id = $1")
            .bind(request_id)
            .fetch_optional(&pool)
            .await
            .expect("usage read");

    assert_eq!(
        stored.expect("the record must exist").0,
        Some(fixture.user_id),
        "user_id must persist to usage_records, or per-person spend cannot be reported"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn a_shared_key_records_no_person() {
    let Some((state, pool)) = setup().await else {
        return skip("a_shared_key_records_no_person");
    };

    use aegis_gateway::middleware::auth;

    let fixture = create_org(&pool, "shared-key").await;
    let (plaintext, _) = create_key(&pool, &fixture, "project-shared").await;

    let context = auth::authenticate_api_key(&state, &plaintext)
        .await
        .expect("a shared key must authenticate");
    assert_eq!(
        context.user_id, None,
        "an unassigned key must not invent a person — NULL is the correct answer"
    );

    cleanup(&pool, &fixture).await;
}

#[tokio::test]
async fn a_member_sees_only_their_own_keys_and_the_shared_ones() {
    // The permission boundary that per-person assignment creates: once a key names a
    // person and carries their budget, listing every key in the org hands one colleague
    // another colleague's spending limit.
    let Some((_state, pool)) = setup().await else {
        return skip("a_member_sees_only_their_own_keys_and_the_shared_ones");
    };

    use common::create_key_assigned;

    let fixture = create_org(&pool, "key-visibility").await;

    // A second person in the same organisation.
    let colleague = repo::create_user(
        &pool,
        &format!("colleague-{}@aegis-test.local", uuid::Uuid::new_v4()),
        Some("hash"),
        None,
    )
    .await
    .expect("colleague");
    repo::add_member(&pool, fixture.org_id, colleague.id, "member", None)
        .await
        .expect("membership");

    let (_, mine) = create_key_assigned(&pool, &fixture, "mine", Some(fixture.user_id)).await;
    let (_, theirs) = create_key_assigned(&pool, &fixture, "theirs", Some(colleague.id)).await;
    let (_, shared) = create_key(&pool, &fixture, "shared").await;

    let visible = repo::list_api_keys_for_member(&pool, fixture.org_id, fixture.user_id)
        .await
        .expect("scoped list");
    let ids: Vec<uuid::Uuid> = visible.iter().map(|k| k.id).collect();

    assert!(ids.contains(&mine), "a member must see their own key");
    assert!(
        ids.contains(&shared),
        "a member must see shared project keys"
    );
    assert!(
        !ids.contains(&theirs),
        "a member must NOT see a colleague's personal key"
    );

    // An admin still sees everything — the narrow view is a member restriction, not a
    // hole in administration.
    let all = repo::list_api_keys(&pool, fixture.org_id)
        .await
        .expect("admin list");
    let all_ids: Vec<uuid::Uuid> = all.iter().map(|k| k.id).collect();
    assert!(all_ids.contains(&theirs), "an admin must see every key");

    cleanup(&pool, &fixture).await;
}

// -----------------------------------------------------------------------------
// Onboarding tour completion.
// -----------------------------------------------------------------------------

/// A fresh signup has never completed onboarding; marking it complete persists (so
/// `GET /api/auth/me` reflects it on a later request, not just the response to the call
/// that set it) and is idempotent — replaying the tour from Settings and finishing it again
/// must not error just because it was already marked once.
#[tokio::test]
async fn completing_onboarding_persists_and_is_idempotent() {
    use aegis_gateway::routes::management;
    use axum::extract::State;

    let Some((state, pool)) = setup().await else {
        return skip("completing_onboarding_persists_and_is_idempotent");
    };

    let fixture = create_org(&pool, "onboarding").await;

    let before = repo::find_user_by_id(&pool, fixture.user_id)
        .await
        .expect("query")
        .expect("user exists");
    assert!(
        before.onboarding_completed_at.is_none(),
        "a fresh signup must not already be marked onboarded"
    );

    let headers = owner_session_headers(&pool, &fixture).await;
    let response = management::complete_onboarding(State(state.clone()), headers.clone()).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let after = repo::find_user_by_id(&pool, fixture.user_id)
        .await
        .expect("query")
        .expect("user exists");
    assert!(after.onboarding_completed_at.is_some());

    // Replaying the tour and finishing it again must not error.
    let response_again = management::complete_onboarding(State(state), headers).await;
    assert_eq!(response_again.status(), axum::http::StatusCode::OK);

    cleanup(&pool, &fixture).await;
}

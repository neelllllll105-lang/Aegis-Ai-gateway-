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
        repo::update_org_settings(&pool, fixture.org_id, None, None, None, Some(true))
            .await
            .expect("enable capture");
    assert!(with_capture.content_capture);

    let private = repo::update_org_settings(&pool, fixture.org_id, None, None, Some(true), None)
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
/// handler with a real database: a freshly signed-up organisation — the same access any
/// free-tier signup gets automatically, no plan gate, no review — attempts to register a
/// BYOK "custom provider" pointed at the cloud metadata service. Before
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

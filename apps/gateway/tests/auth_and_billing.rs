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

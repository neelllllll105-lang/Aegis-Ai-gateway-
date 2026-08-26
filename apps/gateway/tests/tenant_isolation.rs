//! Cross-tenant isolation tests.
//!
//! `MASTER_BUILD.md` Part 13 item 2 calls a cross-tenant leak company-ending, and Part 9
//! item 4 requires integration tests that attempt cross-tenant access and confirm it
//! fails. This file is that requirement.
//!
//! Each test creates two genuinely separate organisations and has one deliberately try to
//! reach the other's data through the real repository functions — the same code the API
//! handlers call. A test that only checked a `WHERE` clause by reading source would not
//! catch a handler that bypasses the repository; these go through the actual query path.

mod common;

use common::{cleanup, create_key, create_org, repo, setup, skip};

#[tokio::test]
async fn an_organisation_cannot_read_another_organisations_api_key() {
    let Some((_, pool)) = setup().await else {
        return skip("an_organisation_cannot_read_another_organisations_api_key");
    };

    let victim = create_org(&pool, "victim").await;
    let attacker = create_org(&pool, "attacker").await;
    let (_, victim_key_id) = create_key(&pool, &victim, "victim-production").await;

    // The victim can see their own key.
    let own = repo::find_api_key(&pool, victim.org_id, victim_key_id)
        .await
        .expect("query");
    assert!(own.is_some(), "an org must be able to read its own key");

    // The attacker, knowing the exact key id, gets nothing.
    let stolen = repo::find_api_key(&pool, attacker.org_id, victim_key_id)
        .await
        .expect("query");
    assert!(
        stolen.is_none(),
        "CROSS-TENANT LEAK: one organisation read another organisation's API key"
    );

    cleanup(&pool, &victim).await;
    cleanup(&pool, &attacker).await;
}

#[tokio::test]
async fn an_organisation_cannot_revoke_another_organisations_key() {
    let Some((_, pool)) = setup().await else {
        return skip("an_organisation_cannot_revoke_another_organisations_key");
    };

    let victim = create_org(&pool, "victim-revoke").await;
    let attacker = create_org(&pool, "attacker-revoke").await;
    let (_, victim_key_id) = create_key(&pool, &victim, "production").await;

    let revoked = repo::revoke_api_key(&pool, attacker.org_id, victim_key_id)
        .await
        .expect("query");
    assert!(
        !revoked,
        "an org revoked another org's key — denial of service"
    );

    // And the key still works for its owner.
    let key = repo::find_api_key(&pool, victim.org_id, victim_key_id)
        .await
        .expect("query")
        .expect("key still exists");
    assert!(
        key.is_usable(),
        "the victim's key was disabled by another tenant"
    );

    cleanup(&pool, &victim).await;
    cleanup(&pool, &attacker).await;
}

#[tokio::test]
async fn an_organisation_cannot_read_another_organisations_provider_credential() {
    // The most sensitive data we hold: a customer's own provider key.
    let Some((_, pool)) = setup().await else {
        return skip("an_organisation_cannot_read_another_organisations_provider_credential");
    };

    let victim = create_org(&pool, "victim-cred").await;
    let attacker = create_org(&pool, "attacker-cred").await;

    let credential = repo::create_credential(
        &pool,
        victim.org_id,
        "openai",
        b"encrypted-bytes-here",
        Some("...abcd"),
        None,
        Some("production"),
        true,
    )
    .await
    .expect("credential creation");

    let stolen = repo::find_credential(&pool, attacker.org_id, credential.id)
        .await
        .expect("query");
    assert!(
        stolen.is_none(),
        "CROSS-TENANT LEAK: one organisation read another organisation's provider credential"
    );

    let deleted = repo::delete_credential(&pool, attacker.org_id, credential.id)
        .await
        .expect("query");
    assert!(!deleted, "an org deleted another org's provider credential");

    cleanup(&pool, &victim).await;
    cleanup(&pool, &attacker).await;
}

#[tokio::test]
async fn usage_records_are_never_visible_across_tenants() {
    let Some((state, pool)) = setup().await else {
        return skip("usage_records_are_never_visible_across_tenants");
    };

    use aegis_gateway::metering::savings::SavingsBreakdown;
    use aegis_gateway::metering::usage::UsageEvent;
    use aegis_gateway::money::MicroCents;
    use aegis_gateway::types::{CacheOutcome, RoutingReason, TokenUsage};
    use chrono::{Duration, Utc};

    let victim = create_org(&pool, "victim-usage").await;
    let attacker = create_org(&pool, "attacker-usage").await;

    let event = UsageEvent::new(
        uuid::Uuid::new_v4(),
        victim.org_id,
        None,
        None,
        "gpt-4o".into(),
        "gpt-4o-mini".into(),
        "openai".into(),
        TokenUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            estimated: false,
        },
        SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), 2_000),
        200,
        0.4,
        CacheOutcome::Miss,
        RoutingReason::Complexity,
        Some(0.2),
        200,
    );
    let inserted = repo::insert_usage_record(&pool, &event)
        .await
        .expect("insert");
    assert!(inserted, "the usage record should have been written");

    let from = Utc::now() - Duration::hours(1);
    let to = Utc::now() + Duration::hours(1);

    let own = repo::usage_summary(&pool, victim.org_id, from, to)
        .await
        .expect("query");
    assert_eq!(own.requests, 1, "an org must see its own usage");

    let stolen = repo::usage_summary(&pool, attacker.org_id, from, to)
        .await
        .expect("query");
    assert_eq!(
        stolen.requests, 0,
        "CROSS-TENANT LEAK: usage from another organisation appeared in the summary"
    );

    let rows = repo::list_requests(&pool, attacker.org_id, from, to, 100, 0)
        .await
        .expect("query");
    assert!(
        rows.is_empty(),
        "CROSS-TENANT LEAK: another organisation's requests appeared in the log"
    );

    drop(state);
    cleanup(&pool, &victim).await;
    cleanup(&pool, &attacker).await;
}

#[tokio::test]
async fn a_user_cannot_reach_an_organisation_they_do_not_belong_to() {
    let Some((_, pool)) = setup().await else {
        return skip("a_user_cannot_reach_an_organisation_they_do_not_belong_to");
    };

    let victim = create_org(&pool, "victim-org").await;
    let outsider = create_org(&pool, "outsider-org").await;

    // The membership check is part of the query, so an outsider gets nothing even with a
    // valid organisation id.
    let reachable = repo::find_org_for_user(&pool, victim.org_id, outsider.user_id)
        .await
        .expect("query");
    assert!(
        reachable.is_none(),
        "CROSS-TENANT LEAK: a non-member resolved another organisation"
    );

    // The genuine member can.
    let own = repo::find_org_for_user(&pool, victim.org_id, victim.user_id)
        .await
        .expect("query");
    assert!(
        own.is_some(),
        "a member must be able to reach their own org"
    );

    cleanup(&pool, &victim).await;
    cleanup(&pool, &outsider).await;
}

#[tokio::test]
async fn teams_budgets_and_policies_are_all_org_scoped() {
    // A sweep across the remaining tenant-owned resources, so a newly added one is less
    // likely to be the one nobody checked.
    let Some((_, pool)) = setup().await else {
        return skip("teams_budgets_and_policies_are_all_org_scoped");
    };

    let victim = create_org(&pool, "victim-sweep").await;
    let attacker = create_org(&pool, "attacker-sweep").await;

    let team = repo::create_team(&pool, victim.org_id, "engineering", Some(1_000_000))
        .await
        .expect("team creation");
    let budget = repo::create_budget(
        &pool,
        repo::NewBudget {
            org_id: victim.org_id,
            team_id: None,
            api_key_id: None,
            region: None,
            period: "monthly",
            limit_mc: 500_000,
            hard_limit: true,
        },
    )
    .await
    .expect("budget creation");
    let policy = repo::create_policy(
        &pool,
        victim.org_id,
        "default",
        &serde_json::json!([{"when": {}, "then": {"passthrough": true}}]),
    )
    .await
    .expect("policy creation");

    assert!(
        repo::list_teams(&pool, attacker.org_id)
            .await
            .expect("query")
            .is_empty(),
        "CROSS-TENANT LEAK: teams"
    );
    assert!(
        repo::list_budgets(&pool, attacker.org_id)
            .await
            .expect("query")
            .is_empty(),
        "CROSS-TENANT LEAK: budgets"
    );
    assert!(
        repo::list_policies(&pool, attacker.org_id)
            .await
            .expect("query")
            .is_empty(),
        "CROSS-TENANT LEAK: policies"
    );

    assert!(!repo::delete_team(&pool, attacker.org_id, team.id)
        .await
        .expect("query"));
    assert!(!repo::delete_budget(&pool, attacker.org_id, budget.id)
        .await
        .expect("query"));
    assert!(!repo::delete_policy(&pool, attacker.org_id, policy.id)
        .await
        .expect("query"));

    // The owner still has all three.
    assert_eq!(
        repo::list_teams(&pool, victim.org_id)
            .await
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        repo::list_budgets(&pool, victim.org_id)
            .await
            .expect("query")
            .len(),
        1
    );
    assert_eq!(
        repo::list_policies(&pool, victim.org_id)
            .await
            .expect("query")
            .len(),
        1
    );

    cleanup(&pool, &victim).await;
    cleanup(&pool, &attacker).await;
}

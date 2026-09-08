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
use uuid::Uuid;

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

    let rows = repo::list_requests(&pool, attacker.org_id, from, to, 100, 0, None)
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

    let team = repo::create_team(&pool, victim.org_id, "engineering", Some(1_000_000), None)
        .await
        .expect("team creation");
    let budget = repo::create_budget(
        &pool,
        repo::NewBudget {
            org_id: victim.org_id,
            team_id: None,
            api_key_id: None,
            user_id: None,
            region: None,
            period: "monthly",
            limit_mc: 500_000,
            limit_tokens: None,
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
    assert!(
        repo::update_team(&pool, attacker.org_id, team.id, "pwned")
            .await
            .expect("query")
            .is_none(),
        "CROSS-TENANT LEAK: an attacker organisation renamed a victim's project"
    );
    // The name really is untouched, not just the row count.
    assert_eq!(
        repo::list_teams(&pool, victim.org_id).await.expect("query")[0].name,
        "engineering"
    );

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

#[tokio::test]
async fn a_project_lead_is_only_a_lead_of_the_team_they_were_added_to() {
    // team_memberships existed since migration 0001 with no write path anywhere in the
    // application — a team could be created and nobody could ever actually join it. This
    // proves the write path (added for IG-1 §1) and, more importantly, that leading one
    // team confers no authority over a different one, even within the same organisation.
    let Some((_, pool)) = setup().await else {
        return skip("a_project_lead_is_only_a_lead_of_the_team_they_were_added_to");
    };

    let org = create_org(&pool, "project-lead").await;
    let lead = repo::create_user(&pool, "lead@test.invalid", Some("$argon2id$fake"), None)
        .await
        .expect("user creation");
    repo::add_member(&pool, org.org_id, lead.id, "member", None)
        .await
        .expect("org membership");

    let led_team = repo::create_team(&pool, org.org_id, "led-project", None, None)
        .await
        .expect("team creation");
    let other_team = repo::create_team(&pool, org.org_id, "other-project", None, None)
        .await
        .expect("team creation");
    repo::add_team_member(&pool, led_team.id, lead.id, "lead")
        .await
        .expect("team membership");

    assert_eq!(
        repo::role_in_team(&pool, led_team.id, lead.id)
            .await
            .expect("query"),
        Some("lead".to_string())
    );
    assert_eq!(
        repo::role_in_team(&pool, other_team.id, lead.id)
            .await
            .expect("query"),
        None,
        "leading one project must not resolve as leading a different one"
    );

    // Changing role is an upsert, not a duplicate row.
    repo::add_team_member(&pool, led_team.id, lead.id, "member")
        .await
        .expect("role change");
    assert_eq!(
        repo::role_in_team(&pool, led_team.id, lead.id)
            .await
            .expect("query"),
        Some("member".to_string())
    );

    assert!(repo::remove_team_member(&pool, led_team.id, lead.id)
        .await
        .expect("query"));
    assert_eq!(
        repo::role_in_team(&pool, led_team.id, lead.id)
            .await
            .expect("query"),
        None
    );

    cleanup(&pool, &org).await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(lead.id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn team_membership_and_budgets_do_not_leak_across_teams_in_the_same_org() {
    let Some((_, pool)) = setup().await else {
        return skip("team_membership_and_budgets_do_not_leak_across_teams_in_the_same_org");
    };

    let org = create_org(&pool, "same-org-teams").await;
    let team_a = repo::create_team(&pool, org.org_id, "team-a", None, None)
        .await
        .expect("team creation");
    let team_b = repo::create_team(&pool, org.org_id, "team-b", None, None)
        .await
        .expect("team creation");

    // team_belongs_to_org is the guard every project-scoped handler runs first. Both teams
    // belong to this org; a random id must not.
    assert!(repo::team_belongs_to_org(&pool, team_a.id, org.org_id)
        .await
        .expect("query"));
    assert!(
        !repo::team_belongs_to_org(&pool, Uuid::new_v4(), org.org_id)
            .await
            .expect("query")
    );

    repo::create_budget(
        &pool,
        repo::NewBudget {
            org_id: org.org_id,
            team_id: Some(team_a.id),
            api_key_id: None,
            user_id: None,
            region: None,
            period: "monthly",
            limit_mc: 100_000,
            limit_tokens: Some(50_000),
            hard_limit: true,
        },
    )
    .await
    .expect("budget creation");

    let budgets = repo::list_budgets(&pool, org.org_id).await.expect("query");
    let team_a_budget = budgets
        .iter()
        .find(|b| b.team_id == Some(team_a.id))
        .expect("team_a's budget must be listed");
    assert_eq!(team_a_budget.limit_tokens, Some(50_000));
    assert!(
        !budgets.iter().any(|b| b.team_id == Some(team_b.id)),
        "a budget scoped to team_a must not also apply to team_b"
    );

    cleanup(&pool, &org).await;
}

#[tokio::test]
async fn a_user_scoped_budget_only_names_one_person() {
    let Some((_, pool)) = setup().await else {
        return skip("a_user_scoped_budget_only_names_one_person");
    };

    let org = create_org(&pool, "user-budget").await;
    let other = repo::create_user(&pool, "other@test.invalid", Some("$argon2id$fake"), None)
        .await
        .expect("user creation");
    repo::add_member(&pool, org.org_id, other.id, "member", None)
        .await
        .expect("org membership");

    repo::create_budget(
        &pool,
        repo::NewBudget {
            org_id: org.org_id,
            team_id: None,
            api_key_id: None,
            user_id: Some(org.user_id),
            region: None,
            period: "monthly",
            limit_mc: 25_000,
            limit_tokens: None,
            hard_limit: true,
        },
    )
    .await
    .expect("budget creation");

    let budgets = repo::list_budgets(&pool, org.org_id).await.expect("query");
    assert!(budgets.iter().any(|b| b.user_id == Some(org.user_id)));
    assert!(
        !budgets.iter().any(|b| b.user_id == Some(other.id)),
        "a budget scoped to one person must not resolve for a different one"
    );

    cleanup(&pool, &org).await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(other.id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn per_person_usage_summaries_only_include_that_persons_own_keys() {
    // The whole point of `assigned_to_user_id` and `usage_records.user_id`: "what did this
    // employee spend" must be answerable, and must not include a colleague's spend.
    let Some((_, pool)) = setup().await else {
        return skip("per_person_usage_summaries_only_include_that_persons_own_keys");
    };

    let org = create_org(&pool, "per-person-usage").await;
    let colleague = repo::create_user(
        &pool,
        "colleague@test.invalid",
        Some("$argon2id$fake"),
        None,
    )
    .await
    .expect("user creation");
    repo::add_member(&pool, org.org_id, colleague.id, "member", None)
        .await
        .expect("org membership");

    let now = chrono::Utc::now();
    let record_for = |user_id: Uuid, cost_mc: i64| aegis_gateway::metering::usage::UsageEvent {
        request_id: Uuid::new_v4(),
        org_id: org.org_id,
        api_key_id: None,
        team_id: None,
        user_id: Some(user_id),
        requested_model: "openai/gpt-4o".into(),
        served_model: "openai/gpt-4o-mini".into(),
        provider: "openai".into(),
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: 0,
        cache_write_tokens: 0,
        tokens_estimated: false,
        baseline_cost_mc: cost_mc * 2,
        actual_cost_mc: cost_mc,
        input_cost_mc: 0,
        output_cost_mc: 0,
        gross_savings_mc: cost_mc,
        aegis_fee_mc: 0,
        routing_savings_mc: cost_mc,
        compression_savings_mc: 0,
        cache_savings_mc: 0,
        latency_ms: 100,
        gateway_overhead_ms: 0.5,
        cache_hit: false,
        cache_type: None,
        routing_reason: "complexity".into(),
        complexity_score: None,
        status_code: 200,
        error_type: None,
        tokens_saved_by_compression: 0,
        techniques_fired: None,
        cache_bust_hits: 0,
        region: None,
        reserved_mc: 0,
        reserved_tokens: 0,
        created_at: now,
    };

    repo::insert_usage_record(&pool, &record_for(org.user_id, 1_000))
        .await
        .expect("insert");
    repo::insert_usage_record(&pool, &record_for(org.user_id, 2_000))
        .await
        .expect("insert");
    repo::insert_usage_record(&pool, &record_for(colleague.id, 9_000))
        .await
        .expect("insert");

    let from = now - chrono::Duration::hours(1);
    let to = now + chrono::Duration::hours(1);

    let owner_summary = repo::usage_summary_for_user(&pool, org.org_id, org.user_id, from, to)
        .await
        .expect("query");
    assert_eq!(owner_summary.requests, 2);
    assert_eq!(owner_summary.actual_cost_mc, 3_000);

    let colleague_summary = repo::usage_summary_for_user(&pool, org.org_id, colleague.id, from, to)
        .await
        .expect("query");
    assert_eq!(colleague_summary.requests, 1);
    assert_eq!(
        colleague_summary.actual_cost_mc, 9_000,
        "CROSS-PERSON LEAK: one employee's usage summary included a colleague's spend"
    );

    cleanup(&pool, &org).await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(colleague.id)
        .execute(&pool)
        .await;
}

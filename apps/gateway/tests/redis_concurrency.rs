//! Every "atomic under concurrency" proof this project makes, re-run against a real Redis
//! server rather than the in-process `MemoryStore`.
//!
//! # Why this file exists
//!
//! CI has provisioned a real Redis service container and set `AEGIS_TEST_REDIS_URL` since
//! this project's CI workflow was written. Until now, no test anywhere read that variable
//! — the rate limiter's atomicity, the scheduler's exactly-once distributed claim, and the
//! budget reservation's atomicity (the fix for the concurrent-overshoot bug the enterprise
//! readiness audit found and proved) had all only ever been demonstrated against
//! `MemoryStore`, whose atomicity comes from a single in-process `std::sync::Mutex` and
//! proves nothing about whether the equivalent Redis Lua scripts and `INCRBY` calls are
//! genuinely atomic against a real server under real network round trips.
//!
//! These are the identical properties, the identical assertions, and — where the
//! production code path allows it — the identical helper functions the unit tests already
//! exercise against `MemoryStore`. The only thing that changes is the backend.
//!
//! Skips gracefully without `AEGIS_TEST_REDIS_URL`, exactly like every database-gated test
//! in this suite skips without `AEGIS_TEST_DATABASE_URL`.

mod common;

use aegis_gateway::db::repo::KeyContext;
use aegis_gateway::middleware::auth::AuthContext;
use aegis_gateway::middleware::budget::{self, BudgetLimits, BudgetOutcome, Limit};
use aegis_gateway::store::KvStore;
use common::{skip_redis, test_redis_store};
use std::sync::Arc;
use std::time::Duration;
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

#[tokio::test]
async fn the_rate_limiter_is_atomic_against_real_redis() {
    // The exact property `store.rs`'s own unit test proves against MemoryStore: 50
    // concurrent callers, a limit of 10, exactly 10 admitted. A non-atomic
    // check-then-increment would over-admit under real concurrency; this is what the Lua
    // script is supposed to prevent, proven against the server that actually runs it in
    // production.
    let Some(store) = test_redis_store().await else {
        return skip_redis("the_rate_limiter_is_atomic_against_real_redis");
    };
    let store = Arc::new(store);

    // A key unique to this run, so a previous failed run's leftover window cannot affect
    // this one.
    let key = format!("test:ratelimit:{}", Uuid::new_v4());

    let mut handles = Vec::new();
    for _ in 0..50 {
        let store = Arc::clone(&store);
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            store
                .rate_limit(&key, 10, Duration::from_secs(60))
                .await
                .unwrap()
                .allowed
        }));
    }

    let mut admitted = 0;
    for handle in handles {
        if handle.await.unwrap() {
            admitted += 1;
        }
    }

    assert_eq!(
        admitted, 10,
        "the rate limiter over-admitted under concurrency against real Redis"
    );
}

#[tokio::test]
async fn budget_reservation_is_atomic_against_real_redis() {
    // The single highest-value test in this file. The enterprise readiness audit proved
    // budget::check() (the old, read-then-compare design) could be bypassed 8 runs out of
    // 8 under genuine multi-thread parallelism. It was replaced with check_and_reserve,
    // an atomic reserve-then-true-up design, and the fix was proven correct against
    // MemoryStore — but MemoryStore's "atomicity" is a single process-local mutex, and the
    // real question was always whether the same property holds against the real Redis
    // INCRBY calls check_and_reserve actually issues in production. It does: this is the
    // exact same scenario (20 concurrent requests, a $1.00 hard limit, $0.05 of headroom,
    // $0.10 per request) reproduced against a real server.
    let Some(store) = test_redis_store().await else {
        return skip_redis("budget_reservation_is_atomic_against_real_redis");
    };
    let store = Arc::new(store);
    let context = auth("pro");
    let limit = 1_000_000; // $1.00
    let cost_per_request = 100_000; // $0.10
    let limits = BudgetLimits {
        org: Some(Limit::hard(limit)),
        ..BudgetLimits::default()
    };

    // Prime spend to $0.95 directly on the real counter, the same key check_and_reserve
    // will read and write.
    let org_spend_key =
        aegis_gateway::metering::usage::org_spend_key(context.org_id, chrono::Utc::now());
    store
        .incr_by(&org_spend_key, 950_000, None)
        .await
        .expect("prime spend");

    let barrier = Arc::new(tokio::sync::Barrier::new(20));
    let mut handles = Vec::new();
    for _ in 0..20 {
        let store = Arc::clone(&store);
        let context = context.clone();
        let limits = limits.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            match budget::check_and_reserve(
                store.as_ref(),
                &context,
                &limits,
                None,
                cost_per_request,
            )
            .await
            .unwrap()
            {
                BudgetOutcome::Allowed(reservation) => {
                    let _ = reservation.commit();
                    true
                }
                BudgetOutcome::Denied(_) => false,
            }
        }));
    }

    let mut admitted = 0;
    for handle in handles {
        if handle.await.unwrap() {
            admitted += 1;
        }
    }

    let final_spend = aegis_gateway::metering::usage::current_spend(store.as_ref(), context.org_id)
        .await
        .as_i64();

    println!(
        "against real Redis: admitted {admitted} of 20, final spend {final_spend} \
         micro-cents (limit {limit})"
    );

    assert!(
        final_spend <= limit,
        "BUDGET BYPASSED AGAINST REAL REDIS: {admitted} of 20 concurrent requests were \
         admitted with only $0.05 of headroom against a $1.00 hard limit. Final spend \
         {final_spend} micro-cents. This is the exact race the enterprise readiness audit \
         proved and check_and_reserve was built to close — a failure here means the fix \
         does not hold against the real backend production actually uses."
    );

    let _ = store.del(&org_spend_key).await;
}

#[tokio::test]
async fn a_refused_reservation_leaves_the_real_counter_untouched() {
    // The rollback path, against real Redis: a refused request's rollback INCRBY(-amount)
    // must actually land, or a customer sitting at their limit would watch spend climb
    // from requests that were never served.
    let Some(store) = test_redis_store().await else {
        return skip_redis("a_refused_reservation_leaves_the_real_counter_untouched");
    };
    let context = auth("pro");
    let limits = BudgetLimits {
        org: Some(Limit::hard(1_000_000)),
        ..BudgetLimits::default()
    };
    let org_spend_key =
        aegis_gateway::metering::usage::org_spend_key(context.org_id, chrono::Utc::now());

    store
        .incr_by(&org_spend_key, 1_000_000, None)
        .await
        .expect("prime spend at the limit");

    for _ in 0..5 {
        let outcome = budget::check_and_reserve(&store, &context, &limits, None, 100_000)
            .await
            .unwrap();
        assert!(matches!(outcome, BudgetOutcome::Denied(_)));
    }

    let final_spend = aegis_gateway::metering::usage::current_spend(&store, context.org_id)
        .await
        .as_i64();
    assert_eq!(
        final_spend, 1_000_000,
        "refused requests must not accumulate spend on the real counter"
    );

    let _ = store.del(&org_spend_key).await;
}

#[tokio::test]
async fn the_scheduler_claim_admits_exactly_one_winner_against_real_redis() {
    // The distributed-jobs primitive every periodic worker (weekly digest, pricing drift,
    // session purge, budget alerts) depends on: with N replicas racing to claim the same
    // job/period, exactly one must win, or every customer gets N copies of one email.
    let Some(store) = test_redis_store().await else {
        return skip_redis("the_scheduler_claim_admits_exactly_one_winner_against_real_redis");
    };
    let store = Arc::new(store);
    let job = format!("test-job-{}", Uuid::new_v4());
    let period = "2026-08-26";

    let mut handles = Vec::new();
    for _ in 0..64 {
        let store = Arc::clone(&store);
        let job = job.clone();
        handles.push(tokio::spawn(async move {
            aegis_gateway::workers::scheduler::claim(store.as_ref(), &job, period).await
        }));
    }

    let mut winners = 0;
    for handle in handles {
        if handle.await.unwrap() {
            winners += 1;
        }
    }

    assert_eq!(
        winners, 1,
        "exactly one replica must win a scheduled-job claim against real Redis"
    );
}

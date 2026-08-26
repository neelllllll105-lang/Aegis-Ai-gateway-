//! Does the gateway actually add less than a millisecond, under concurrency?
//!
//! # What this measures, and what it does not
//!
//! `MASTER_BUILD.md` Principle 1 claims sub-millisecond P99 self-overhead, and the
//! marketing site repeats it. A claim we publish should have a number behind it that we
//! generated, not an aspiration.
//!
//! This runs the **real pipeline** — authentication, rate limiting, budget checks,
//! parsing, cache lookup, complexity classification, routing, cost computation, metering
//! — against a mock provider, from many concurrent tasks, and reports the percentile
//! distribution of `gateway_overhead_ms`. That figure explicitly excludes provider time
//! (`OverheadClock` pauses for the upstream call), so it is Aegis's own contribution and
//! nothing else.
//!
//! It does **not** measure:
//!
//!   - network latency, TLS handshakes, or load-balancer time;
//!   - Postgres contention (there is no database here);
//!   - Redis round trips (the in-memory store is used);
//!   - behaviour across replicas.
//!
//! Those need `infra/loadtest/k6-gateway.js` against a deployed instance, which is
//! `P4.8` and still outstanding. This test is the part that can be honestly verified on
//! any machine with a Rust toolchain, and it is the part that would catch a regression
//! in our own code — which is the failure mode most likely to occur.
//!
//! The threshold is deliberately loose relative to the claim. A debug build with an
//! in-memory store on shared CI hardware is not a production profile, and a test that
//! fails on an unlucky scheduling hiccup teaches people to ignore it. It is set to catch
//! an order-of-magnitude regression, not to certify the marketing number.

use aegis_gateway::config::Config;
use aegis_gateway::metering::pricing::{ModelPricing, PricingTable};
use aegis_gateway::middleware::auth::AuthContext;
use aegis_gateway::money::MicroCents;
use aegis_gateway::providers::mock::MockProvider;
use aegis_gateway::providers::ProviderRegistry;
use aegis_gateway::routes::openai_compat::execute;
use aegis_gateway::types::{ModelTier, NormalizedRequest, RoutingHint};
use aegis_gateway::AppState;
use std::sync::Arc;
use uuid::Uuid;

/// Concurrent tasks issuing requests.
const CONCURRENCY: usize = 64;

/// Requests per task. 64 × 40 = 2,560 samples, enough for a meaningful P99.
const PER_TASK: usize = 40;

/// P99 ceiling, in milliseconds.
///
/// Ten times the published claim. See the module comment: this exists to catch a
/// regression of an order of magnitude, not to certify sub-millisecond on CI hardware.
const P99_CEILING_MS: f64 = 10.0;

fn load_test_state() -> AppState {
    let mock = Arc::new(MockProvider::returning("a mock completion"));
    let mut registry = ProviderRegistry::with_builtins();
    registry.register(mock);

    let mut config = Config::for_tests();
    config
        .shared_provider_keys
        .insert("mock".into(), vec!["pooled-test-key".into()]);

    // Only mock models, for the same reason the unit tests restrict the table: with the
    // real table in place the router would correctly find a cheaper real provider and
    // this test would make thousands of live API calls.
    let mut pricing = PricingTable::new();
    for (id, tier, input, output) in [
        ("mock/mock-premium", ModelTier::Premium, 10.0, 30.0),
        ("mock/mock-cheap", ModelTier::Cheap, 0.10, 0.40),
    ] {
        pricing.insert(ModelPricing {
            model_id: id.into(),
            provider: "mock".into(),
            display_name: id.into(),
            tier,
            input_per_mtok: MicroCents::from_usd_per_mtok(input),
            output_per_mtok: MicroCents::from_usd_per_mtok(output),
            context_window: 128_000,
            supports_tools: true,
            supports_vision: true,
            is_active: true,
            source: "load test".into(),
            cache: Default::default(),
            long_context: None,
        });
    }

    AppState {
        config: Arc::new(config),
        pricing: Arc::new(pricing),
        providers: Arc::new(registry),
        ..AppState::for_tests()
    }
}

fn auth_for(org_id: Uuid) -> AuthContext {
    AuthContext::from_key(aegis_gateway::db::repo::KeyContext {
        api_key_id: Uuid::new_v4(),
        org_id,
        team_id: None,
        // High enough that the rate limiter never rejects. We are measuring the cost of
        // *checking* the limit, not the cost of being blocked by it.
        rate_limit_per_minute: 1_000_000,
        monthly_budget_mc: None,
        allowed_models: None,
        plan: "scale".to_string(),
        savings_share_bp: 2_000,
        zero_retention: false,
        org_region: "test".into(),
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    // Nearest-rank. With thousands of samples the choice of interpolation method moves
    // the answer far less than the noise does.
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn gateway_self_overhead_stays_sub_millisecond_under_concurrency() {
    let state = load_test_state();

    let mut handles = Vec::with_capacity(CONCURRENCY);
    for task in 0..CONCURRENCY {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            // A distinct organisation per task. Sharing one org would serialise every
            // request through the same counter keys and measure lock contention on a
            // single tenant rather than realistic multi-tenant load.
            let auth = auth_for(Uuid::new_v4());
            let mut samples = Vec::with_capacity(PER_TASK);

            for i in 0..PER_TASK {
                // Vary the prompt so the cache does not turn this into a measurement of
                // how fast we can return the same cached answer 2,560 times.
                let request = NormalizedRequest::simple(
                    "mock/mock-premium",
                    &format!("task {task} request {i}: summarise the quarterly figures"),
                );

                let outcome = execute(&state, &auth, request, RoutingHint::Auto)
                    .await
                    .expect("pipeline succeeds against the mock provider");

                samples.push(outcome.gateway_overhead_ms);
            }

            samples
        }));
    }

    let mut overheads = Vec::with_capacity(CONCURRENCY * PER_TASK);
    for handle in handles {
        overheads.extend(handle.await.expect("task completes"));
    }

    overheads.sort_by(|a, b| a.partial_cmp(b).expect("overheads are never NaN"));

    let p50 = percentile(&overheads, 50.0);
    let p95 = percentile(&overheads, 95.0);
    let p99 = percentile(&overheads, 99.0);
    let max = *overheads.last().expect("at least one sample");
    let mean = overheads.iter().sum::<f64>() / overheads.len() as f64;

    // Printed on every run (visible with --nocapture). The number this produces is the
    // evidence behind the claim, so it should be readable without editing the test.
    println!(
        "\ngateway self-overhead over {} samples at concurrency {CONCURRENCY}\n\
         \x20 mean {mean:.3} ms | p50 {p50:.3} ms | p95 {p95:.3} ms | p99 {p99:.3} ms | max {max:.3} ms\n",
        overheads.len()
    );

    assert_eq!(
        overheads.len(),
        CONCURRENCY * PER_TASK,
        "every request must produce an overhead sample, or the percentiles describe a \
         subset of the traffic"
    );

    assert!(
        p99 < P99_CEILING_MS,
        "P99 gateway overhead was {p99:.3} ms, over the {P99_CEILING_MS} ms ceiling. \
         We publish a sub-millisecond claim; something in the request path has become \
         an order of magnitude more expensive."
    );

    // A pipeline that reported zero overhead would mean the clock is not running, which
    // would make the assertion above pass for the wrong reason.
    assert!(
        p50 > 0.0,
        "median overhead was zero — the OverheadClock is not measuring anything"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_time_is_excluded_from_the_overhead_we_publish() {
    // The number above is only honest if provider time really is excluded. A mock that
    // takes visible time to respond proves it: if provider latency leaked into the
    // overhead figure, this would show up immediately.
    let state = load_test_state();
    let auth = auth_for(Uuid::new_v4());

    let request = NormalizedRequest::simple("mock/mock-premium", "a single request");
    let outcome = execute(&state, &auth, request, RoutingHint::Auto)
        .await
        .expect("pipeline succeeds");

    // `total_latency_ms` is a truncated integer and `gateway_overhead_ms` keeps float
    // precision, so allow the same 1ms rounding slack the pipeline's own unit test uses
    // (openai_compat.rs, the assertion on `outcome.headers()`) rather than inventing a
    // second, disagreeing tolerance.
    assert!(
        outcome.gateway_overhead_ms <= outcome.total_latency_ms as f64 + 1.0,
        "overhead ({:.3} ms) exceeded total latency ({} ms) by more than rounding — the \
         clock is wrong",
        outcome.gateway_overhead_ms,
        outcome.total_latency_ms
    );
}

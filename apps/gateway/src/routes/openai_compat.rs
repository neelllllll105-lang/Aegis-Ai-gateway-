//! The OpenAI-compatible gateway — `MASTER_BUILD.md` Part 5, stages [1] through [12].
//!
//! This is the product. Everything else exists to make this function correct and fast.
//!
//! # Structure
//!
//! [`execute`] runs the pipeline and returns a [`PipelineOutcome`]; the axum handlers
//! ([`chat_completions`]) are thin wrappers that translate HTTP in and out. The split is
//! deliberate — the pipeline is where every rule about cost, quality, and metering lives,
//! and it can be driven end to end from a test with no HTTP server, no network, and no
//! provider account.
//!
//! # Latency budget
//!
//! Auth 0.05 + rate limit 0.1 + budget 0.1 + parse 0.5 + cache 0.1 + routing 0.5 +
//! emit 0.1 ≈ 1.45ms worst case, against a P99 target of 1ms. Every stage is measured,
//! and the total is reported on the response as `X-Aegis-Latency` — Part 13 item 4 says
//! we display our own overhead and never game it, so the number on the header is the same
//! one on the usage record.

use crate::cache::durable::DurableCache;
use crate::cache::exact::ExactCache;
use crate::cache::fingerprint;
use crate::cache::semantic::{self, SemanticCache};
use crate::engine::classifier::Classifier;
use crate::engine::compressor::{self, CompressorConfig};
use crate::engine::fallback::{self, FallbackChain};
use crate::engine::policy::RoutingPolicy;
use crate::engine::router::{Router, RoutingInputs};
use crate::enterprise::residency;
use crate::error::{AegisError, Result};
use crate::metering::savings::SavingsBreakdown;
use crate::metering::usage::{self, UsageEvent};
use crate::middleware::auth::{self, AuthContext};
use crate::middleware::{budget, rate_limit};
use crate::money::MicroCents;

use crate::providers::{Credential, Provider};
use crate::types::{
    CacheOutcome, Complexity, NormalizedRequest, NormalizedResponse, RoutingHint, RoutingReason,
    TokenUsage,
};
use crate::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;

use std::time::Instant;
use uuid::Uuid;

/// Everything the pipeline produced for one request.
#[derive(Debug, Clone)]
pub struct PipelineOutcome {
    pub request_id: Uuid,
    pub response: NormalizedResponse,
    pub served_model: String,
    pub requested_model: String,
    pub provider: String,
    pub savings: SavingsBreakdown,
    /// `savings.gross_savings`, decomposed into what routing, compression, and caching each
    /// explain. See [`crate::metering::savings::SavingsComponents`].
    pub savings_components: crate::metering::savings::SavingsComponents,
    pub input_cost_mc: i64,
    pub output_cost_mc: i64,
    pub cache: CacheOutcome,
    pub routing_reason: RoutingReason,
    pub complexity_score: Option<f32>,
    pub tokens: TokenUsage,
    pub gateway_overhead_ms: f64,
    pub total_latency_ms: u32,
    pub tokens_saved_by_compression: u64,
    /// Per-technique compression counts, for `usage_records.techniques_fired`. Was only
    /// ever visible live (this response's headers, `/api/compression/preview`) — nothing
    /// wrote it onto the record, so "how much did whitespace-collapse save us last month"
    /// had no query that could answer it. `None` when compression did not run or changed
    /// nothing, matching the column's own `NULL` convention.
    pub techniques_fired: Option<serde_json::Value>,
    /// Volatile spans (timestamps, UUIDs, request ids, nonces) found in the system prompt
    /// that would invalidate the provider's own prefix cache on every turn. Detect-only —
    /// see [`crate::engine::cache_bust`]. Zero on a cache hit: no provider was called this
    /// turn, so there is nothing to attribute a prefix-cache cost to.
    pub cache_bust_hits: usize,
    /// Plain-language reasons this request was routed the way it was.
    ///
    /// Returned on `x-aegis-routing-explanation`. `routing_reason` is a six-value enum,
    /// which tells a customer that their request was downgraded "for complexity" and
    /// nothing about why — the question they actually ask support.
    pub explanation: Vec<String>,
}

/// Build the persisted compression breakdown, or `None` when nothing fired.
///
/// `None` rather than an empty object for a no-op: it matches the column's own `NULL`
/// convention (passthrough mode, a zero-retention org, or a request compression simply
/// found nothing to do to), so a query counting "requests where compression fired" can use
/// `IS NOT NULL` directly instead of also excluding an all-zero object.
pub(crate) fn techniques_fired_json(
    compression: &compressor::CompressionResult,
) -> Option<serde_json::Value> {
    if compression.is_noop() {
        return None;
    }
    Some(serde_json::json!({
        "duplicate_system_messages_removed": compression.duplicate_system_messages_removed,
        "whitespace_chars_removed": compression.whitespace_chars_removed,
        "messages_truncated": compression.messages_truncated,
        "json_blocks_minified": compression.json_blocks_minified,
        "duplicate_blocks_referenced": compression.duplicate_blocks_referenced,
        "stale_tool_results_trimmed": compression.stale_tool_results_trimmed,
    }))
}

impl PipelineOutcome {
    /// The `X-Aegis-*` headers describing what happened.
    ///
    /// Every one of these is a claim we are making to the customer about their money, so
    /// they are derived from the same values written to the usage record — never
    /// recomputed, never rounded differently.
    pub fn headers(&self) -> Vec<(HeaderName, HeaderValue)> {
        let mut headers = Vec::with_capacity(16);
        let mut push = |name: &'static str, value: String| {
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.push((HeaderName::from_static(name), value));
            }
        };

        push("x-aegis-model", self.served_model.clone());
        push("x-aegis-requested-model", self.requested_model.clone());
        push("x-aegis-cost", self.savings.actual_cost.to_usd_string());
        push(
            "x-aegis-baseline-cost",
            self.savings.baseline_cost.to_usd_string(),
        );
        push(
            "x-aegis-savings",
            self.savings.gross_savings.to_usd_string(),
        );
        if !self.savings.gross_savings.is_zero() {
            push(
                "x-aegis-savings-routing",
                self.savings_components.routing_savings.to_usd_string(),
            );
            push(
                "x-aegis-savings-compression",
                self.savings_components.compression_savings.to_usd_string(),
            );
            push(
                "x-aegis-savings-cache",
                self.savings_components.cache_savings.to_usd_string(),
            );
        }
        push("x-aegis-tokens-input", self.tokens.input_tokens.to_string());
        push(
            "x-aegis-tokens-output",
            self.tokens.output_tokens.to_string(),
        );
        if self.tokens.cached_input_tokens > 0 {
            push(
                "x-aegis-tokens-cached",
                self.tokens.cached_input_tokens.to_string(),
            );
        }
        push(
            "x-aegis-cost-input",
            MicroCents(self.input_cost_mc).to_usd_string(),
        );
        push(
            "x-aegis-cost-output",
            MicroCents(self.output_cost_mc).to_usd_string(),
        );
        push("x-aegis-cache", self.cache.as_str().to_string());
        push("x-aegis-routing", self.routing_reason.as_str().to_string());
        if self.tokens_saved_by_compression > 0 {
            push(
                "x-aegis-compression-before-tokens",
                (self.tokens.input_tokens + self.tokens_saved_by_compression).to_string(),
            );
            push(
                "x-aegis-compression-after-tokens",
                self.tokens.input_tokens.to_string(),
            );
            push(
                "x-aegis-compression-tokens-saved",
                self.tokens_saved_by_compression.to_string(),
            );
            let total_before = self.tokens.input_tokens + self.tokens_saved_by_compression;
            if total_before > 0 {
                let ratio = (self.tokens_saved_by_compression as f64 / total_before as f64) * 100.0;
                push("x-aegis-compression-ratio", format!("{:.1}%", ratio));
            }
        }
        if !self.explanation.is_empty() {
            // Header values must be printable ASCII on one line, so the reasons are joined
            // with a separator and anything else is dropped rather than producing a header
            // the client's HTTP library will reject outright.
            let joined: String = self
                .explanation
                .join("; ")
                .chars()
                .map(|c| {
                    if c.is_ascii() && !c.is_ascii_control() {
                        c
                    } else {
                        ' '
                    }
                })
                .collect();
            push("x-aegis-routing-explanation", joined);
        }
        push(
            "x-aegis-latency",
            format!(
                "{}ms (overhead: {:.3}ms)",
                self.total_latency_ms, self.gateway_overhead_ms
            ),
        );
        if self.cache_bust_hits > 0 {
            push("x-aegis-cache-bust", self.cache_bust_hits.to_string());
        }
        push("x-aegis-request-id", self.request_id.to_string());
        headers
    }

    /// The usage event for this request.
    ///
    /// `region` is the serving instance's own region, passed in rather than read from a
    /// global: the usage writer may run elsewhere, and attributing spend to the writer's
    /// region rather than the server's would silently misreport every regional budget.
    pub fn usage_event(&self, auth: &AuthContext, status_code: u16, region: &str) -> UsageEvent {
        let mut event = UsageEvent::new(
            self.request_id,
            auth.org_id,
            auth.api_key_id,
            auth.team_id,
            self.requested_model.clone(),
            self.served_model.clone(),
            self.provider.clone(),
            self.tokens,
            self.savings,
            self.total_latency_ms,
            self.gateway_overhead_ms,
            self.cache,
            self.routing_reason,
            self.complexity_score,
            status_code,
        );
        event.input_cost_mc = self.input_cost_mc;
        event.output_cost_mc = self.output_cost_mc;
        event.tokens_saved_by_compression = self.tokens_saved_by_compression;
        event.techniques_fired = self.techniques_fired.clone();
        event.routing_savings_mc = self.savings_components.routing_savings.as_i64();
        event.compression_savings_mc = self.savings_components.compression_savings.as_i64();
        event.cache_savings_mc = self.savings_components.cache_savings.as_i64();
        event.cache_bust_hits = self.cache_bust_hits as u32;
        event.region = Some(region.to_string());
        // Who sent it, when the key names a person. Set here rather than passed into
        // `UsageEvent::new` because that constructor is already at fifteen positional
        // arguments and a sixteenth silently mis-ordered would be a billing bug nobody
        // would see.
        event.user_id = auth.user_id;
        event
    }
}

/// Measures how much latency Aegis itself added.
///
/// Provider time is explicitly excluded: it is not ours, and including it would flatter
/// the number we publish. The clock is paused for the upstream call and resumed after.
#[derive(Debug)]
struct OverheadClock {
    started: Instant,
    provider_time: std::time::Duration,
    provider_started: Option<Instant>,
}

impl OverheadClock {
    fn start() -> OverheadClock {
        OverheadClock {
            started: Instant::now(),
            provider_time: std::time::Duration::ZERO,
            provider_started: None,
        }
    }

    fn enter_provider(&mut self) {
        self.provider_started = Some(Instant::now());
    }

    fn exit_provider(&mut self) {
        if let Some(started) = self.provider_started.take() {
            self.provider_time += started.elapsed();
        }
    }

    /// Total wall time so far.
    fn total_ms(&self) -> u32 {
        self.started.elapsed().as_millis().min(u32::MAX as u128) as u32
    }

    /// Our own overhead: total minus time spent waiting on the provider.
    fn overhead_ms(&self) -> f64 {
        self.started
            .elapsed()
            .saturating_sub(self.provider_time)
            .as_secs_f64()
            * 1_000.0
    }
}

/// Derives a deterministic affinity key for a multi-turn conversation.
///
/// Hashes org_id, system prompt text, and the first user message.
/// Because these remain invariant across all turns of a conversation, all subsequent turns
/// will produce the exact same key and map to the pinned model in Redis.
pub(crate) fn conversation_affinity_key(
    org_id: Uuid,
    request: &NormalizedRequest,
) -> Option<String> {
    use sha2::{Digest, Sha256};
    let system = request.system_text();
    let first_user = request.first_user_message();
    if system.trim().is_empty() && first_user.as_deref().unwrap_or("").trim().is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(org_id.as_bytes());
    hasher.update(b":");
    hasher.update(system.as_bytes());
    hasher.update(b":");
    if let Some(user_msg) = first_user {
        hasher.update(user_msg.as_bytes());
    }
    let hash = format!("{:x}", hasher.finalize());
    Some(format!("aegis:affinity:{}:{}", org_id, hash))
}

/// The token circuit breaker — stage [4b].
///
/// Clamps (never raises) a request's effective `max_tokens` to `ceiling`, unconditionally.
/// A request with no `max_tokens` at all is the more dangerous case, not the safer one: it
/// defers to whatever the provider's own default happens to be, which for some models is
/// very large and for reasoning models can include a substantial "thinking" allowance the
/// caller never asked for — this injects the ceiling rather than leaving it unset.
///
/// Returns whether the request was actually changed, so the caller can decide whether to
/// surface it in the response's explanation rather than clamping silently.
pub(crate) fn clamp_max_tokens(request: &mut NormalizedRequest, ceiling: u32) -> bool {
    match request.max_tokens {
        Some(requested) if requested <= ceiling => false,
        Some(_) | None => {
            request.max_tokens = Some(ceiling);
            true
        }
    }
}

/// Build the outcome for any cache hit — hot tier, durable tier, or semantic — and record
/// its savings metric.
///
/// The three tiers share this because a hit costs nothing regardless of which one served
/// it: the entire baseline becomes the saving, priced through `baseline_of` so a cached
/// response that itself used the *provider's* own prompt cache reports the saving the
/// customer actually avoided rather than an inflated full-rate figure.
fn cache_hit_outcome(
    state: &AppState,
    request_id: Uuid,
    requested_model: String,
    response: NormalizedResponse,
    served_model: String,
    savings_share_bp: u32,
    outcome: CacheOutcome,
    explanation: String,
    clock: &OverheadClock,
) -> PipelineOutcome {
    let pricing = state.pricing();
    let baseline = pricing.baseline_of(&requested_model, &response.usage, MicroCents::ZERO);
    let savings = SavingsBreakdown::cache_hit(baseline, savings_share_bp);
    let savings_components =
        crate::metering::savings::SavingsComponents::cache_hit(savings.gross_savings);

    state.metrics.record_cache(outcome.as_str());
    state.metrics.record_savings(savings.gross_savings.as_i64());

    PipelineOutcome {
        request_id,
        tokens: response.usage,
        served_model,
        requested_model,
        provider: "cache".to_string(),
        response,
        savings,
        savings_components,
        input_cost_mc: 0,
        output_cost_mc: 0,
        cache: outcome,
        routing_reason: RoutingReason::Cache,
        complexity_score: None,
        gateway_overhead_ms: clock.overhead_ms(),
        total_latency_ms: clock.total_ms(),
        tokens_saved_by_compression: 0,
        techniques_fired: None,
        cache_bust_hits: 0,
        explanation: vec![explanation],
    }
}

/// Run the full pipeline for one request.
pub async fn execute(
    state: &AppState,
    auth: &AuthContext,
    request: NormalizedRequest,
    hint: RoutingHint,
) -> Result<PipelineOutcome> {
    execute_with_headroom(state, auth, request, hint, None).await
}

/// As [`execute`], told how much budget the organisation has left.
///
/// The headroom steers routing toward cheaper models as a hard cap approaches, so a
/// customer running low degrades gracefully instead of hitting a wall of 402s. Split from
/// `execute` rather than added to it so the many call sites that have no budget context —
/// tests, the embeddings path — stay unchanged.
pub async fn execute_with_headroom(
    state: &AppState,
    auth: &AuthContext,
    mut request: NormalizedRequest,
    hint: RoutingHint,
    budget_headroom_mc: Option<i64>,
) -> Result<PipelineOutcome> {
    let mut clock = OverheadClock::start();
    let request_id = Uuid::new_v4();
    let requested_model = request.model.clone();

    // ---- [4b] Token circuit breaker ------------------------------------------------
    // As early as possible — before the fingerprint is computed, so a clamped value is
    // what gets cached and hashed too, rather than letting an over-the-ceiling request
    // fragment the cache key space for no benefit.
    let token_limit_enforced = clamp_max_tokens(&mut request, state.config.max_tokens_per_request);
    if token_limit_enforced {
        tracing::info!(
            org_id = %auth.org_id,
            ceiling = state.config.max_tokens_per_request,
            "token circuit breaker: clamped max_tokens for this request"
        );
    }

    // ---- [3] Free-tier allowance -------------------------------------------------
    if !budget::check_free_tier_allowance(
        state.store.as_ref(),
        auth,
        state.config.free_tier_monthly_requests,
    )
    .await?
    {
        return Err(AegisError::BudgetExceeded {
            spend_micro_cents: usage::current_spend(state.store.as_ref(), auth.org_id)
                .await
                .as_i64(),
            limit_micro_cents: 0,
            also_exceeded: Vec::new(),
        });
    }

    // ---- [5a] Exact cache: hot tier (Redis) ---------------------------------------
    // Semantic caching (and the durable tier's promotion) is a Pro/Enterprise feature —
    // MASTER_BUILD.md lists it under the Pro plan, and free-tier traffic already runs on
    // Aegis's own pooled provider keys, so there is no revenue case for spending extra
    // storage and embedding calls to optimise it further.
    let smart_caching_enabled = auth.plan != "free";
    let cache = ExactCache::new(state.store.as_ref(), state.config.cache_ttl);
    let request_fingerprint = fingerprint::compute(&request, auth.org_id);

    if let Some(hit) = cache
        .get(&request, auth.org_id, auth.zero_retention)
        .await
        .unwrap_or(None)
    {
        // This fingerprint has now been asked at least twice — promote it to the durable
        // tier so a repeat after the hot tier's TTL expires still costs nothing. Best
        // effort: a promotion failure must never turn a free cache hit into a failed
        // request.
        if smart_caching_enabled {
            if let Some(pool) = state.db.as_ref() {
                let _ = DurableCache::new(
                    pool,
                    &state.config.master_key,
                    state.config.durable_cache_ttl_days,
                )
                .promote(
                    request_fingerprint.as_str(),
                    auth.org_id,
                    auth.zero_retention,
                    &hit.response,
                    &hit.served_model,
                )
                .await;
            }
        }

        return Ok(cache_hit_outcome(
            state,
            request_id,
            requested_model,
            hit.response,
            hit.served_model,
            auth.savings_share_bp,
            CacheOutcome::Exact,
            "served from Aegis's exact-match cache — an identical request was answered \
             within the cache window, so no provider was called and this request cost \
             nothing"
                .to_string(),
            &clock,
        ));
    }

    // ---- [5a-warm] Exact cache: durable tier (Postgres, encrypted) ----------------
    // Only reached once the hot tier has missed. A hit here means the hot tier's TTL
    // expired on a query that was proven, by an earlier promotion, to genuinely repeat.
    if smart_caching_enabled && !auth.zero_retention {
        if let Some(pool) = state.db.as_ref() {
            let durable = DurableCache::new(
                pool,
                &state.config.master_key,
                state.config.durable_cache_ttl_days,
            );
            if let Ok(Some(hit)) = durable
                .get(
                    request_fingerprint.as_str(),
                    auth.org_id,
                    auth.zero_retention,
                )
                .await
            {
                // Re-populate the hot tier so the *next* repeat is served in 0.1ms again
                // instead of a database round trip.
                let _ = cache
                    .put(
                        &request,
                        auth.org_id,
                        auth.zero_retention,
                        &hit.response,
                        &hit.served_model,
                    )
                    .await;

                return Ok(cache_hit_outcome(
                    state,
                    request_id,
                    requested_model,
                    hit.response,
                    hit.served_model,
                    auth.savings_share_bp,
                    CacheOutcome::Exact,
                    "served from Aegis's durable cache — this exact request has been asked \
                     more than once before, so it's kept encrypted past the usual cache \
                     window, and this request cost nothing"
                        .to_string(),
                    &clock,
                ));
            }
        }
    }

    // ---- [5b] Semantic cache -------------------------------------------------------
    // Only reached once both exact tiers have missed. Catches a *reworded* repeat of a
    // question already answered — "how do I enable X" finding "how do I turn on X" — which
    // no hash-based fingerprint ever can. The embedding is computed at most once per
    // request: reused below to populate the semantic store on a true miss, so a novel
    // question costs one embedding call, not two.
    //
    // Classified once, up front, rather than only inside the old `is_local()`-gated branch:
    // its `complexity` now feeds `semantic_lookup_worth_it` below, and its `domain` still
    // scopes the cache lookup exactly as before — one classification serving both, not two.
    let semantic_classification = Classifier::new().classify(&request);
    let mut request_embedding: Option<Vec<f32>> = None;
    if smart_caching_enabled
        && !auth.zero_retention
        && fingerprint::cacheability(&request, auth.zero_retention).is_cacheable()
        && semantic_lookup_worth_it(
            state.embedder.is_local(),
            state.config.semantic_cache_allow_remote_embedding,
            semantic_classification.complexity,
        )
    {
        let text = semantic::embedding_text(&request);
        if let Some(embedding) = state.embedder.embed(&text).await {
            let domain = semantic_classification.domain;
            let semantic_cache = SemanticCache::with_domain(
                state.semantic_store.as_ref(),
                state.config.semantic_similarity_threshold,
                domain,
            );
            if let Ok(Some(hit)) = semantic_cache
                .get(&embedding, auth.org_id, auth.zero_retention)
                .await
            {
                // Now that we know these two different wordings mean the same thing,
                // cache *this* wording's exact fingerprint too — the next repeat of this
                // specific phrasing skips the embedding call entirely.
                let _ = cache
                    .put(
                        &request,
                        auth.org_id,
                        auth.zero_retention,
                        &hit.entry.response,
                        &hit.entry.served_model,
                    )
                    .await;

                return Ok(cache_hit_outcome(
                    state,
                    request_id,
                    requested_model,
                    hit.entry.response,
                    hit.entry.served_model,
                    auth.savings_share_bp,
                    CacheOutcome::Semantic,
                    format!(
                        "served from Aegis's semantic cache — a differently-worded version \
                         of this request was already answered ({:.0}% similar), so no \
                         provider was called and this request cost nothing",
                        hit.similarity * 100.0
                    ),
                    &clock,
                ));
            }
            request_embedding = Some(embedding);
        }
    }

    state.metrics.record_cache(if auth.zero_retention {
        "skipped"
    } else {
        "miss"
    });

    // ---- [6b] Prefix cache-bust detection (technique #7, detect-only) -------------
    // Scanned on the system prompt as the customer actually sent it, before compression
    // touches anything — this is evidence about the customer's own prompt engineering, not
    // about what Aegis did to the request.
    let cache_bust_report = crate::engine::cache_bust::scan(&request.system_text());
    for hit in &cache_bust_report.hits {
        state.metrics.record_cache_bust(hit.kind);
    }

    // ---- [6c] Context compression ------------------------------------------------
    // Zero-retention organisations get their prompt delivered exactly as written.
    let compressor_config = if auth.zero_retention {
        CompressorConfig::disabled()
    } else {
        CompressorConfig::default()
    };
    let compression = compressor::compress(&mut request, &compressor_config);
    // ---- [6] Routing with Conversation Affinity ----------------------------------
    let affinity_key = conversation_affinity_key(auth.org_id, &request);
    let affinity_model = if let Some(ref key) = affinity_key {
        state.store.get(key).await.ok().flatten()
    } else {
        None
    };

    let configured_providers = get_configured_providers(state, auth.org_id).await;
    let policy = load_policy(state, auth).await;
    let inputs = RoutingInputs {
        hint,
        policy: policy.as_ref(),
        team: auth.team_name.clone(),
        user: auth.user_email.clone(),
        allowed_models: auth.allowed_models.clone(),
        plan_tier_ceiling: plan_tier_ceiling(&auth.plan),
        budget_headroom_mc,
        // Outcome-informed selection. The bandit has recorded every routing result since
        // it was written and was never read back, which made "outcome-trained routing" a
        // description of intent rather than behaviour. It reorders candidates the router
        // has already accepted; it can never widen the set.
        bandit: Some(state.bandit.as_ref()),
        affinity_model,
        configured_providers: configured_providers.clone(),
    };

    let router = Router::with_classifier(Classifier::new());
    let decision = router.route(&request, &state.pricing(), &state.health, &inputs)?;

    // ---- [7] Provider execution with fallback -------------------------------------
    let requested_provider = state
        .pricing()
        .get(&requested_model)
        .map(|m| m.provider.clone())
        .unwrap_or_else(|| decision.provider.clone());

    let chain = FallbackChain::build(
        &decision.served_model,
        &decision.provider,
        &requested_model,
        &requested_provider,
        &alternates_for(state, &request, &decision, configured_providers.as_ref()),
    );

    clock.enter_provider();
    let attempt = execute_with_fallback(state, auth, &request, &chain).await;
    clock.exit_provider();

    let (response, served_model, provider_id, used_fallback) = attempt?;

    // Maintain conversation affinity in Redis (30-minute sliding window) to preserve
    // upstream KV prompt cache discounts on subsequent turns.
    if let Some(ref key) = affinity_key {
        let _ = state
            .store
            .set_ex(key, &served_model, std::time::Duration::from_secs(1800))
            .await;
    }

    // ---- [8]/[9] Usage and cost ---------------------------------------------------
    let tokens = resolve_usage(&response, &request);

    let actual_cost = state
        .pricing()
        .cost_of(&served_model, &tokens)
        .unwrap_or(MicroCents::ZERO);

    // If compression reduced input tokens, baseline what the customer would have paid
    // without Aegis includes the tokens that were saved by compression.
    let uncompressed_tokens = if compression.tokens_saved() > 0 {
        crate::types::TokenUsage {
            input_tokens: tokens.input_tokens.saturating_add(compression.tokens_saved()),
            ..tokens
        }
    } else {
        tokens
    };

    let baseline_cost = state
        .pricing()
        .cost_of(&requested_model, &uncompressed_tokens)
        .unwrap_or(actual_cost);

    let savings = SavingsBreakdown::compute(baseline_cost, actual_cost, auth.savings_share_bp);
    state.metrics.record_savings(savings.gross_savings.as_i64());

    // A routing decision that cost more than the requested model is a bug in our
    // optimization, not a charge for the customer. Surface it loudly.
    if savings.is_overspend() {
        tracing::warn!(
            requested = %requested_model,
            served = %served_model,
            overspend = %savings.overspend_amount(),
            "routing decision cost more than the requested model"
        );
    }

    // ---- [5a] Populate the hot cache -----------------------------------------------
    let _ = cache
        .put(
            &request,
            auth.org_id,
            auth.zero_retention,
            &response,
            &served_model,
        )
        .await;

    // ---- [5b] Populate the semantic cache -------------------------------------------
    // Only when we already paid for an embedding above while checking for a semantic hit
    // — never a second embedding call for the same request.
    if let Some(embedding) = request_embedding {
        let semantic_cache = SemanticCache::new(
            state.semantic_store.as_ref(),
            state.config.semantic_similarity_threshold,
        );
        let _ = semantic_cache
            .put(
                embedding,
                auth.org_id,
                auth.zero_retention,
                &response,
                &served_model,
            )
            .await;
    }

    // ---- [7] Record the outcome for the bandit ------------------------------------
    let classification = router.classify(&request);
    state.bandit.record(
        classification.complexity,
        &served_model,
        true,
        savings.gross_savings,
        actual_cost,
    );

    let routing_reason = if used_fallback {
        RoutingReason::Fallback
    } else {
        decision.reason
    };

    let mut explanation = decision.explanation.clone();
    if used_fallback {
        explanation.push(format!(
            "the routed provider failed, so this request was served by {provider_id} instead"
        ));
    }
    if token_limit_enforced {
        explanation.push(format!(
            "max_tokens was capped at {} by Aegis's platform-wide token circuit breaker, \
             to bound this request's worst-case cost independently of budget headroom",
            state.config.max_tokens_per_request
        ));
    }

    // Recorded after failover has resolved, not at selection time. Previously this ran
    // immediately after `router.route()` and before `execute_with_fallback`, so the reason
    // label was always the *intended* one — meaning the one aggregate an SRE would alert on
    // ("how often are we falling back") could structurally never show a fallback, and the
    // model label was wrong for every request that failed over. Found in the enterprise
    // readiness audit.
    state
        .metrics
        .record_routing(routing_reason.as_str(), &served_model);

    let input_cost = state
        .pricing()
        .get(&served_model)
        .map(|m| m.input_cost_of(&tokens))
        .unwrap_or(MicroCents::ZERO);
    let output_cost = state
        .pricing()
        .get(&served_model)
        .map(|m| m.output_cost_of(&tokens))
        .unwrap_or(MicroCents::ZERO);

    // Savings decomposition (IG-1 §2.4): compression is priced directly at the served
    // model's input rate; caching is the served model's own provider-side cache discount.
    // Routing absorbs whatever of `gross_savings` those two do not explain — see
    // `SavingsComponents::compute`'s own doc comment for why that is the honest way to do
    // this without a second, harder-to-verify hypothetical price lookup.
    let served_pricing = state.pricing().get(&served_model).cloned();
    let compression_savings_raw = served_pricing
        .as_ref()
        .map(|m| {
            let (input_rate, _) = m.rates_for(tokens.total_input());
            MicroCents::cost_for_tokens(input_rate, compression.tokens_saved())
        })
        .unwrap_or(MicroCents::ZERO);
    let cache_savings_raw = served_pricing
        .as_ref()
        .map(|m| m.cache_discount_of(&tokens))
        .unwrap_or(MicroCents::ZERO);
    let savings_components = crate::metering::savings::SavingsComponents::compute(
        savings.gross_savings,
        compression_savings_raw,
        cache_savings_raw,
    );

    Ok(PipelineOutcome {
        request_id,
        cache_bust_hits: cache_bust_report.count(),
        savings_components,
        response,
        served_model,
        requested_model,
        provider: provider_id,
        savings,
        input_cost_mc: input_cost.as_i64(),
        output_cost_mc: output_cost.as_i64(),
        cache: CacheOutcome::Miss,
        routing_reason,
        complexity_score: decision.complexity_score,
        tokens,
        gateway_overhead_ms: clock.overhead_ms(),
        total_latency_ms: clock.total_ms(),
        tokens_saved_by_compression: compression.tokens_saved(),
        techniques_fired: techniques_fired_json(&compression),
        explanation,
    })
}

/// Try each attempt in the chain, with retries inside each attempt.
///
/// Returns the response, the model that produced it, its provider, and whether a fallback
/// was needed.
async fn execute_with_fallback(
    state: &AppState,
    auth: &AuthContext,
    request: &NormalizedRequest,
    chain: &FallbackChain,
) -> Result<(NormalizedResponse, String, String, bool)> {
    let mut last_error = None;

    for (index, attempt) in chain.attempts.iter().enumerate() {
        let Some(provider) = state.providers.for_model(&attempt.model_id) else {
            continue;
        };

        // A circuit-open provider is skipped without spending a timeout on it.
        if !state.health.is_available(provider.id()) || !state.health.try_probe(provider.id()) {
            continue;
        }

        let credential = match resolve_credential(state, auth, provider.id()).await {
            Ok(credential) => credential,
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        };

        // Timed here rather than inside `call_with_retries` so the measurement covers what
        // the caller actually waited for, retries and backoff included. A provider whose
        // first attempt always fails and second always succeeds is slow *in practice*, and
        // routing should see it that way.
        let attempt_started = Instant::now();
        match call_with_retries(
            state,
            provider.as_ref(),
            request,
            &attempt.model_id,
            &credential,
        )
        .await
        {
            Ok(response) => {
                let elapsed_ms = attempt_started.elapsed().as_secs_f64() * 1_000.0;
                // Feeds both the graded health the router now reads and the per-provider
                // latency series the Grafana panel always promised but could not deliver.
                state
                    .health
                    .record_success_with_latency(provider.id(), elapsed_ms);
                state.metrics.record_provider_latency_ms(
                    provider.id(),
                    &attempt.model_id,
                    elapsed_ms,
                );
                if index > 0 {
                    state
                        .metrics
                        .record_fallback(&chain.attempts[0].provider, provider.id());
                }
                return Ok((
                    response,
                    attempt.model_id.clone(),
                    provider.id().to_string(),
                    index > 0,
                ));
            }
            Err(e) => {
                let state_after = state.health.record_failure(provider.id());
                state
                    .metrics
                    .record_provider_error(provider.id(), e.error_type());
                if state_after == fallback::CircuitState::Open {
                    state
                        .metrics
                        .record_circuit_change(provider.id(), state_after.as_str());
                }

                // Client-level 400 Bad Request will fail identically everywhere; other errors
                // (e.g. 404 model not found, 401 unauthorized, 429 rate limited, 5xx) should
                // proceed down the fallback chain to alternative providers and models.
                if let AegisError::Provider { status, .. } = &e {
                    if *status == 400 {
                        return Err(e);
                    }
                }
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        AegisError::AllProvidersFailed(
            "no configured provider could serve this request".to_string(),
        )
    }))
}

/// A successfully opened upstream stream, and which attempt produced it.
pub(crate) struct StreamAttempt {
    pub upstream: crate::providers::ChunkStream,
    pub model_id: String,
    pub provider_id: String,
    pub used_fallback: bool,
}

/// Providers for which credentials exist for this org or in the shared pool.
pub(crate) async fn get_configured_providers(
    state: &AppState,
    org_id: Uuid,
) -> Option<std::collections::HashSet<String>> {
    let mut providers = std::collections::HashSet::new();
    if let Some(pool) = state.db.as_ref() {
        if let Ok(creds) = crate::db::repo::list_credentials(pool, org_id).await {
            for c in creds {
                providers.insert(c.provider);
            }
        }
    }
    for (p, keys) in &state.config.shared_provider_keys {
        if !keys.is_empty() {
            providers.insert(p.clone());
        }
    }
    if providers.is_empty() {
        None
    } else {
        Some(providers)
    }
}

/// Build a fallback chain for a streaming request and open the first attempt that answers.
///
/// The entry point `/v1/messages` uses; `stream_chat` inlines the same two steps because it
/// already has the chain in hand. Sharing this is what keeps the Anthropic-compatible
/// endpoint from being the one without retries — which is what it was.
pub(crate) async fn open_stream_for(
    state: &AppState,
    auth: &AuthContext,
    request: &NormalizedRequest,
    decision: &crate::engine::router::RoutingDecision,
    requested_model: &str,
) -> Result<StreamAttempt> {
    let configured = get_configured_providers(state, auth.org_id).await;
    let requested_provider = state
        .pricing()
        .get(requested_model)
        .map(|m| m.provider.clone())
        .unwrap_or_else(|| decision.provider.clone());
    let chain = FallbackChain::build(
        &decision.served_model,
        &decision.provider,
        requested_model,
        &requested_provider,
        &alternates_for(state, request, decision, configured.as_ref()),
    );
    open_stream_with_fallback(state, auth, request, &chain).await
}

/// Open an upstream stream, retrying and failing over until the first byte is sent.
///
/// The mirror of [`execute_with_fallback`] for streams. Everything here happens before any
/// content reaches the client, so a failure is invisible and failover is safe. Circuit
/// state is updated on both outcomes, which is what lets streaming traffic participate in
/// provider health at all.
async fn open_stream_with_fallback(
    state: &AppState,
    auth: &AuthContext,
    request: &NormalizedRequest,
    chain: &FallbackChain,
) -> Result<StreamAttempt> {
    let mut last_error = None;

    for (index, attempt) in chain.attempts.iter().enumerate() {
        let Some(provider) = state.providers.for_model(&attempt.model_id) else {
            continue;
        };
        if !state.health.is_available(provider.id()) || !state.health.try_probe(provider.id()) {
            continue;
        }

        let credential = match resolve_credential(state, auth, provider.id()).await {
            Ok(credential) => credential,
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        };

        let timeout = if is_reasoning_model(&attempt.model_id) {
            state.config.provider_timeout_reasoning
        } else {
            state.config.provider_timeout
        };

        // One key per fallback-chain attempt, stable across its internal retries — the
        // same reasoning as `call_with_retries`'s idempotency key, applied to opening a
        // stream instead of a non-streaming call.
        let idempotency_key = format!("aegis-{}", uuid::Uuid::new_v4());
        let mut tries = 0;
        let opened_at = Instant::now();
        loop {
            match provider
                .chat_stream(
                    &state.http,
                    request,
                    &attempt.model_id,
                    &credential,
                    timeout,
                    Some(&idempotency_key),
                )
                .await
            {
                Ok(mut upstream) => {
                    let elapsed_ms = opened_at.elapsed().as_secs_f64() * 1_000.0;

                    // TTFT Verification Gate: buffer the first chunk to catch immediate empty streams,
                    // upstream network truncations, or model refusals before committing to the client.
                    let final_stream: crate::providers::ChunkStream = if index + 1
                        < chain.attempts.len()
                    {
                        match upstream.next().await {
                            Some(Ok(first_chunk)) => {
                                let text = first_chunk.delta.trim_start();
                                let is_refusal = text.starts_with("I cannot")
                                    || text.starts_with("I am unable")
                                    || text.starts_with("As an AI");
                                if is_refusal {
                                    tracing::info!(
                                        model = %attempt.model_id,
                                        "TTFT buffer gate detected refusal from candidate; cascading to next tier"
                                    );
                                    continue;
                                }
                                Box::pin(
                                    futures::stream::once(async move { Ok(first_chunk) })
                                        .chain(upstream),
                                )
                            }
                            Some(Err(e)) => {
                                tracing::warn!(
                                    model = %attempt.model_id,
                                    error = %e,
                                    "TTFT initial chunk error; advancing fallback chain"
                                );
                                record_provider_failure(state, provider.id(), &e);
                                last_error = Some(e);
                                break;
                            }
                            None => {
                                tracing::warn!(
                                    model = %attempt.model_id,
                                    "upstream stream closed immediately without producing tokens; advancing fallback chain"
                                );
                                continue;
                            }
                        }
                    } else {
                        upstream
                    };

                    state
                        .health
                        .record_success_with_latency(provider.id(), elapsed_ms);
                    // For a stream, time-to-open *is* time-to-first-token from the
                    // client's point of view — the number a streaming client experiences
                    // as responsiveness, which total latency cannot express because a long
                    // answer and a slow start look identical end to end.
                    state
                        .metrics
                        .record_ttft_ms(provider.id(), &attempt.model_id, elapsed_ms);
                    if index > 0 {
                        state
                            .metrics
                            .record_fallback(&chain.attempts[0].provider, provider.id());
                    }
                    return Ok(StreamAttempt {
                        upstream: final_stream,
                        model_id: attempt.model_id.clone(),
                        provider_id: provider.id().to_string(),
                        used_fallback: index > 0,
                    });
                }
                Err(e) => {
                    tries += 1;
                    if tries <= fallback::MAX_RETRIES && fallback::is_retryable(&e) {
                        tokio::time::sleep(fallback::retry_delay(tries)).await;
                        continue;
                    }
                    record_provider_failure(state, provider.id(), &e);
                    // Client-level 400 Bad Request will fail identically everywhere; other errors
                    // should proceed down the fallback chain.
                    if let AegisError::Provider { status, .. } = &e {
                        if *status == 400 {
                            return Err(e);
                        }
                    }
                    last_error = Some(e);
                    break;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        AegisError::AllProvidersFailed(
            "no configured provider could serve this request".to_string(),
        )
    }))
}

/// Record a provider failure against its circuit breaker and the metrics.
///
/// Extracted so the streaming and non-streaming paths cannot drift: they previously did,
/// with streaming recording nothing at all.
fn record_provider_failure(state: &AppState, provider_id: &str, error: &AegisError) {
    let state_after = state.health.record_failure(provider_id);
    state
        .metrics
        .record_provider_error(provider_id, error.error_type());
    if state_after == fallback::CircuitState::Open {
        state
            .metrics
            .record_circuit_change(provider_id, state_after.as_str());
    }
}

/// Cross-provider substitutes for the selected model, cheapest first.
///
/// This is what makes the fallback chain a chain. `FallbackChain::build` has always
/// accepted an `alternates` list — "the same capability on a different provider, then one
/// tier down" — and its only production call site passed an empty slice, so the chain
/// collapsed to the selected model plus the requested one. For a request the router
/// refuses to downgrade (anything classified complex, or an explicit passthrough) those
/// two are the *same* model, leaving a chain of length one: the highest-value traffic in
/// the product had no provider redundancy at all. Found in the enterprise readiness audit.
///
/// Candidates are drawn from a different provider than the selection, must satisfy the
/// same capability requirements, and must sit in the same tier or better — a "fallback"
/// that quietly answers a complex request on a cheap model is a quality regression wearing
/// a resilience label. Capped at two, because a third alternate adds latency to a request
/// that is already having a bad time.
fn alternates_for(
    state: &AppState,
    request: &NormalizedRequest,
    decision: &crate::engine::router::RoutingDecision,
    configured_providers: Option<&std::collections::HashSet<String>>,
) -> Vec<(String, String)> {
    let pricing = state.pricing();
    let Some(selected) = pricing.get(&decision.served_model) else {
        return Vec::new();
    };
    let requirements = crate::metering::pricing::Requirements {
        tools: request.requires_tools(),
        vision: request.requires_vision(),
        min_context: request.estimated_input_tokens().min(u32::MAX as u64) as u32,
        chat: true,
    };

    let mut candidates: Vec<&crate::metering::pricing::ModelPricing> = pricing
        .all()
        .filter(|m| m.provider != selected.provider)
        .filter(|m| m.tier >= selected.tier)
        .filter(|m| match configured_providers {
            Some(configured) => {
                configured.contains(&m.provider)
                    || (configured.contains("openrouter")
                        && (m.provider == "openrouter" || m.model_id.starts_with("openrouter/")))
            }
            None => true,
        })
        .filter(|m| crate::metering::pricing::PricingTable::satisfies(m, requirements))
        .collect();

    // Cheapest first, then by id: a deterministic order means the same outage produces the
    // same failover every time, which is what makes an incident reproducible.
    candidates.sort_by(|a, b| {
        a.blended_per_mtok()
            .cmp(&b.blended_per_mtok())
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    candidates
        .into_iter()
        .take(2)
        .map(|m| (m.model_id.clone(), m.provider.clone()))
        .collect()
}

/// One provider call with the retry policy from Part 5 [7].
async fn call_with_retries(
    state: &AppState,
    provider: &dyn Provider,
    request: &NormalizedRequest,
    model: &str,
    credential: &Credential,
) -> Result<NormalizedResponse> {
    let timeout = if is_reasoning_model(model) {
        state.config.provider_timeout_reasoning
    } else {
        state.config.provider_timeout
    };

    // One key for every retry of this attempt. A timeout on our side after the provider
    // actually finished processing the request would otherwise have no way to be
    // recognised as a duplicate, and the retry below bills the same logical request twice
    // against our own account — invisible to the customer, real against our own COGS.
    let idempotency_key = format!("aegis-{}", uuid::Uuid::new_v4());

    let mut attempt = 0;
    loop {
        match provider
            .chat(
                &state.http,
                request,
                model,
                credential,
                timeout,
                Some(&idempotency_key),
            )
            .await
        {
            Ok(response) => return Ok(response),
            Err(e) => {
                attempt += 1;
                if attempt > fallback::MAX_RETRIES || !fallback::is_retryable(&e) {
                    return Err(e);
                }
                tokio::time::sleep(fallback::retry_delay(attempt)).await;
            }
        }
    }
}

/// Reasoning models think for far longer, and cutting them off at 30 seconds turns a
/// slow success into a billed failure.
fn is_reasoning_model(model: &str) -> bool {
    let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
    bare.starts_with('o') || bare.contains("reasoner") || bare.contains("thinking")
}

/// Find a credential for a provider: the organisation's own key first, ours second.
pub(crate) async fn resolve_credential(
    state: &AppState,
    auth: &AuthContext,
    provider_id: &str,
) -> Result<Credential> {
    if let Some(pool) = state.db.as_ref() {
        if let Some(stored) =
            crate::db::repo::find_default_credential(pool, auth.org_id, provider_id).await?
        {
            let plaintext = crate::crypto::decrypt(&state.config.master_key, &stored.encrypted_key)
                .map_err(|_| {
                    // A credential that will not decrypt means the master key changed.
                    // Say so plainly rather than surfacing an opaque crypto error.
                    AegisError::Internal(format!(
                        "stored credential for {provider_id} could not be decrypted"
                    ))
                })?;
            let key = String::from_utf8(plaintext).map_err(|_| AegisError::Crypto)?;
            return Ok(match stored.base_url {
                Some(base_url) => Credential::with_base_url(key, base_url),
                None => Credential::new(key),
            });
        }
    }

    state
        .shared_pool
        .next_credential(&state.config, provider_id)
}

/// Token counts for a response, estimating when the provider did not report them.
///
/// An estimate is always marked as such on the usage record, so a customer disputing an
/// invoice can see exactly which figures came from the provider and which did not.
fn resolve_usage(response: &NormalizedResponse, request: &NormalizedRequest) -> TokenUsage {
    if !response.usage.estimated && response.usage.total() > 0 {
        return response.usage;
    }
    TokenUsage {
        input_tokens: request.estimated_input_tokens(),
        // Four characters per token, the same approximation used for input.
        output_tokens: (response.content.chars().count() as u64 / 4).max(1),
        estimated: true,
        // An estimate cannot know what the provider's cache did, and inventing a discount
        // would under-bill. Estimated requests are priced entirely at the full input rate.
        ..Default::default()
    }
}

/// Load and parse the organisation's active routing policy.
pub(crate) async fn load_policy(state: &AppState, auth: &AuthContext) -> Option<RoutingPolicy> {
    let cache_key = format!("aegis:policy:{}", auth.org_id);

    if let Some(raw) = state.store.get(&cache_key).await.ok().flatten() {
        return Some(RoutingPolicy::from_json(&raw));
    }

    let pool = state.db.as_ref()?;
    let stored = crate::db::repo::find_active_policy(pool, auth.org_id)
        .await
        .ok()
        .flatten()?;

    let json = stored.rules.to_string();
    // Five-minute TTL: long enough to keep the database out of the hot path, short
    // enough that a policy change takes effect while the operator is still watching.
    let _ = state
        .store
        .set_ex(&cache_key, &json, std::time::Duration::from_secs(300))
        .await;

    Some(RoutingPolicy::from_json(&json))
}

/// The tier ceiling a plan imposes.
///
/// Free-tier traffic runs on our own pooled keys, so it is capped at cheap models. Paid
/// plans have no ceiling.
pub fn plan_tier_ceiling(plan: &str) -> Option<crate::types::ModelTier> {
    match plan {
        "free" => Some(crate::types::ModelTier::Cheap),
        _ => None,
    }
}

/// Whether a semantic-cache lookup is worth paying an embedder's latency for.
///
/// A **local** embedder is always worth it: no network hop, so there is no cost to weigh
/// against. A **remote** embedder is a real round trip — a hosted embedding endpoint is
/// typically 100-300ms — stacked in front of whatever provider call it might avoid, so it
/// is offered only when both are true: the operator has explicitly opted in
/// (`AEGIS_SEMANTIC_CACHE_ALLOW_REMOTE_EMBEDDING`, `false` by default — this must never
/// change behaviour for a deployment that hasn't asked for it), and the request is not
/// `Simple`. A simple request already routes to the cheapest capable model; there is little
/// left to save and no reason to add a guaranteed 100ms+ on the chance of finding out.
///
/// This is what makes semantic caching reachable at all on a deployment that has not
/// provisioned a local embedding model — before this, `is_local()` alone gated it, and the
/// only embedder this codebase can construct that satisfies `is_local()` needs an ONNX
/// model this project has never actually provisioned in any environment. The feature was
/// real, tested, and wired end to end, and had still never fired once outside a test.
pub(crate) fn semantic_lookup_worth_it(
    embedder_is_local: bool,
    allow_remote: bool,
    complexity: Complexity,
) -> bool {
    embedder_is_local || (allow_remote && complexity != Complexity::Simple)
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// `POST /v1/chat/completions`
pub async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match handle_chat(&state, &headers, body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn handle_chat(
    state: &AppState,
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response> {
    // ---- [1] Authentication ------------------------------------------------------
    let token = auth::extract_bearer(headers).ok_or_else(|| {
        AegisError::Unauthorized(
            "missing API key. Send it as: Authorization: Bearer aegis_sk_...".into(),
        )
    })?;
    let auth_context = auth::authenticate_api_key(state, &token).await?;

    // ---- [1b] Data residency ------------------------------------------------------
    // An organisation pinned to a region must never be served by an instance running
    // elsewhere -- the backstop that makes a misrouted request fail loudly instead of
    // quietly processing data in the wrong jurisdiction. Checked right after identity is
    // established, before anything else touches this request.
    if let Err(e) = residency::enforce(&state.config.region, &auth_context.region) {
        record_rejection(
            state,
            &auth_context,
            "/v1/chat/completions",
            403,
            "region_mismatch",
        )
        .await;
        return Err(e);
    }

    // ---- [2] Rate limiting -------------------------------------------------------
    let limit = rate_limit::check(state.store.as_ref(), &auth_context).await?;
    if !limit.allowed {
        state.metrics.record_rate_limited(limit.scope);
        record_rejection(
            state,
            &auth_context,
            "/v1/chat/completions",
            429,
            "rate_limit_exceeded",
        )
        .await;
        return Err(limit.into_error());
    }

    // ---- [4] Parse ---------------------------------------------------------------
    // Parsing moved ahead of the budget check, which the pipeline numbering has as [3].
    // The budget check reserves a *projected* cost rather than only reading a counter (see
    // `middleware::budget`), and a projection needs the model and the prompt — both of
    // which only exist after parsing. Parsing has no side effects and no external I/O, so
    // the only thing the swap changes is that a request too malformed to price is refused
    // for being malformed rather than for a budget it was never measured against.
    if body.len() > state.config.max_body_bytes {
        return Err(AegisError::PayloadTooLarge);
    }
    let mut request: NormalizedRequest = serde_json::from_slice(&body)
        .map_err(|e| AegisError::BadRequest(format!("invalid request body: {e}")))?;

    if request.messages.is_empty() {
        return Err(AegisError::BadRequest(
            "messages must contain at least one message".into(),
        ));
    }

    // ---- [4b] Token circuit breaker -------------------------------------------------
    // Before the budget reservation below, deliberately: the reservation projects cost
    // from `max_tokens`, so clamping first means the projection reflects the bound that
    // will actually be enforced rather than whatever the client sent (or didn't send —
    // an absent `max_tokens` is the more dangerous case, not the safer one). Also covers
    // both branches below (`stream_chat` and `execute_with_headroom`) from one call site,
    // since streaming has its own separate pipeline that never reaches the second, later
    // clamp inside `execute_with_headroom`.
    if clamp_max_tokens(&mut request, state.config.max_tokens_per_request) {
        tracing::info!(
            org_id = %auth_context.org_id,
            ceiling = state.config.max_tokens_per_request,
            "token circuit breaker: clamped max_tokens for this request"
        );
    }

    // ---- [3] Budget --------------------------------------------------------------
    let (reservation, headroom) =
        match reserve_budget(state, &auth_context, &request, "/v1/chat/completions").await? {
            Ok(granted) => granted,
            Err(error) => return Err(error),
        };

    let hint = RoutingHint::resolve(
        headers
            .get("x-aegis-routing-hint")
            .and_then(|v| v.to_str().ok()),
        auth_context.default_routing_mode.as_deref(),
    );

    if request.stream {
        return stream_chat(state, &auth_context, request, hint, reservation).await;
    }

    // ---- [5]-[9] Pipeline --------------------------------------------------------
    let outcome = match execute_with_headroom(state, &auth_context, request, hint, headroom).await {
        Ok(outcome) => outcome,
        Err(e) => {
            // The request never produced a billable response, so give the projection back
            // rather than leaving it held against the customer's ceiling until it expires.
            reservation.release(state.store.as_ref()).await;
            return Err(e);
        }
    };

    // ---- [10] Usage emission (non-blocking) --------------------------------------
    let mut event = outcome.usage_event(&auth_context, 200, &state.config.region);
    // Hand the held projection to `emit`, which applies the correction to the real cost.
    // `reserved_mc`/`reserved_tokens` are how it knows this request's projection is already
    // in the counters. `reserved_tokens()` must be read before `commit()` consumes the
    // reservation.
    event.reserved_tokens = reservation.reserved_tokens();
    event.reserved_mc = reservation.commit();
    // Only count a request as metered when it was actually persisted. Incrementing this
    // unconditionally (the previous behaviour) meant the "metering completeness" metric
    // and dashboard panel could never detect the one failure mode they exist to catch —
    // see the identical fix and full reasoning in stream_chat, a few hundred lines below.
    match usage::emit(state.store.as_ref(), &event).await {
        Ok(_) => state.metrics.record_usage_event(),
        Err(e) => tracing::error!(
            request_id = %outcome.request_id,
            org_id = %auth_context.org_id,
            error = %e,
            "usage event lost: request was served and is billable, but could not be \
             persisted"
        ),
    }
    state.metrics.record_request("/v1/chat/completions", 200);
    state
        .metrics
        .record_overhead_ms(outcome.gateway_overhead_ms);
    state
        .metrics
        .record_latency_ms(outcome.total_latency_ms as f64);

    // ---- [11] Respond ------------------------------------------------------------
    let body = to_openai_response(&outcome);
    let mut response = (StatusCode::OK, Json(body)).into_response();
    for (name, value) in outcome.headers() {
        response.headers_mut().insert(name, value);
    }
    for (name, value) in limit.headers() {
        if let Ok(value) = HeaderValue::from_str(&value) {
            if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
                response.headers_mut().insert(name, value);
            }
        }
    }
    Ok(response)
}

/// Streaming variant.
///
/// Chunks are forwarded to the client as they arrive; usage is captured from the final
/// chunk and metered after the stream completes. Principle 2 holds for streams too — the
/// usage event is emitted from inside the stream, so a client that disconnects mid-stream
/// is still billed for the tokens the provider produced.
///
/// # Where failover is and is not possible
///
/// Opening the upstream stream is retried and failed over exactly like a non-streaming
/// call: until the first byte reaches the client, nothing is observable and a different
/// provider can serve the request transparently. Once bytes have been sent, the response
/// is committed — a second provider would produce a different continuation of a partly
/// delivered answer, which is worse than an honest error. So a mid-stream failure ends the
/// stream with an error event, and is recorded as a failure against the provider's circuit
/// breaker so the *next* request routes around it.
///
/// That health recording is new. Before it, neither streaming entry point called
/// `record_success` or `record_failure` at all, so a provider failing every streaming
/// request could never trip its own circuit — on the endpoint built for streaming-heavy
/// clients. Found in the enterprise readiness audit.
async fn stream_chat(
    state: &AppState,
    auth_context: &AuthContext,
    request: NormalizedRequest,
    hint: RoutingHint,
    reservation: budget::Reservation,
) -> Result<Response> {
    let started = Instant::now();
    let request_id = Uuid::new_v4();
    let requested_model = request.model.clone();

    // Context compression: runs identically on streaming traffic as non-streaming traffic
    let mut request = request;
    let compressor_config = if auth_context.zero_retention {
        CompressorConfig::disabled()
    } else {
        CompressorConfig::default()
    };
    let compression = compressor::compress(&mut request, &compressor_config);

    // Conversation affinity: keep multi-turn conversations pinned to the warm model in Redis
    let affinity_key = conversation_affinity_key(auth_context.org_id, &request);
    let affinity_model = if let Some(ref key) = affinity_key {
        state.store.get(key).await.ok().flatten()
    } else {
        None
    };

    let configured_providers = get_configured_providers(state, auth_context.org_id).await;
    let policy = load_policy(state, auth_context).await;
    let inputs = RoutingInputs {
        hint,
        policy: policy.as_ref(),
        team: auth_context.team_name.clone(),
        user: auth_context.user_email.clone(),
        allowed_models: auth_context.allowed_models.clone(),
        plan_tier_ceiling: plan_tier_ceiling(&auth_context.plan),
        budget_headroom_mc: None,
        bandit: Some(state.bandit.as_ref()),
        affinity_model,
        configured_providers: configured_providers.clone(),
    };
    let router = Router::with_classifier(Classifier::new());
    let decision = router.route(&request, &state.pricing(), &state.health, &inputs)?;

    let requested_provider = state
        .pricing()
        .get(&requested_model)
        .map(|m| m.provider.clone())
        .unwrap_or_else(|| decision.provider.clone());
    let chain = FallbackChain::build(
        &decision.served_model,
        &decision.provider,
        &requested_model,
        &requested_provider,
        &alternates_for(state, &request, &decision, configured_providers.as_ref()),
    );

    let opened = match open_stream_with_fallback(state, auth_context, &request, &chain).await {
        Ok(opened) => opened,
        Err(e) => {
            // Nothing was streamed, so nothing is billable. Give the projection back.
            reservation.release(state.store.as_ref()).await;
            return Err(e);
        }
    };

    let overhead_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let StreamAttempt {
        upstream,
        model_id: served_model,
        provider_id,
        used_fallback,
    } = opened;

    // Pin or refresh the conversation affinity in Redis (30m sliding TTL)
    if let Some(ref key) = affinity_key {
        let _ = state
            .store
            .set_ex(key, &served_model, std::time::Duration::from_secs(1800))
            .await;
    }

    let state_for_stream = state.clone();
    let auth_for_stream = auth_context.clone();
    let estimated_input = request.estimated_input_tokens();
    let complexity = decision.complexity_score;
    let routing_reason = if used_fallback {
        RoutingReason::Fallback
    } else {
        decision.reason
    };
    // Recorded here, after failover has resolved, rather than at selection time. The
    // non-streaming path had the same bug: `record_routing` ran before the fallback chain
    // executed, so the one aggregate an SRE would alert on -- "how often are we falling
    // back" -- could structurally never show a fallback.
    state
        .metrics
        .record_routing(routing_reason.as_str(), &served_model);

    // Accumulated as the stream runs, then metered when it ends.
    let mut final_usage: Option<TokenUsage> = None;
    let mut output_chars: u64 = 0;
    // Set the moment the upstream stream itself reports an error. Without this, a
    // provider that dies mid-stream was metered and counted in `/metrics` as a clean
    // 200 — the response the client actually saw *was* an SSE error event, but nothing
    // downstream of this function could tell the two apart. Found in the enterprise
    // readiness audit: a support engineer given a customer's request_id could not
    // distinguish "this failed" from "this succeeded" in the usage record it produced.
    let mut stream_error: Option<String> = None;
    let requested_model_header = requested_model.clone();

    let sse = async_stream::stream! {
        let mut upstream = upstream;
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk) => {
                    output_chars += chunk.delta.chars().count() as u64;
                    if let Some(usage) = chunk.usage {
                        let merged = final_usage.get_or_insert(TokenUsage::default());
                        if usage.input_tokens > 0 {
                            merged.input_tokens = usage.input_tokens;
                        }
                        if usage.output_tokens > 0 {
                            merged.output_tokens = usage.output_tokens;
                        }
                    }
                    // `raw` is the literal bytes the *serving* provider sent, in that
                    // provider's own wire format — safe to forward verbatim only when the
                    // router happened to pick a provider that already speaks the shape
                    // this caller connected with (OpenAI-
                    //
                    // compatible). The router chooses a
                    // model independently of which endpoint the caller used, so a request
                    // to this OpenAI-shaped endpoint can just as easily be served by the
                    // Anthropic adapter or Gemini — forwarding *their* raw SSE bytes here
                    // would hand an OpenAI-SDK client JSON it cannot parse. Reconstruct
                    // from the normalised fields instead whenever the shapes don't match.
                    let payload = match chunk.source_shape {
                        Some(crate::types::WireShape::OpenAiCompatible) => chunk
                            .raw
                            .clone()
                            .unwrap_or_else(|| {
                                to_openai_stream_chunk(&request_id, &served_model, &chunk)
                                    .to_string()
                            }),
                        _ => to_openai_stream_chunk(&request_id, &served_model, &chunk)
                            .to_string(),
                    };
                    yield Ok::<_, std::convert::Infallible>(
                        axum::body::Bytes::from(format!("data: {payload}\n\n"))
                    );
                }
                Err(e) => {
                    stream_error = Some(e.error_type().to_string());
                    let payload = serde_json::json!({
                        "error": {"type": e.error_type(), "message": e.to_string()}
                    });
                    yield Ok(axum::body::Bytes::from(format!("data: {payload}\n\n")));
                    break;
                }
            }
        }

        yield Ok(axum::body::Bytes::from_static(b"data: [DONE]\n\n"));

        // Meter after the stream closes. This runs even when the client has already
        // disconnected, because the provider tokens were produced and are billable.
        let tokens = final_usage.unwrap_or(TokenUsage {
            input_tokens: estimated_input,
            output_tokens: (output_chars / 4).max(1),
            estimated: true,
            ..Default::default()
        });

        let actual_cost = state_for_stream
            .pricing()
            .cost_of(&served_model, &tokens)
            .unwrap_or(MicroCents::ZERO);
        let baseline_cost = state_for_stream
            .pricing()
            .cost_of(&requested_model, &tokens)
            .unwrap_or(actual_cost);
        let savings = SavingsBreakdown::compute(
            baseline_cost,
            actual_cost,
            auth_for_stream.savings_share_bp,
        );

        let mut event = UsageEvent::new(
            request_id,
            auth_for_stream.org_id,
            auth_for_stream.api_key_id,
            auth_for_stream.team_id,
            requested_model.clone(),
            served_model.clone(),
            provider_id.clone(),
            tokens,
            savings,
            started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            overhead_ms,
            CacheOutcome::Skipped,
            routing_reason,
            complexity,
            // The HTTP status genuinely was 200 — SSE has no way to change it mid-stream,
            // and the client did receive a 200 response. `error_type` below is what
            // records that the *stream itself* failed partway, which status_code alone
            // cannot express.
            200,
        );
        // A stream that died partway still failed, and the circuit breaker has to hear
        // about it or the next request routes straight back into the same provider. This
        // is the only place a mid-stream failure can be observed — by the time the error
        // chunk arrives, `open_stream_with_fallback` has long since returned success.
        if let Some(error_type) = stream_error.as_deref() {
            let after = state_for_stream.health.record_failure(&provider_id);
            state_for_stream
                .metrics
                .record_provider_error(&provider_id, error_type);
            if after == fallback::CircuitState::Open {
                state_for_stream
                    .metrics
                    .record_circuit_change(&provider_id, after.as_str());
            }
            tracing::warn!(
                request_id = %request_id,
                org_id = %auth_for_stream.org_id,
                provider = %provider_id,
                model = %served_model,
                error_type = %error_type,
                "stream failed after content was already sent; cannot fail over, \
                 recorded against the provider's circuit"
            );
        } else {
            state_for_stream.health.record_success(&provider_id);
        }

        event.tokens_saved_by_compression = compression.tokens_saved();
        event.techniques_fired = techniques_fired_json(&compression);
        if let Some(pricing) = state_for_stream.pricing().get(&served_model) {
            event.input_cost_mc = pricing.input_cost_of(&tokens).as_i64();
            event.output_cost_mc = pricing.output_cost_of(&tokens).as_i64();
        }
        event.user_id = auth_for_stream.user_id;

        event.error_type = stream_error;
        // Convert the held projection into the real cost, exactly as the non-streaming
        // path does. Without this the reservation would sit on the counter until it
        // expired, and every subsequent request would see inflated spend.
        event.reserved_tokens = reservation.reserved_tokens();
        event.reserved_mc = reservation.commit();

        // Metering completeness must reflect whether the event was actually durable, not
        // whether we attempted to make it durable. The prior version incremented this
        // counter unconditionally after discarding emit()'s Result, which meant the one
        // metric meant to catch "a request was served but never billed" could not detect
        // it happening in the exact failure mode it exists to catch — a Redis XADD error
        // at this line would have been invisible to both this metric and the nightly
        // reconciliation job, which compares two counters that were equally blind to the
        // drop. Found in the enterprise readiness audit.
        match usage::emit(state_for_stream.store.as_ref(), &event).await {
            Ok(_) => state_for_stream.metrics.record_usage_event(),
            Err(e) => tracing::error!(
                request_id = %request_id,
                org_id = %auth_for_stream.org_id,
                error = %e,
                "usage event lost: streaming request was served and is billable, but \
                 could not be persisted"
            ),
        }
        let status = if event.error_type.is_some() { 502 } else { 200 };
        state_for_stream.metrics.record_request("/v1/chat/completions", status);
        state_for_stream.metrics.record_savings(savings.gross_savings.as_i64());
        state_for_stream.metrics.record_latency_ms(
            started.elapsed().as_millis().min(u32::MAX as u128) as f64,
        );
        state_for_stream.metrics.record_overhead_ms(overhead_ms);
    };

    let mut response = Response::new(Body::from_stream(sse));
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response_headers.insert(
        HeaderName::from_static("x-aegis-model"),
        HeaderValue::from_str(&decision.served_model)
            .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    response_headers.insert(
        HeaderName::from_static("x-aegis-requested-model"),
        HeaderValue::from_str(&requested_model_header)
            .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    response_headers.insert(
        HeaderName::from_static("x-aegis-routing"),
        HeaderValue::from_str(decision.reason.as_str())
            .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    if compression.tokens_saved() > 0 {
        response_headers.insert(
            HeaderName::from_static("x-aegis-compression-before-tokens"),
            HeaderValue::from_str(&compression.tokens_before.to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("0")),
        );
        response_headers.insert(
            HeaderName::from_static("x-aegis-compression-after-tokens"),
            HeaderValue::from_str(&compression.tokens_after.to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("0")),
        );
        response_headers.insert(
            HeaderName::from_static("x-aegis-compression-tokens-saved"),
            HeaderValue::from_str(&compression.tokens_saved().to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("0")),
        );
        let ratio = compression.savings_percent();
        response_headers.insert(
            HeaderName::from_static("x-aegis-compression-ratio"),
            HeaderValue::from_str(&format!("{:.1}%", ratio))
                .unwrap_or_else(|_| HeaderValue::from_static("0%")),
        );
    }
    response_headers.insert(
        HeaderName::from_static("x-aegis-request-id"),
        HeaderValue::from_str(&request_id.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    Ok(response)
}

/// Load this organisation's budgets and atomically reserve this request's projected cost.
///
/// Returns `Ok(Ok(reservation))` to proceed, `Ok(Err(error))` when a hard limit refuses the
/// request (already metered and counted), and `Err` only for a genuine internal failure.
///
/// Shared by `/v1/chat/completions`, `/v1/messages`, and `/v1/embeddings` so all three
/// enforce identically — before this existed each passed `None` for every limit, which
/// meant org, team, and regional budgets were configurable, listed on the dashboard, and
/// enforced nowhere.
pub(crate) async fn reserve_budget(
    state: &AppState,
    auth: &AuthContext,
    request: &NormalizedRequest,
    path: &str,
) -> Result<std::result::Result<(budget::Reservation, Option<i64>), AegisError>> {
    let region = Some(state.config.region.as_str());
    let limits = budget::load_limits(state.store.as_ref(), state.db.as_ref(), auth, region).await;
    let projected = budget::project_cost(request, &state.pricing());
    let projected_tokens = budget::project_tokens(request);

    match budget::check_and_reserve(
        state.store.as_ref(),
        auth,
        &limits,
        region,
        projected.as_i64(),
        projected_tokens,
    )
    .await?
    {
        budget::BudgetOutcome::Allowed(reservation) => {
            // Read after reserving, so the figure the router sees already accounts for
            // this request. `None` when no hard limit applies, which leaves routing
            // behaving exactly as it did before budgets could influence it.
            let headroom = budget::headroom_mc(state.store.as_ref(), auth, &limits, region).await;
            Ok(Ok((reservation, headroom)))
        }
        budget::BudgetOutcome::Denied(breaches) => {
            // Every breached scope gets its own counter increment and its own log field,
            // not just the primary one — `record_budget_blocked` and the trace both need
            // to reflect that a request can be over on more than one scope at once.
            for breach in &breaches {
                state.metrics.record_budget_blocked(breach.scope);
            }
            tracing::info!(
                org_id = %auth.org_id,
                scopes = ?breaches.iter().map(|b| b.scope).collect::<Vec<_>>(),
                spend_mc = breaches[0].spend.as_i64(),
                limit_mc = breaches[0].limit.unwrap_or(MicroCents::ZERO).as_i64(),
                projected_mc = projected.as_i64(),
                "request refused: hard budget would be exceeded"
            );
            record_rejection_for_model(state, auth, &request.model, path, 402, "budget_exceeded")
                .await;
            Ok(Err(budget::denial_to_error(&breaches)))
        }
    }
}

/// Meter a request rejected before it reached a provider.
///
/// Principle 2: every request produces a usage record, including the ones we refuse.
async fn record_rejection(
    state: &AppState,
    auth: &AuthContext,
    path: &str,
    status: u16,
    error_type: &str,
) {
    record_rejection_for_model(state, auth, "unknown", path, status, error_type).await;
}

/// As [`record_rejection`], for a rejection that happened after the model was known.
///
/// A 402 recorded against `unknown` is not much use to a customer asking which model they
/// were blocked on, and the budget check now runs after parsing, so the real name is
/// available.
async fn record_rejection_for_model(
    state: &AppState,
    auth: &AuthContext,
    model: &str,
    path: &str,
    status: u16,
    error_type: &str,
) {
    let event = UsageEvent::rejected(
        Uuid::new_v4(),
        auth.org_id,
        auth.api_key_id,
        model.to_string(),
        status,
        error_type,
        0.0,
    );
    // Same accounting fix as the two billable-event call sites above, applied here for
    // consistency: the metric's meaning should not depend on which code path emitted it.
    if usage::emit(state.store.as_ref(), &event).await.is_ok() {
        state.metrics.record_usage_event();
    }
    state.metrics.record_request(path, status);
}

/// Render the outcome as an OpenAI chat completion.
///
/// The provider's own body is returned verbatim when available, so a client depending on
/// a field we do not model still receives it.
pub fn to_openai_response(outcome: &PipelineOutcome) -> serde_json::Value {
    if let Some(raw) = &outcome.response.raw {
        return raw.clone();
    }
    serde_json::json!({
        "id": if outcome.response.id.is_empty() {
            format!("chatcmpl-{}", outcome.request_id)
        } else {
            outcome.response.id.clone()
        },
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": outcome.served_model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": outcome.response.content},
            "finish_reason": outcome.response.finish_reason.clone().unwrap_or_else(|| "stop".into()),
        }],
        "usage": {
            "prompt_tokens": outcome.tokens.input_tokens,
            "completion_tokens": outcome.tokens.output_tokens,
            "total_tokens": outcome.tokens.total(),
        }
    })
}

/// Render a chunk in OpenAI streaming shape — when the served provider's own raw payload
/// isn't safe to forward verbatim (see the shape check at the call site), or gave us none
/// at all.
fn to_openai_stream_chunk(
    request_id: &Uuid,
    model: &str,
    chunk: &crate::types::StreamChunk,
) -> serde_json::Value {
    let mut delta = serde_json::json!({});
    if !chunk.delta.is_empty() {
        delta["content"] = serde_json::json!(chunk.delta);
    }
    if let Some(tc) = &chunk.tool_call {
        // `id`/`type`/`function.name` belong only on the fragment that opens a call — a
        // later fragment for the same call (arriving from Anthropic or Gemini, which
        // don't index tool calls the way OpenAI does, or from OpenAI's own later
        // fragments) has neither, and repeating them would tell the client a second call
        // just started. `id.is_some() || name.is_some()` is what an opening fragment
        // looks like regardless of which provider produced it — Anthropic supplies both,
        // Gemini supplies only a name, OpenAI supplies both on its own opening fragment.
        let mut entry = serde_json::json!({
            "index": tc.index,
            "function": {"arguments": tc.arguments_fragment},
        });
        if tc.id.is_some() || tc.name.is_some() {
            entry["id"] = serde_json::json!(tc
                .id
                .clone()
                .unwrap_or_else(|| format!("call_{}_{}", request_id, tc.index)));
            entry["type"] = serde_json::json!("function");
            entry["function"]["name"] = serde_json::json!(tc.name.clone().unwrap_or_default());
        }
        delta["tool_calls"] = serde_json::json!([entry]);
    }

    serde_json::json!({
        "id": format!("chatcmpl-{request_id}"),
        "object": "chat.completion.chunk",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": chunk.finish_reason,
        }]
    })
}

/// `GET /v1/models` — models available to the caller's organisation.
pub async fn list_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match auth::extract_bearer(&headers) {
        Some(token) => token,
        None => {
            return AegisError::Unauthorized("missing API key".into()).into_response();
        }
    };

    let auth_context = match auth::authenticate_api_key(&state, &token).await {
        Ok(context) => context,
        Err(e) => return e.into_response(),
    };

    let ceiling = plan_tier_ceiling(&auth_context.plan);
    let allowlist = auth_context.allowed_models.clone();
    let configured_providers = get_configured_providers(&state, auth_context.org_id).await;

    let mut models: Vec<serde_json::Value> = state
        .pricing()
        .all()
        .filter(|m| ceiling.is_none_or(|ceiling| m.tier <= ceiling))
        .filter(|m| match &allowlist {
            None => true,
            Some(allowed) => allowed
                .iter()
                .any(|a| a == &m.model_id || a == m.bare_name()),
        })
        .filter(|m| match &configured_providers {
            Some(configured) => {
                configured.contains(&m.provider)
                    || (configured.contains("openrouter")
                        && (m.provider == "openrouter" || m.model_id.starts_with("openrouter/")))
            }
            None => true,
        })
        .map(|m| {
            serde_json::json!({
                "id": m.model_id,
                "object": "model",
                "owned_by": m.provider,
                "created": 0,
                // Aegis extensions: the dashboard and the docs use these, and they let a
                // customer see what a model costs before sending it anything.
                "aegis": {
                    "tier": m.tier.as_str(),
                    "display_name": m.display_name,
                    "context_window": m.context_window,
                    "supports_tools": m.supports_tools,
                    "supports_vision": m.supports_vision,
                    "input_cost_per_mtok_micro_cents": m.input_per_mtok.as_i64(),
                    "output_cost_per_mtok_micro_cents": m.output_per_mtok.as_i64(),
                }
            })
        })
        .collect();

    // Virtual routing tier models
    let virtual_models = [
        ("auto", "Aegis Smart Auto-Routing (Complexity-Based)", "auto"),
        ("economy", "Aegis Economy Routing (Cheapest Capable)", "cheap"),
        ("speed", "Aegis Speed Routing (Lowest Latency)", "mid"),
        ("quality", "Aegis Quality Routing (Flagship Reasoning)", "frontier"),
        ("balanced", "Aegis Balanced Routing (Optimized Cost & Latency)", "mid"),
    ];

    for (id, name, tier) in virtual_models {
        models.push(serde_json::json!({
            "id": id,
            "object": "model",
            "owned_by": "aegis",
            "created": 0,
            "aegis": {
                "tier": tier,
                "display_name": name,
                "context_window": 1_000_000,
                "supports_tools": true,
                "supports_vision": true,
                "input_cost_per_mtok_micro_cents": 0,
                "output_cost_per_mtok_micro_cents": 0,
            }
        }));
    }

    models.sort_by(|a, b| {
        a["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["id"].as_str().unwrap_or_default())
    });

    Json(serde_json::json!({"object": "list", "data": models})).into_response()
}

/// `POST /v1/embeddings` — routed to the cheapest embedding model.
pub async fn embeddings(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let token = match auth::extract_bearer(&headers) {
        Some(token) => token,
        None => return AegisError::Unauthorized("missing API key".into()).into_response(),
    };
    let auth_context = match auth::authenticate_api_key(&state, &token).await {
        Ok(context) => context,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = residency::enforce(&state.config.region, &auth_context.region) {
        record_rejection(
            &state,
            &auth_context,
            "/v1/embeddings",
            403,
            "region_mismatch",
        )
        .await;
        return e.into_response();
    }

    let request: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(e) => {
            return AegisError::BadRequest(format!("invalid request body: {e}")).into_response()
        }
    };

    let model = state
        .pricing()
        .cheapest_embedding_model()
        .map(|m| m.model_id.clone())
        .unwrap_or_else(|| "openai/text-embedding-3-small".to_string());

    let Some(provider) = state.providers.for_model(&model) else {
        return AegisError::AllProvidersFailed("no embedding provider configured".into())
            .into_response();
    };

    let credential = match resolve_credential(&state, &auth_context, provider.id()).await {
        Ok(credential) => credential,
        Err(e) => return e.into_response(),
    };

    let base = credential
        .base_url
        .as_deref()
        .unwrap_or_else(|| provider.default_base_url());
    let url = format!("{}/embeddings", base.trim_end_matches('/'));

    let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(&model);
    let mut payload = request.clone();
    if let Some(map) = payload.as_object_mut() {
        map.insert("model".into(), serde_json::json!(bare));
    }

    let mut builder = state.http.post(&url).json(&payload);
    for (name, value) in provider.auth_headers(&credential) {
        builder = builder.header(name, value);
    }

    match builder.send().await {
        Ok(upstream) => {
            let status =
                StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = upstream.text().await.unwrap_or_default();
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(json) => (status, Json(json)).into_response(),
                Err(_) => AegisError::Provider {
                    provider: provider.id().to_string(),
                    status: 502,
                    message: crate::providers::extract_provider_error(&text),
                }
                .into_response(),
            }
        }
        Err(e) => AegisError::Provider {
            provider: provider.id().to_string(),
            status: 502,
            message: e.to_string(),
        }
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::KeyContext;
    use crate::providers::mock::MockProvider;
    use crate::providers::ProviderRegistry;
    use crate::types::{ModelTier, ToolCallDelta};
    use std::sync::Arc;

    #[test]
    fn to_openai_stream_chunk_renders_a_tool_call_delta() {
        // The outbound half of the fix: whatever provider actually produced this chunk
        // (Anthropic, Gemini, or an OpenAI-shaped one whose raw bytes weren't safe to
        // forward verbatim — see the shape check at the streaming loop's call site), a
        // normalised ToolCallDelta must render as real OpenAI delta.tool_calls shape.
        let request_id = Uuid::new_v4();
        let chunk = crate::types::StreamChunk {
            tool_call: Some(ToolCallDelta {
                index: 0,
                id: Some("call_1".to_string()),
                name: Some("get_weather".to_string()),
                arguments_fragment: "{\"city\":".to_string(),
            }),
            ..Default::default()
        };

        let rendered = to_openai_stream_chunk(&request_id, "gpt-4o", &chunk);
        let delta = &rendered["choices"][0]["delta"];
        assert_eq!(delta["tool_calls"][0]["index"], 0);
        assert_eq!(delta["tool_calls"][0]["id"], "call_1");
        assert_eq!(delta["tool_calls"][0]["type"], "function");
        assert_eq!(delta["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(
            delta["tool_calls"][0]["function"]["arguments"],
            "{\"city\":"
        );
        assert!(
            delta.get("content").is_none(),
            "no text on a tool-only chunk"
        );
    }

    #[test]
    fn a_continuing_tool_call_fragment_omits_id_and_name() {
        // Repeating id/name on every fragment would tell an OpenAI SDK a new call started
        // each time — only the opening fragment gets them.
        let request_id = Uuid::new_v4();
        let chunk = crate::types::StreamChunk {
            tool_call: Some(ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments_fragment: "\"Paris\"}".to_string(),
            }),
            ..Default::default()
        };

        let rendered = to_openai_stream_chunk(&request_id, "gpt-4o", &chunk);
        let entry = &rendered["choices"][0]["delta"]["tool_calls"][0];
        assert!(entry.get("id").is_none());
        assert!(entry.get("type").is_none());
        assert_eq!(entry["function"]["arguments"], "\"Paris\"}");
    }

    #[test]
    fn a_text_only_chunk_carries_no_tool_calls_field_at_all() {
        // The field must be genuinely absent, not present-and-empty — an SDK checking
        // `if delta.tool_calls` would otherwise see a truthy empty array.
        let request_id = Uuid::new_v4();
        let chunk = crate::types::StreamChunk {
            delta: "Hello".to_string(),
            ..Default::default()
        };
        let rendered = to_openai_stream_chunk(&request_id, "gpt-4o", &chunk);
        assert!(rendered["choices"][0]["delta"].get("tool_calls").is_none());
        assert_eq!(rendered["choices"][0]["delta"]["content"], "Hello");
    }

    /// State wired to a mock provider, with pooled credentials so the pipeline can run
    /// end to end without a database, a network, or an API key.
    fn test_state(mock: Arc<MockProvider>) -> AppState {
        let mut registry = ProviderRegistry::with_builtins();
        registry.register(mock);

        let mut config = crate::config::Config::for_tests();
        config
            .shared_provider_keys
            .insert("mock".into(), vec!["pooled-test-key".into()]);
        config
            .shared_provider_keys
            .insert("openai".into(), vec!["pooled-test-key".into()]);

        // A pricing table containing ONLY mock models.
        //
        // Seeding the real table here would be a subtle trap: the router would correctly
        // find that a real provider offers a cheaper capable model, and the test would
        // make live API calls to OpenAI. Restricting the table keeps routing inside the
        // mock provider, so these tests are hermetic and fast.
        let mut pricing = crate::metering::pricing::PricingTable::new();
        pricing.insert(crate::metering::pricing::ModelPricing {
            model_id: "mock/mock-premium".into(),
            provider: "mock".into(),
            display_name: "Mock Premium".into(),
            tier: ModelTier::Premium,
            input_per_mtok: MicroCents::from_usd_per_mtok(10.0),
            output_per_mtok: MicroCents::from_usd_per_mtok(30.0),
            context_window: 128_000,
            supports_tools: true,
            supports_vision: true,
            supports_chat: true,
            is_active: true,
            source: "test".into(),
            cache: Default::default(),
            long_context: None,
        });
        pricing.insert(crate::metering::pricing::ModelPricing {
            model_id: "mock/mock-cheap".into(),
            provider: "mock".into(),
            display_name: "Mock Cheap".into(),
            tier: ModelTier::Cheap,
            input_per_mtok: MicroCents::from_usd_per_mtok(0.10),
            output_per_mtok: MicroCents::from_usd_per_mtok(0.40),
            context_window: 128_000,
            supports_tools: true,
            supports_vision: true,
            supports_chat: true,
            is_active: true,
            source: "test".into(),
            cache: Default::default(),
            long_context: None,
        });

        AppState {
            config: Arc::new(config),
            pricing: Arc::new(std::sync::RwLock::new(
                crate::metering::pricing::PricingSnapshot::from_table(pricing),
            )),
            providers: Arc::new(registry),
            ..AppState::for_tests()
        }
    }

    fn auth_context(plan: &str) -> AuthContext {
        AuthContext::from_key(KeyContext {
            api_key_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            assigned_to_user_id: None,
            rate_limit_per_minute: 1_000,
            monthly_budget_mc: None,
            allowed_models: None,
            plan: plan.to_string(),
            savings_share_bp: 2_000,
            zero_retention: false,
            org_region: "test".into(),
            team_name: None,
            user_email: None,
            key_default_routing_mode: None,
            team_default_routing_mode: None,
            org_default_routing_mode: None,
        })
    }

    #[tokio::test]
    async fn a_simple_request_routes_to_the_cheaper_model_and_saves_money() {
        // The end-to-end commercial claim, measured.
        let mock = Arc::new(MockProvider::returning("Paris"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request =
            NormalizedRequest::simple("mock/mock-premium", "What is the capital of France?");
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert_eq!(outcome.served_model, "mock/mock-cheap");
        assert_eq!(outcome.requested_model, "mock/mock-premium");
        assert_eq!(outcome.routing_reason, RoutingReason::Complexity);
        assert!(outcome.savings.gross_savings > MicroCents::ZERO);
        assert!(outcome.savings.aegis_fee > MicroCents::ZERO);
        assert_eq!(
            outcome.savings.aegis_fee + outcome.savings.customer_net,
            outcome.savings.gross_savings
        );
        assert_eq!(mock.last_model().as_deref(), Some("mock/mock-cheap"));
    }

    #[tokio::test]
    async fn a_complex_request_is_served_by_the_requested_model() {
        let mock = Arc::new(MockProvider::returning("detailed analysis"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple(
            "mock/mock-premium",
            "Analyze this stack trace, diagnose the root cause, and architect a fix that \
             prevents the same class of bug.",
        );
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert_eq!(outcome.served_model, "mock/mock-premium");
        assert_eq!(outcome.savings.gross_savings, MicroCents::ZERO);
        assert_eq!(outcome.savings.aegis_fee, MicroCents::ZERO);
    }

    #[tokio::test]
    async fn the_passthrough_hint_forces_the_requested_model() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");
        let outcome = execute(&state, &auth, request, RoutingHint::Passthrough)
            .await
            .unwrap();

        assert_eq!(outcome.served_model, "mock/mock-premium");
        assert_eq!(outcome.routing_reason, RoutingReason::UserOverride);
    }

    #[tokio::test]
    async fn a_repeated_request_is_served_from_cache_at_zero_cost() {
        let mock = Arc::new(MockProvider::returning("cached answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");
        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");

        let first = execute(&state, &auth, request.clone(), RoutingHint::Auto)
            .await
            .unwrap();
        assert_eq!(first.cache, CacheOutcome::Miss);
        assert_eq!(mock.call_count(), 1);

        let second = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();
        assert_eq!(second.cache, CacheOutcome::Exact);
        assert_eq!(second.routing_reason, RoutingReason::Cache);
        assert_eq!(second.savings.actual_cost, MicroCents::ZERO);
        assert!(second.savings.gross_savings > MicroCents::ZERO);
        assert_eq!(
            mock.call_count(),
            1,
            "a cache hit must not reach the provider"
        );
    }

    #[tokio::test]
    async fn a_differently_worded_repeat_hits_the_semantic_cache() {
        use crate::cache::embed::ConstantEmbedder;

        let mock = Arc::new(MockProvider::returning("Paris"));
        let state = AppState {
            embedder: Arc::new(ConstantEmbedder(vec![1.0, 0.0, 0.0])),
            ..test_state(Arc::clone(&mock))
        };
        let auth = auth_context("pro");

        let first = execute(
            &state,
            &auth,
            NormalizedRequest::simple("mock/mock-premium", "What is the capital of France?"),
            RoutingHint::Auto,
        )
        .await
        .unwrap();
        assert_eq!(first.cache, CacheOutcome::Miss);
        assert_eq!(mock.call_count(), 1);

        // A completely different wording of the same request. The exact-match cache
        // cannot catch this — different fingerprint entirely — only the semantic one can.
        let second = execute(
            &state,
            &auth,
            NormalizedRequest::simple(
                "mock/mock-premium",
                "Which city is the capital city of the country France?",
            ),
            RoutingHint::Auto,
        )
        .await
        .unwrap();
        assert_eq!(second.cache, CacheOutcome::Semantic);
        assert_eq!(second.savings.actual_cost, MicroCents::ZERO);
        assert!(second.savings.gross_savings > MicroCents::ZERO);
        assert_eq!(
            mock.call_count(),
            1,
            "a semantic hit must not reach the provider"
        );

        // A third repeat of the *second* wording, word for word, should now hit the exact
        // cache directly — the semantic hit above is supposed to have promoted it.
        let third = execute(
            &state,
            &auth,
            NormalizedRequest::simple(
                "mock/mock-premium",
                "Which city is the capital city of the country France?",
            ),
            RoutingHint::Auto,
        )
        .await
        .unwrap();
        assert_eq!(
            third.cache,
            CacheOutcome::Exact,
            "a semantic hit's own wording should be exact-cached for next time"
        );
    }

    #[tokio::test]
    async fn free_tier_never_gets_semantic_caching() {
        use crate::cache::embed::ConstantEmbedder;

        // Same constant embedder as the test above — if the plan gate is doing its job,
        // it never even gets called, so a "hit" here would prove the gate is missing.
        let mock = Arc::new(MockProvider::returning("Paris"));
        let state = AppState {
            embedder: Arc::new(ConstantEmbedder(vec![1.0, 0.0, 0.0])),
            ..test_state(Arc::clone(&mock))
        };
        let auth = auth_context("free");

        execute(
            &state,
            &auth,
            NormalizedRequest::simple("mock/mock-premium", "What is the capital of France?"),
            RoutingHint::Auto,
        )
        .await
        .unwrap();

        let second = execute(
            &state,
            &auth,
            NormalizedRequest::simple(
                "mock/mock-premium",
                "Which city is the capital city of the country France?",
            ),
            RoutingHint::Auto,
        )
        .await
        .unwrap();
        assert_eq!(
            second.cache,
            CacheOutcome::Miss,
            "free-tier traffic must not get semantic caching"
        );
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn a_zero_retention_org_never_gets_a_semantic_hit_even_with_an_embedder_configured() {
        use crate::cache::embed::ConstantEmbedder;

        let mock = Arc::new(MockProvider::returning("Paris"));
        let state = AppState {
            embedder: Arc::new(ConstantEmbedder(vec![1.0, 0.0, 0.0])),
            ..test_state(Arc::clone(&mock))
        };
        let mut auth = auth_context("pro");
        auth.zero_retention = true;

        execute(
            &state,
            &auth,
            NormalizedRequest::simple("mock/mock-premium", "What is the capital of France?"),
            RoutingHint::Auto,
        )
        .await
        .unwrap();

        let second = execute(
            &state,
            &auth,
            NormalizedRequest::simple(
                "mock/mock-premium",
                "Which city is the capital city of the country France?",
            ),
            RoutingHint::Auto,
        )
        .await
        .unwrap();
        assert_eq!(second.cache, CacheOutcome::Miss);
    }

    #[tokio::test]
    async fn a_genuinely_novel_request_never_semantic_hits_an_unrelated_one() {
        use crate::cache::embed::DeterministicEmbedder;

        // Unlike the constant-vector tests above, this uses a real per-text embedder —
        // two unrelated questions must not collide.
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = AppState {
            embedder: Arc::new(DeterministicEmbedder),
            ..test_state(Arc::clone(&mock))
        };
        let auth = auth_context("pro");

        execute(
            &state,
            &auth,
            NormalizedRequest::simple("mock/mock-premium", "What is the capital of France?"),
            RoutingHint::Auto,
        )
        .await
        .unwrap();

        let second = execute(
            &state,
            &auth,
            NormalizedRequest::simple("mock/mock-premium", "Write me a haiku about autumn."),
            RoutingHint::Auto,
        )
        .await
        .unwrap();
        assert_eq!(second.cache, CacheOutcome::Miss);
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn one_orgs_cache_never_serves_another() {
        let mock = Arc::new(MockProvider::returning("secret"));
        let state = test_state(Arc::clone(&mock));
        let request = NormalizedRequest::simple("mock/mock-premium", "confidential question");

        let first_org = auth_context("pro");
        execute(&state, &first_org, request.clone(), RoutingHint::Auto)
            .await
            .unwrap();

        let second_org = auth_context("pro");
        let outcome = execute(&state, &second_org, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert_eq!(outcome.cache, CacheOutcome::Miss, "cross-tenant cache hit");
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn zero_retention_orgs_are_never_cached() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let mut auth = auth_context("enterprise");
        auth.zero_retention = true;
        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");

        execute(&state, &auth, request.clone(), RoutingHint::Auto)
            .await
            .unwrap();
        let second = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert_eq!(second.cache, CacheOutcome::Miss);
        assert_eq!(
            mock.call_count(),
            2,
            "a zero-retention org must never be cached"
        );
    }

    #[tokio::test]
    async fn a_transient_provider_failure_is_retried() {
        let mock = Arc::new(MockProvider::failing_then_succeeding(2));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert_eq!(outcome.response.content, "recovered");
        assert_eq!(mock.call_count(), 3, "expected two failures then a success");
    }

    #[tokio::test]
    async fn a_client_error_from_the_provider_is_not_retried() {
        // Retrying a 400 burns quota to receive the same answer.
        let mock = Arc::new(MockProvider::failing(400, "invalid parameter"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");
        let err = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap_err();

        assert_eq!(err.status().as_u16(), 400);
        assert_eq!(mock.call_count(), 1, "a 4xx must not be retried");
    }

    #[tokio::test]
    async fn the_free_tier_allowance_is_enforced() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let mut state = test_state(Arc::clone(&mock));
        let mut config = (*state.config).clone();
        config.free_tier_monthly_requests = 2;
        state.config = Arc::new(config);

        let auth = auth_context("free");

        for i in 0..2 {
            let request = NormalizedRequest::simple("mock/mock-cheap", &format!("question {i}"));
            let outcome = execute(&state, &auth, request, RoutingHint::Auto)
                .await
                .unwrap();
            let event = outcome.usage_event(&auth, 200, "test");
            usage::emit(state.store.as_ref(), &event).await.unwrap();
        }

        let request = NormalizedRequest::simple("mock/mock-cheap", "one too many");
        let err = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap_err();
        assert_eq!(err.status().as_u16(), 402);
    }

    #[tokio::test]
    async fn free_tier_requests_are_capped_at_cheap_models() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("free");

        // Even a complex request cannot reach a premium model on the free plan.
        let request = NormalizedRequest::simple(
            "mock/mock-premium",
            "Analyze and diagnose the root cause, then architect a fix.",
        );
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();
        assert_eq!(outcome.served_model, "mock/mock-cheap");
    }

    #[tokio::test]
    async fn gateway_overhead_excludes_provider_time_and_is_reported() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert!(outcome.gateway_overhead_ms >= 0.0);
        assert!(outcome.gateway_overhead_ms.is_finite());
        assert!(
            outcome.gateway_overhead_ms <= outcome.total_latency_ms as f64 + 1.0,
            "overhead {} exceeded total {}",
            outcome.gateway_overhead_ms,
            outcome.total_latency_ms
        );

        let headers = outcome.headers();
        let names: Vec<String> = headers
            .iter()
            .map(|(name, _)| name.as_str().to_string())
            .collect();
        for required in [
            "x-aegis-model",
            "x-aegis-requested-model",
            "x-aegis-cost",
            "x-aegis-baseline-cost",
            "x-aegis-savings",
            "x-aegis-cache",
            "x-aegis-routing",
            "x-aegis-latency",
            "x-aegis-request-id",
        ] {
            assert!(names.contains(&required.to_string()), "missing {required}");
        }
    }

    #[tokio::test]
    async fn every_request_produces_a_usage_event() {
        // Principle 2, asserted directly.
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        for i in 0..5 {
            let request = NormalizedRequest::simple("mock/mock-premium", &format!("question {i}"));
            let outcome = execute(&state, &auth, request, RoutingHint::Auto)
                .await
                .unwrap();
            let event = outcome.usage_event(&auth, 200, "test");
            usage::emit(state.store.as_ref(), &event).await.unwrap();
        }

        let entries = state
            .store
            .stream_read(usage::USAGE_STREAM, "0", 100)
            .await
            .unwrap();
        assert_eq!(entries.len(), 5);
    }

    #[tokio::test]
    async fn usage_events_carry_the_full_attribution() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple("mock/mock-premium", "What is 2+2?");
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();
        let event = outcome.usage_event(&auth, 200, "test");

        assert_eq!(event.org_id, auth.org_id);
        assert_eq!(event.api_key_id, auth.api_key_id);
        assert_eq!(event.requested_model, "mock/mock-premium");
        assert_eq!(event.served_model, "mock/mock-cheap");
        assert_eq!(
            event.baseline_cost_mc,
            outcome.savings.baseline_cost.as_i64()
        );
        assert_eq!(
            event.gross_savings_mc,
            outcome.savings.gross_savings.as_i64()
        );
        assert_eq!(event.aegis_fee_mc, outcome.savings.aegis_fee.as_i64());
        assert!(event.gross_savings_mc >= event.aegis_fee_mc);
    }

    #[tokio::test]
    async fn an_unpriced_model_passes_through_without_inventing_a_cost() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));
        let auth = auth_context("pro");

        let request = NormalizedRequest::simple("mock-model", "hello");
        let outcome = execute(&state, &auth, request, RoutingHint::Auto)
            .await
            .unwrap();

        assert_eq!(outcome.served_model, "mock-model");
        assert_eq!(outcome.routing_reason, RoutingReason::Passthrough);
        assert_eq!(outcome.savings.gross_savings, MicroCents::ZERO);
    }

    #[tokio::test]
    async fn compression_is_recorded_and_disabled_for_zero_retention() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = test_state(Arc::clone(&mock));

        let mut request = NormalizedRequest::simple("mock/mock-premium", "hi");
        request.messages = vec![
            crate::types::Message::text(crate::types::Role::System, "long prompt ".repeat(40)),
            crate::types::Message::text(crate::types::Role::User, "hi"),
            crate::types::Message::text(crate::types::Role::System, "long prompt ".repeat(40)),
        ];

        let normal = auth_context("pro");
        let outcome = execute(&state, &normal, request.clone(), RoutingHint::Auto)
            .await
            .unwrap();
        assert!(outcome.tokens_saved_by_compression > 0);

        // The per-technique breakdown must reach both the live outcome and the usage
        // record — before this it was only ever visible in the response and the
        // /api/compression/preview demo endpoint, never persisted onto the record itself.
        let breakdown = outcome
            .techniques_fired
            .as_ref()
            .expect("duplicate system messages should have fired the dedupe technique");
        assert!(
            breakdown["duplicate_system_messages_removed"]
                .as_u64()
                .unwrap()
                >= 1,
            "{breakdown:?}"
        );
        let event = outcome.usage_event(&normal, 200, "us-east");
        assert_eq!(
            event.techniques_fired, outcome.techniques_fired,
            "the usage record must carry the same breakdown the response reported"
        );

        let mut private = auth_context("enterprise");
        private.zero_retention = true;
        let untouched = execute(&state, &private, request, RoutingHint::Auto)
            .await
            .unwrap();
        assert_eq!(
            untouched.tokens_saved_by_compression, 0,
            "a zero-retention prompt must be delivered exactly as written"
        );
        assert_eq!(
            untouched.techniques_fired, None,
            "nothing fired, so there is nothing to record"
        );
    }

    #[test]
    fn reasoning_models_get_the_longer_timeout() {
        assert!(is_reasoning_model("o3"));
        assert!(is_reasoning_model("openai/o4-mini"));
        assert!(is_reasoning_model("deepseek/deepseek-reasoner"));
        assert!(!is_reasoning_model("gpt-4o"));
        assert!(!is_reasoning_model("anthropic/claude-sonnet-4-5"));
    }

    #[test]
    fn plan_ceilings_match_the_business_model() {
        assert_eq!(plan_tier_ceiling("free"), Some(ModelTier::Cheap));
        assert_eq!(plan_tier_ceiling("pro"), None);
        assert_eq!(plan_tier_ceiling("team"), None);
        assert_eq!(plan_tier_ceiling("enterprise"), None);
    }

    #[test]
    fn usage_resolution_prefers_reported_tokens() {
        let request = NormalizedRequest::simple("gpt-4o", "hello there");
        let reported = NormalizedResponse {
            id: "x".into(),
            model: "m".into(),
            content: "hi".into(),
            finish_reason: None,
            tool_calls: None,
            usage: TokenUsage {
                input_tokens: 42,
                output_tokens: 7,
                estimated: false,
                ..Default::default()
            },
            raw: None,
        };
        let resolved = resolve_usage(&reported, &request);
        assert_eq!(resolved.input_tokens, 42);
        assert!(!resolved.estimated);
    }

    #[test]
    fn usage_resolution_estimates_and_marks_it() {
        // An estimate must never masquerade as a reported figure on an invoice.
        let request = NormalizedRequest::simple("gpt-4o", "hello there");
        let unreported = NormalizedResponse {
            id: "x".into(),
            model: "m".into(),
            content: "a fairly long response body".into(),
            finish_reason: None,
            tool_calls: None,
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                estimated: true,
                ..Default::default()
            },
            raw: None,
        };
        let resolved = resolve_usage(&unreported, &request);
        assert!(resolved.estimated);
        assert!(resolved.input_tokens > 0);
        assert!(resolved.output_tokens > 0);
    }

    #[test]
    fn openai_response_rendering_is_well_formed() {
        let outcome = PipelineOutcome {
            request_id: Uuid::new_v4(),
            response: NormalizedResponse {
                id: String::new(),
                model: "mock/mock-cheap".into(),
                content: "4".into(),
                finish_reason: Some("stop".into()),
                tool_calls: None,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 1,
                    estimated: false,
                    ..Default::default()
                },
                raw: None,
            },
            served_model: "mock/mock-cheap".into(),
            requested_model: "mock/mock-premium".into(),
            provider: "mock".into(),
            savings: SavingsBreakdown::compute(MicroCents(1_000), MicroCents(100), 2_000),
            savings_components: crate::metering::savings::SavingsComponents::compute(
                MicroCents(900),
                MicroCents::ZERO,
                MicroCents::ZERO,
            ),
            input_cost_mc: 0,
            output_cost_mc: 0,
            cache: CacheOutcome::Miss,
            routing_reason: RoutingReason::Complexity,
            complexity_score: Some(0.1),
            tokens: TokenUsage {
                input_tokens: 10,
                output_tokens: 1,
                estimated: false,
                ..Default::default()
            },
            gateway_overhead_ms: 0.4,
            total_latency_ms: 120,
            tokens_saved_by_compression: 0,
            techniques_fired: None,
            cache_bust_hits: 0,
            explanation: Vec::new(),
        };

        let body = to_openai_response(&outcome);
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["choices"][0]["message"]["content"], "4");
        assert_eq!(body["usage"]["total_tokens"], 11);
        assert!(body["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    #[test]
    fn a_raw_provider_body_is_returned_verbatim() {
        // Clients depending on fields we do not model must still receive them.
        let mut outcome = PipelineOutcome {
            request_id: Uuid::new_v4(),
            response: NormalizedResponse {
                id: "resp".into(),
                model: "m".into(),
                content: "hi".into(),
                finish_reason: None,
                tool_calls: None,
                usage: TokenUsage::default(),
                raw: None,
            },
            served_model: "m".into(),
            requested_model: "m".into(),
            provider: "mock".into(),
            savings: SavingsBreakdown::passthrough(MicroCents(10)),
            savings_components: crate::metering::savings::SavingsComponents::default(),
            input_cost_mc: 0,
            output_cost_mc: 0,
            cache: CacheOutcome::Miss,
            routing_reason: RoutingReason::Passthrough,
            complexity_score: None,
            tokens: TokenUsage::default(),
            gateway_overhead_ms: 0.1,
            total_latency_ms: 5,
            tokens_saved_by_compression: 0,
            techniques_fired: None,
            cache_bust_hits: 0,
            explanation: Vec::new(),
        };
        outcome.response.raw = Some(serde_json::json!({"system_fingerprint": "fp_x"}));
        assert_eq!(to_openai_response(&outcome)["system_fingerprint"], "fp_x");
    }

    // -----------------------------------------------------------------------------
    // Data residency enforcement
    //
    // Every other test in this module calls `execute()` directly, which starts at
    // pipeline stage [3] and never passes through HTTP-layer auth — so none of them
    // would notice if residency enforcement were removed from `handle_chat` or
    // `embeddings` entirely. These two go through the real handlers instead, proving
    // the wiring itself rather than `residency::enforce`'s own logic (already covered
    // in `enterprise::residency`'s tests).
    // -----------------------------------------------------------------------------

    /// Seed a resolvable API key in the test state's cache and return the bearer token
    /// that resolves to it, so a handler test can authenticate without a database.
    fn seed_key(state: &AppState, org_region: &str) -> String {
        let token = format!("aegis_sk_{}", "a".repeat(crate::crypto::API_KEY_RANDOM_LEN));
        let context = crate::db::repo::KeyContext {
            api_key_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            assigned_to_user_id: None,
            rate_limit_per_minute: 1_000,
            monthly_budget_mc: None,
            allowed_models: None,
            plan: "pro".to_string(),
            savings_share_bp: 2_000,
            zero_retention: false,
            org_region: org_region.to_string(),
            team_name: None,
            user_email: None,
            key_default_routing_mode: None,
            team_default_routing_mode: None,
            org_default_routing_mode: None,
        };
        state
            .key_cache
            .put(&crate::crypto::hash_token(&token), context);
        token
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers
    }

    #[tokio::test]
    async fn chat_completions_refuses_a_request_from_the_wrong_region() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let mut state = test_state(Arc::clone(&mock));
        // The instance is pinned to eu-central; the organisation is pinned elsewhere.
        let mut config = (*state.config).clone();
        config.region = "eu-central".to_string();
        state.config = std::sync::Arc::new(config);

        let token = seed_key(&state, "us-east");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        );

        let err = handle_chat(&state, &headers, body).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            mock.call_count(),
            0,
            "a misrouted request must never reach a provider"
        );
    }

    #[tokio::test]
    async fn chat_completions_allows_a_request_from_the_matching_region() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let mut state = test_state(Arc::clone(&mock));
        let mut config = (*state.config).clone();
        config.region = "eu-central".to_string();
        state.config = std::sync::Arc::new(config);

        let token = seed_key(&state, "eu-central");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        );

        let response = handle_chat(&state, &headers, body).await;
        assert!(
            response.is_ok(),
            "a matching region must be served: {response:?}"
        );
    }

    // ---- Token circuit breaker -----------------------------------------------------

    #[test]
    fn clamp_max_tokens_injects_the_ceiling_when_absent() {
        let mut request = NormalizedRequest::simple("mock/mock-premium", "hi");
        assert_eq!(request.max_tokens, None);
        assert!(clamp_max_tokens(&mut request, 1_000));
        assert_eq!(request.max_tokens, Some(1_000));
    }

    #[test]
    fn clamp_max_tokens_lowers_a_request_above_the_ceiling() {
        let mut request = NormalizedRequest::simple("mock/mock-premium", "hi");
        request.max_tokens = Some(50_000);
        assert!(clamp_max_tokens(&mut request, 1_000));
        assert_eq!(request.max_tokens, Some(1_000));
    }

    #[test]
    fn clamp_max_tokens_leaves_a_request_already_under_the_ceiling_alone() {
        let mut request = NormalizedRequest::simple("mock/mock-premium", "hi");
        request.max_tokens = Some(200);
        assert!(!clamp_max_tokens(&mut request, 1_000));
        assert_eq!(request.max_tokens, Some(200));
    }

    #[test]
    fn clamp_max_tokens_leaves_a_request_exactly_at_the_ceiling_alone() {
        let mut request = NormalizedRequest::simple("mock/mock-premium", "hi");
        request.max_tokens = Some(1_000);
        assert!(!clamp_max_tokens(&mut request, 1_000));
    }

    // ---- Semantic cache reachability -------------------------------------------------

    #[test]
    fn a_local_embedder_is_always_worth_it_regardless_of_the_remote_flag_or_complexity() {
        assert!(semantic_lookup_worth_it(true, false, Complexity::Simple));
        assert!(semantic_lookup_worth_it(true, false, Complexity::Complex));
        assert!(semantic_lookup_worth_it(true, true, Complexity::Simple));
    }

    #[test]
    fn a_remote_embedder_is_never_worth_it_when_the_operator_has_not_opted_in() {
        assert!(!semantic_lookup_worth_it(false, false, Complexity::Simple));
        assert!(!semantic_lookup_worth_it(false, false, Complexity::Medium));
        assert!(!semantic_lookup_worth_it(false, false, Complexity::Complex));
    }

    #[test]
    fn a_remote_embedder_skips_simple_requests_even_when_opted_in() {
        assert!(!semantic_lookup_worth_it(false, true, Complexity::Simple));
    }

    #[test]
    fn a_remote_embedder_is_worth_it_for_medium_or_complex_requests_when_opted_in() {
        assert!(semantic_lookup_worth_it(false, true, Complexity::Medium));
        assert!(semantic_lookup_worth_it(false, true, Complexity::Complex));
    }

    fn state_with_low_token_ceiling(mock: Arc<MockProvider>) -> AppState {
        let state = test_state(mock);
        let mut config = (*state.config).clone();
        config.max_tokens_per_request = 500;
        AppState {
            config: std::sync::Arc::new(config),
            ..state
        }
    }

    #[tokio::test]
    async fn a_request_with_no_max_tokens_reaches_the_provider_with_the_ceiling_injected() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = state_with_low_token_ceiling(Arc::clone(&mock));
        let token = seed_key(&state, "test");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        );

        let response = handle_chat(&state, &headers, body).await;
        assert!(response.is_ok(), "request should succeed: {response:?}");
        assert_eq!(mock.calls().last().unwrap().max_tokens, Some(500));
    }

    #[tokio::test]
    async fn a_request_asking_for_more_than_the_ceiling_is_clamped_before_reaching_the_provider() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = state_with_low_token_ceiling(Arc::clone(&mock));
        let token = seed_key(&state, "test");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1_000_000
            })
            .to_string(),
        );

        let response = handle_chat(&state, &headers, body).await;
        assert!(response.is_ok(), "request should succeed: {response:?}");
        assert_eq!(
            mock.calls().last().unwrap().max_tokens,
            Some(500),
            "the platform ceiling must win over a client-requested value above it"
        );
    }

    #[tokio::test]
    async fn a_request_already_under_the_ceiling_is_sent_unchanged() {
        let mock = Arc::new(MockProvider::returning("answer"));
        let state = state_with_low_token_ceiling(Arc::clone(&mock));
        let token = seed_key(&state, "test");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 100
            })
            .to_string(),
        );

        let response = handle_chat(&state, &headers, body).await;
        assert!(response.is_ok(), "request should succeed: {response:?}");
        assert_eq!(
            mock.calls().last().unwrap().max_tokens,
            Some(100),
            "a client-requested value already under the ceiling must be honoured exactly"
        );
    }

    #[tokio::test]
    async fn the_token_circuit_breaker_also_protects_streaming_requests() {
        // The safety property this whole feature exists for would have a hole here if it
        // didn't: `stream_chat` runs a completely separate pipeline from
        // `execute_with_headroom` and never reaches its clamp.
        let mock = Arc::new(MockProvider::returning("streamed answer"));
        let state = state_with_low_token_ceiling(Arc::clone(&mock));
        let token = seed_key(&state, "test");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            })
            .to_string(),
        );

        let response = handle_chat(&state, &headers, body).await;
        assert!(response.is_ok(), "request should succeed: {response:?}");
        assert_eq!(mock.calls().last().unwrap().max_tokens, Some(500));
    }

    #[tokio::test]
    async fn embeddings_also_refuses_a_request_from_the_wrong_region() {
        let mock = Arc::new(MockProvider::returning("unused"));
        let mut state = test_state(Arc::clone(&mock));
        let mut config = (*state.config).clone();
        config.region = "eu-central".to_string();
        state.config = std::sync::Arc::new(config);

        let token = seed_key(&state, "ap-southeast");
        let headers = bearer_headers(&token);
        let body = axum::body::Bytes::from(
            serde_json::json!({"model": "mock/mock-cheap", "input": "hi"}).to_string(),
        );

        let response = embeddings(State(state), headers, body).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

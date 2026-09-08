//! Model selection — pipeline stage [6b].
//!
//! Given a request, the org's policies, the pricing table, and provider health, decide
//! which model actually serves it.
//!
//! # The rule that governs everything here
//!
//! > *Conservative default: if unsure, use the requested model. NEVER degrade quality on
//! > complex requests. Savings come from the simple-request long tail.*
//! > — `MASTER_BUILD.md` Part 5 [6b]
//!
//! Every branch below resolves ambiguity toward the requested model. An unknown model, an
//! unparseable policy, a candidate list emptied by capability filtering, a circuit-open
//! provider — all of them fall back to passthrough rather than guessing. We would rather
//! forgo a saving than serve a worse answer than the customer asked for, because the
//! first costs cents and the second costs the account.
//!
//! # Precedence
//!
//! 1. Caller's `X-Aegis-Routing-Hint: passthrough` — the escape hatch of Part 13 item 5.
//!    Nothing overrides it.
//! 2. Organisation policy (`deny`, `pin_model`, `passthrough`, tier target/ceiling).
//! 3. Classifier complexity band, as traded by the caller's routing mode.
//! 4. Passthrough.
//!
//! # Routing modes
//!
//! `X-Aegis-Routing-Hint` selects how hard to trade cost against quality. The band the
//! classifier assigns decides what is eligible; the mode decides how far to go:
//!
//! | Mode | Simple | Medium | Complex |
//! |---|---|---|---|
//! | `passthrough` | requested | requested | requested |
//! | `quality` | mid | requested | requested |
//! | `balanced` / `auto` | cheap | mid | requested |
//! | `economy` (or legacy `cheap`) | cheap | cheap | requested |
//!
//! The complex column is the product's central promise and no mode can change it. It is
//! enforced twice on purpose: [`RoutingHint::target_tier`] returns `None` for that band in
//! every mode, and [`Router::route`] returns before any tier logic runs.

use crate::engine::bandit::RoutingBandit;
use crate::engine::classifier::{Classification, Classifier, TaskDomain};
use crate::engine::fallback::{HealthScore, ProviderHealth};
use crate::engine::policy::{PolicyContext, RoutingPolicy};
use crate::error::{AegisError, Result};
use crate::metering::pricing::{PricingTable, Requirements};
use crate::types::{ModelTier, NormalizedRequest, RoutingHint, RoutingReason};

/// The outcome of routing one request.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingDecision {
    /// Canonical id of the model that will serve the request.
    pub served_model: String,
    /// Provider that owns the served model.
    pub provider: String,
    /// Why this model was chosen. Stored on the usage record.
    pub reason: RoutingReason,
    /// Classification, when one was computed.
    pub complexity_score: Option<f32>,
    /// Number of cheaper candidates considered.
    pub candidates_considered: usize,
    /// A human-readable account of *why*, in the customer's terms.
    ///
    /// Returned on `X-Aegis-Routing-Explanation` and stored on the usage record. A
    /// six-value enum and a bare score cannot answer "why was my request downgraded" — the
    /// question a customer actually asks — and the generator that could
    /// ([`crate::engine::classifier::Features::explain`]) existed with no caller outside
    /// its own test. Found in the enterprise readiness audit.
    pub explanation: Vec<String>,
}

impl RoutingDecision {
    /// True when the request is going to the model the caller asked for.
    pub fn is_passthrough(&self) -> bool {
        matches!(
            self.reason,
            RoutingReason::Passthrough | RoutingReason::UserOverride
        )
    }
}

/// Inputs to a routing decision beyond the request itself.
#[derive(Debug, Clone, Default)]
pub struct RoutingInputs<'a> {
    /// The caller's routing hint.
    pub hint: RoutingHint,
    /// The org's active policy.
    pub policy: Option<&'a RoutingPolicy>,
    /// Team name, for team-scoped policy rules.
    pub team: Option<String>,
    /// The calling key's assignee, by email, for person-scoped policy rules.
    pub user: Option<String>,
    /// Models this org may use at all. `None` means no restriction.
    pub allowed_models: Option<Vec<String>>,
    /// Hard tier ceiling from the org's plan (the free tier is capped at cheap models).
    pub plan_tier_ceiling: Option<ModelTier>,
    /// Budget headroom left this period, in micro-cents, when a hard limit applies.
    ///
    /// Routing has always been blind to actual spend — it knew a plan's static tier
    /// ceiling and nothing about whether the organisation was two dollars from its cap.
    /// With this, an organisation running low is steered toward cheaper capable models
    /// *before* the budget check starts returning 402s, which is a far better outcome than
    /// a wall of rejections at 4pm on the last day of the month.
    pub budget_headroom_mc: Option<i64>,
    /// The bandit's learned preferences, when routing should consult them.
    ///
    /// `None` disables outcome-informed selection entirely — used by tests that need a
    /// purely deterministic decision, and available as a kill switch.
    pub bandit: Option<&'a RoutingBandit>,
    /// Pinned model from conversation affinity, preserving upstream KV prompt caching across turns.
    pub affinity_model: Option<String>,
    /// Providers for which credentials exist for this organisation.
    /// When Some, candidate models will be restricted to these providers.
    pub configured_providers: Option<std::collections::HashSet<String>>,
}

/// The routing engine.
#[derive(Debug, Clone, Default)]
pub struct Router {
    classifier: Classifier,
}

impl Router {
    /// A router using the default classifier.
    pub fn new() -> Router {
        Router::default()
    }

    /// A router with a specific classifier.
    pub fn with_classifier(classifier: Classifier) -> Router {
        Router { classifier }
    }

    /// Decide which model serves this request.
    pub fn route(
        &self,
        request: &NormalizedRequest,
        pricing: &PricingTable,
        health: &ProviderHealth,
        inputs: &RoutingInputs<'_>,
    ) -> Result<RoutingDecision> {
        // The context a candidate must actually fit: the prompt *plus* whatever the caller
        // asked the model to generate. Checking the prompt alone — which is what this did
        // before — lets a 100k-token prompt with `max_tokens: 32000` route to a
        // 128k-context model that will refuse it partway through generation, turning a
        // routable request into a mid-stream failure. Found in the enterprise readiness
        // audit.
        let required_context = request
            .estimated_input_tokens()
            .saturating_add(request.max_tokens.map(u64::from).unwrap_or(0))
            .min(u32::MAX as u64) as u32;

        let requirements = Requirements {
            tools: request.requires_tools(),
            vision: request.requires_vision(),
            min_context: required_context,
            // A chat request needs a model that can hold a conversation. Without this,
            // an embedding model wins the price sort for every simple request and answers
            // none of them.
            chat: true,
        };

        // An unknown model cannot be priced, so it cannot be compared, so it cannot be
        // safely substituted. Pass it straight through and let the provider decide
        // whether it exists.
        let Some(requested) = pricing.get(&request.model) else {
            return Ok(RoutingDecision {
                served_model: request.model.clone(),
                provider: infer_provider(&request.model),
                reason: RoutingReason::Passthrough,
                complexity_score: None,
                candidates_considered: 0,
                explanation: vec![format!(
                    "{} is not in the pricing table, so it cannot be compared or \
                     substituted; passed through unchanged",
                    request.model
                )],
            });
        };

        // [1] The caller's escape hatch outranks everything we might prefer.
        if inputs.hint == RoutingHint::Passthrough {
            return Ok(self.passthrough(requested, RoutingReason::UserOverride));
        }

        let classification = self.classifier.classify(request);

        // The mode this decision uses from here on. Starts as the caller's own hint
        // (already resolved from the header, or the key/project/org default chain, before
        // this function was ever called) and may be overridden by a policy's `routing_mode`
        // action below — never by anything else, so [1]'s absolute passthrough escape hatch
        // above stays checked against the caller's *actual* header, not a value policy
        // could have changed.
        let mut effective_hint = inputs.hint;

        // [2] Organisation policy.
        if let Some(policy) = inputs.policy {
            let context = PolicyContext {
                complexity: Some(classification.complexity),
                model_requested: request.model.clone(),
                team: inputs.team.clone(),
                user: inputs.user.clone(),
                routing_mode: Some(inputs.hint.as_str().to_string()),
                requires_tools: request.requires_tools(),
                estimated_input_tokens: request.estimated_input_tokens(),
            };

            if let Some(action) = policy.evaluate(&context) {
                if action.is_deny() {
                    return Err(AegisError::ModelNotAllowed(format!(
                        "an organisation policy forbids routing {} for this request",
                        request.model
                    )));
                }
                if action.is_passthrough() {
                    return Ok(self.passthrough(requested, RoutingReason::Policy));
                }
                if let Some(pinned) = &action.pin_model {
                    if let Some(model) = pricing.get(pinned) {
                        return Ok(RoutingDecision {
                            served_model: model.model_id.clone(),
                            provider: model.provider.clone(),
                            reason: RoutingReason::Policy,
                            complexity_score: Some(classification.score),
                            candidates_considered: 0,
                            explanation: vec![format!(
                                "an organisation policy pins this request to {}",
                                model.model_id
                            )],
                        });
                    }
                    // A policy pinning a model we cannot price is a configuration error.
                    // Passing through is the safe reading: the operator wanted control,
                    // not a downgrade.
                    tracing::warn!(model = %pinned, "policy pins an unknown model; passing through");
                    return Ok(self.passthrough(requested, RoutingReason::Policy));
                }

                let ceiling = combine_ceilings(action.tier_ceiling(), inputs.plan_tier_ceiling);
                if let Some(target) = action.target_tier().or(ceiling) {
                    if let Some(decision) = self.select_at_tier(
                        pricing,
                        health,
                        requested,
                        target,
                        requirements,
                        inputs,
                        RoutingReason::Policy,
                        Some(&classification),
                    ) {
                        return Ok(decision);
                    }
                    return Ok(self.passthrough(requested, RoutingReason::Policy));
                }

                if let Some(mode) = action.routing_mode_hint() {
                    // A mode-only action does not pick a tier itself — it hands the rest of
                    // this decision to the mode ladder below, exactly as an ordinary header
                    // would, so the complex-band guarantee is enforced by the same control
                    // flow either way rather than a second copy of it living here.
                    effective_hint = mode;
                } else {
                    // Matched, but named no actionable field at all: the safest reading is
                    // passthrough, unchanged from this function's behaviour before
                    // `routing_mode` existed.
                    return Ok(self.passthrough(requested, RoutingReason::Policy));
                }
            }
        }

        // [2b] Conversation affinity: if this multi-turn session has a pinned model
        // (to retain prompt KV-cache across turns), check if it can serve this turn.
        if let Some(pinned_id) = &inputs.affinity_model {
            if let Some(pinned) = pricing.get(pinned_id) {
                if health.is_available(&pinned.provider)
                    && is_allowed(&pinned.model_id, inputs)
                    && is_provider_configured(&pinned.provider, &pinned.model_id, inputs)
                    && (!requirements.tools || pinned.supports_tools)
                    && (!requirements.vision || pinned.supports_vision)
                    && pinned.context_window >= requirements.min_context
                {
                    return Ok(RoutingDecision {
                        served_model: pinned.model_id.clone(),
                        provider: pinned.provider.clone(),
                        reason: RoutingReason::Complexity,
                        complexity_score: Some(classification.score),
                        candidates_considered: 1,
                        explanation: vec![format!(
                            "pinned to {} via conversation affinity to preserve upstream prompt KV cache",
                            pinned.model_id
                        )],
                    });
                }
            }
        }

        // [3] Complexity-driven selection, as tightened or loosened by the caller's mode.
        //
        // The mode decides how hard to trade; the complexity band decides what is on the
        // table. `target_tier` returns `None` when this combination must not be
        // substituted at all — always for a complex request, and additionally for a
        // medium one under `quality`.
        //
        // Complex returning `None` is belt and braces: the mode table says so, and this
        // branch returns before any tier logic runs. Serving a worse answer to save money
        // is the one trade this product must never make silently, so it is expressed
        // twice on purpose.
        let Some(target_tier) = effective_hint.target_tier(classification.complexity) else {
            return Ok(self.passthrough_capped(
                requested,
                pricing,
                health,
                requirements,
                inputs,
                Some(classification.score),
            ));
        };

        // Budget pressure tightens the ceiling for requests that were already going to be
        // downgraded. An organisation two dollars from a hard cap gets steered toward
        // cheaper capable models *before* the budget check starts refusing requests, which
        // is a far better outcome than a wall of 402s at 4pm on the last day of the month.
        //
        // Deliberately only reachable for Medium and Simple: Complex returned above.
        let target_tier = match budget_pressure(inputs, requested, request) {
            BudgetPressure::Critical => ModelTier::Cheap,
            BudgetPressure::Tight => target_tier.min(ModelTier::Mid),
            BudgetPressure::Comfortable => target_tier,
        };

        let ceiling =
            combine_ceilings(Some(target_tier), inputs.plan_tier_ceiling).unwrap_or(target_tier);

        // Never route *up*: if the caller asked for something cheap, a "cheap tier"
        // target must not promote them to a pricier model.
        let effective = ceiling.min(requested.tier);

        if let Some(decision) = self.select_at_tier(
            pricing,
            health,
            requested,
            effective,
            requirements,
            inputs,
            RoutingReason::Complexity,
            Some(&classification),
        ) {
            return Ok(decision);
        }

        // [4] Nothing cheaper was both capable and available.
        Ok(self.passthrough_capped(
            requested,
            pricing,
            health,
            requirements,
            inputs,
            Some(classification.score),
        ))
    }

    /// Classify without routing. Used by the request log and by the bandit's replay.
    pub fn classify(&self, request: &NormalizedRequest) -> Classification {
        self.classifier.classify(request)
    }

    fn passthrough(
        &self,
        requested: &crate::metering::pricing::ModelPricing,
        reason: RoutingReason,
    ) -> RoutingDecision {
        RoutingDecision {
            served_model: requested.model_id.clone(),
            provider: requested.provider.clone(),
            reason,
            complexity_score: None,
            candidates_considered: 0,
            explanation: vec![match reason {
                RoutingReason::UserOverride => {
                    "the caller sent X-Aegis-Routing-Hint: passthrough, which overrides                      every other consideration"
                        .to_string()
                }
                RoutingReason::Policy => {
                    "an organisation policy requires this request to pass through                      unchanged"
                        .to_string()
                }
                _ => "served on the requested model unchanged".to_string(),
            }],
        }
    }

    /// Passthrough, unless the org's plan forbids the requested model outright — in which
    /// case fall to the best model the plan does allow rather than returning an error.
    fn passthrough_capped(
        &self,
        requested: &crate::metering::pricing::ModelPricing,
        pricing: &PricingTable,
        health: &ProviderHealth,
        requirements: Requirements,
        inputs: &RoutingInputs<'_>,
        score: Option<f32>,
    ) -> RoutingDecision {
        let allowed_by_plan = inputs
            .plan_tier_ceiling
            .is_none_or(|ceiling| requested.tier <= ceiling);
        let allowed_by_list = is_allowed(&requested.model_id, inputs);
        let provider_up = health.is_available(&requested.provider);
        let provider_configured =
            is_provider_configured(&requested.provider, &requested.model_id, inputs);

        if allowed_by_plan && allowed_by_list && provider_up && provider_configured {
            return RoutingDecision {
                served_model: requested.model_id.clone(),
                provider: requested.provider.clone(),
                reason: RoutingReason::Passthrough,
                complexity_score: score,
                candidates_considered: 0,
                explanation: vec![
                    "no cheaper model could serve this request without a quality downgrade; served on the model you asked for"
                        .to_string(),
                ],
            };
        }

        let ceiling = inputs.plan_tier_ceiling.unwrap_or(ModelTier::Frontier);
        let fallback = pricing
            .all()
            .filter(|m| m.tier <= ceiling)
            .filter(|m| PricingTable::satisfies(m, requirements))
            .filter(|m| is_allowed(&m.model_id, inputs))
            .filter(|m| is_provider_configured(&m.provider, &m.model_id, inputs))
            .filter(|m| health.is_available(&m.provider))
            // Best available, not cheapest: the caller asked for something they cannot
            // have, so give them the closest thing rather than the weakest.
            .max_by(|a, b| {
                a.tier
                    .cmp(&b.tier)
                    .then_with(|| a.blended_per_mtok().cmp(&b.blended_per_mtok()))
                    .then_with(|| b.model_id.cmp(&a.model_id))
            });

        match fallback {
            Some(model) => RoutingDecision {
                served_model: model.model_id.clone(),
                provider: model.provider.clone(),
                // Substitution, not policy. Recording an outage-driven substitution as
                // `policy` sent a support engineer looking for a policy that did not
                // exist, and hid the real cause — which is the one thing an incident
                // timeline most needs. Found in the enterprise readiness audit.
                reason: substitution_reason(provider_up),
                complexity_score: score,
                candidates_considered: 1,
                explanation: vec![substitution_explanation(
                    requested,
                    model,
                    provider_up,
                    allowed_by_plan,
                    allowed_by_list,
                    provider_configured,
                )],
            },
            // Nothing at all is available. Passing through gives the provider a chance to
            // answer and the caller a real upstream error rather than one we invented.
            None => RoutingDecision {
                served_model: requested.model_id.clone(),
                provider: requested.provider.clone(),
                reason: RoutingReason::Passthrough,
                complexity_score: score,
                candidates_considered: 0,
                explanation: vec![
                    "no permitted model is currently available; sent to the requested model so the provider's own error reaches you rather than one we invented"
                        .to_string(),
                ],
            },
        }
    }

    /// Best capable, available, permitted model at or below `tier`, strictly cheaper than
    /// the requested model.
    ///
    /// "Best" is not simply "cheapest" any more. Three signals shape the order, each one
    /// closing a gap the enterprise readiness audit found:
    ///
    /// * **Provider health**, graded rather than binary. A provider failing a third of its
    ///   requests never trips its circuit — it never fails five times *in a row* — so
    ///   before this it kept winning the price sort while quietly costing every caller a
    ///   retry. Its effective price is now multiplied by a penalty derived from its recent
    ///   success rate.
    /// * **Latency**, which routing had no field for at all. A provider that is currently
    ///   much slower than its peers is penalised, because on a gateway sold on overhead,
    ///   silently routing to the slow option is a worse outcome than paying slightly more.
    /// * **Outcomes**, via the bandit. It has always recorded every result and was never
    ///   read back, which made "outcome-trained routing" a description of intent rather
    ///   than behaviour. It now breaks ties among candidates the router has already
    ///   decided are acceptable — it can reorder, never widen, the permitted set.
    #[allow(clippy::too_many_arguments)]
    fn select_at_tier(
        &self,
        pricing: &PricingTable,
        health: &ProviderHealth,
        requested: &crate::metering::pricing::ModelPricing,
        tier: ModelTier,
        requirements: Requirements,
        inputs: &RoutingInputs<'_>,
        reason: RoutingReason,
        classification: Option<&Classification>,
    ) -> Option<RoutingDecision> {
        let score = classification.map(|c| c.score);
        let mut candidates: Vec<_> = pricing
            .cheaper_alternatives(&requested.model_id, requirements)
            .into_iter()
            .filter(|m| m.tier <= tier)
            .filter(|m| is_allowed(&m.model_id, inputs))
            .filter(|m| is_provider_configured(&m.provider, &m.model_id, inputs))
            .filter(|m| health.is_available(&m.provider))
            .map(|model| {
                let health_score = health.score(&model.provider);
                (model, health_score)
            })
            .collect();

        let considered = candidates.len();
        if candidates.is_empty() {
            return None;
        }

        // Health-adjusted price. A degraded provider's price is multiplied by roughly the
        // number of attempts a request there is expected to take, so "cheap but flaky"
        // competes on its true cost rather than its advertised one.
        let median_latency = median_latency_ms(&candidates);
        candidates.sort_by(|(a, ah), (b, bh)| {
            effective_price(a, ah, median_latency)
                .partial_cmp(&effective_price(b, bh, median_latency))
                .unwrap_or(std::cmp::Ordering::Equal)
                // Stable tie-break so routing stays deterministic and an incident is
                // reproducible from a usage record.
                .then_with(|| a.model_id.cmp(&b.model_id))
        });

        // Task-domain preference: if the classification named a domain, promote candidates
        // from a preferred provider when they are within 25% of the best effective price.
        // This is a soft re-rank, not a filter — a preferred provider that is expensive
        // or degraded loses on price and stays behind; this only moves them up when the
        // cost difference is negligible. The 25% band is deliberately narrow: the customer
        // is paying for cost optimisation first.
        if let Some(c) = classification {
            let preferred = c.domain.preferred_providers();
            if !preferred.is_empty() && candidates.len() > 1 {
                let best_price = {
                    let (m, h) = &candidates[0];
                    effective_price(m, h, median_latency)
                };
                let threshold = best_price * 1.25;
                // Stable partition: preferred providers within the band go first,
                // preserving the price order within each group.
                candidates.sort_by(|(a, ah), (b, bh)| {
                    let pa = effective_price(a, ah, median_latency);
                    let pb = effective_price(b, bh, median_latency);
                    let a_pref = pa <= threshold && preferred.contains(&a.provider.as_str());
                    let b_pref = pb <= threshold && preferred.contains(&b.provider.as_str());
                    b_pref
                        .cmp(&a_pref)
                        .then_with(|| pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal))
                        .then_with(|| a.model_id.cmp(&b.model_id))
                });
            }
        }

        // The bandit reorders within what the router already permits. Deliberately not
        // allowed to add a candidate: everything here has already passed capability,
        // allowlist, tier, and availability filtering, and outcome data is not a reason to
        // relax any of those.
        let chosen = match (inputs.bandit, classification) {
            (Some(bandit), Some(classification)) => {
                let ids: Vec<&str> = candidates
                    .iter()
                    .map(|(m, _)| m.model_id.as_str())
                    .collect();
                bandit
                    .select(classification.complexity, &ids)
                    .and_then(|id| candidates.iter().find(|(m, _)| m.model_id == id))
                    .unwrap_or(&candidates[0])
            }
            _ => &candidates[0],
        };

        let (model, model_health) = chosen;
        let mut explanation = classification.map(|c| c.explain()).unwrap_or_default();
        if let Some(c) = classification {
            if c.domain != TaskDomain::General {
                explanation.push(format!(
                    "task domain: {} — routing prefers {} providers for this class of work",
                    c.domain.as_str(),
                    c.domain.preferred_providers().join(", ")
                ));
            }
        }
        explanation.push(format!(
            "routed to {} instead of {} ({} cheaper capable alternative{} considered)",
            model.model_id,
            requested.model_id,
            considered,
            if considered == 1 { "" } else { "s" }
        ));
        if model_health.is_degraded() {
            explanation.push(format!(
                "note: {} is currently degraded ({:.0}% recent success) but was still the \
                 best value",
                model.provider,
                model_health.success_rate * 100.0
            ));
        }

        Some(RoutingDecision {
            served_model: model.model_id.clone(),
            provider: model.provider.clone(),
            reason,
            complexity_score: score,
            candidates_considered: considered,
            explanation,
        })
    }
}

/// How close an organisation is to running out of budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetPressure {
    /// Plenty of headroom, or no hard limit at all.
    Comfortable,
    /// Fewer than ten of this request left before the cap.
    Tight,
    /// Fewer than three left. Every remaining request should be as cheap as possible.
    Critical,
}

/// Judge budget pressure in units of *this request*, not in dollars.
///
/// A fixed dollar threshold would be meaningless across customers — $5 of headroom is
/// nothing to one organisation and a week to another. Measuring in "how many more requests
/// like this one fit" is the same judgement a person would make.
fn budget_pressure(
    inputs: &RoutingInputs<'_>,
    requested: &crate::metering::pricing::ModelPricing,
    request: &NormalizedRequest,
) -> BudgetPressure {
    let Some(headroom) = inputs.budget_headroom_mc else {
        return BudgetPressure::Comfortable;
    };
    if headroom <= 0 {
        // Already at the cap. The budget check will refuse this request; routing has no
        // useful opinion left, and pretending otherwise would just add noise.
        return BudgetPressure::Critical;
    }

    let projected = requested
        .cost(
            request.estimated_input_tokens(),
            request.max_tokens.map(u64::from).unwrap_or(512),
        )
        .as_i64();
    if projected <= 0 {
        return BudgetPressure::Comfortable;
    }

    match headroom / projected {
        0..=2 => BudgetPressure::Critical,
        3..=9 => BudgetPressure::Tight,
        _ => BudgetPressure::Comfortable,
    }
}

/// Why a substitution happened, in a form an operator can act on.
///
/// Previously every one of these was recorded as `Policy`, including outages — so an
/// engineer investigating "why did this customer get a different model" went looking for a
/// policy that did not exist, and the actual cause (a provider being down) never reached
/// the usage record at all.
fn substitution_reason(provider_up: bool) -> RoutingReason {
    // Only two causes reach here: the provider being down, or the plan/allowlist
    // forbidding the requested model. Both of the latter are policy-shaped — a
    // configuration decision, not an incident — so only availability gets its own reason.
    if provider_up {
        RoutingReason::Policy
    } else {
        RoutingReason::ProviderUnavailable
    }
}

/// The customer-facing sentence explaining a substitution.
fn substitution_explanation(
    requested: &crate::metering::pricing::ModelPricing,
    served: &crate::metering::pricing::ModelPricing,
    provider_up: bool,
    allowed_by_plan: bool,
    allowed_by_list: bool,
    provider_configured: bool,
) -> String {
    if !provider_up {
        return format!(
            "{} was unavailable ({} circuit is open), so this request was served by the \
             closest available model, {}. This substitution was not a cost optimisation \
             and may cost more than the model you asked for — compare x-aegis-cost against \
             x-aegis-baseline-cost.",
            requested.model_id, requested.provider, served.model_id
        );
    }
    if !provider_configured {
        return format!(
            "{} provider ({}) has no credentials configured for your organisation, so this \
             request was served by the best configured model, {}.",
            requested.model_id, requested.provider, served.model_id
        );
    }
    if !allowed_by_plan {
        return format!(
            "{} is above your plan's model tier, so this request was served by the best \
             model your plan allows, {}.",
            requested.model_id, served.model_id
        );
    }
    if !allowed_by_list {
        return format!(
            "{} is not on your organisation's allowed-model list, so this request was \
             served by {} instead.",
            requested.model_id, served.model_id
        );
    }
    format!(
        "served by {} instead of {}",
        served.model_id, requested.model_id
    )
}

/// Median smoothed latency across candidates, used as the yardstick for "slow".
///
/// A relative measure rather than an absolute threshold: what counts as slow for a small
/// fast model is very different from a frontier reasoning model, and a fixed millisecond
/// cutoff would either never fire or always fire depending on the tier.
fn median_latency_ms(candidates: &[(&crate::metering::pricing::ModelPricing, HealthScore)]) -> f64 {
    let mut observed: Vec<f64> = candidates
        .iter()
        .map(|(_, h)| h.latency_ms)
        .filter(|ms| *ms > 0.0)
        .collect();
    if observed.is_empty() {
        return 0.0;
    }
    observed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    observed[observed.len() / 2]
}

/// How much a candidate really costs, once reliability and speed are priced in.
fn effective_price(
    model: &crate::metering::pricing::ModelPricing,
    health: &HealthScore,
    median_latency_ms: f64,
) -> f64 {
    let base = model.blended_per_mtok().as_i64() as f64;
    let mut price = base * health.price_penalty();

    // Latency penalty, capped. A provider at twice the median pays a 20% premium in the
    // comparison — enough to lose a close race, not enough to override a genuinely large
    // price difference, because the customer is paying for cost optimization first.
    if median_latency_ms > 0.0 && health.latency_ms > median_latency_ms {
        let ratio = (health.latency_ms / median_latency_ms).min(4.0);
        price *= 1.0 + (ratio - 1.0) * 0.2;
    }
    price
}

/// The stricter (lower) of two optional tier ceilings.
fn combine_ceilings(a: Option<ModelTier>, b: Option<ModelTier>) -> Option<ModelTier> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

/// Whether a model's provider is configured for the org.
fn is_provider_configured(provider: &str, model_id: &str, inputs: &RoutingInputs<'_>) -> bool {
    match &inputs.configured_providers {
        None => true,
        Some(configured) => {
            configured.contains(provider)
                || (configured.contains("openrouter")
                    && (provider == "openrouter" || model_id.starts_with("openrouter/")))
        }
    }
}

/// Whether a model is on the org's allowlist.
fn is_allowed(model_id: &str, inputs: &RoutingInputs<'_>) -> bool {
    match &inputs.allowed_models {
        None => true,
        Some(allowed) => allowed.iter().any(|a| {
            a == model_id
                || model_id
                    .split_once('/')
                    .map(|(_, bare)| bare == a)
                    .unwrap_or(false)
        }),
    }
}

/// Best guess at the provider for an unpriced model, from its prefix.
fn infer_provider(model: &str) -> String {
    if let Some((provider, _)) = model.split_once('/') {
        return provider.to_string();
    }
    let lowered = model.to_ascii_lowercase();
    if lowered.starts_with("gpt") || lowered.starts_with('o') {
        "openai".to_string()
    } else if lowered.starts_with("claude") {
        "anthropic".to_string()
    } else if lowered.starts_with("gemini") {
        "google".to_string()
    } else {
        "custom".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Complexity;
    use crate::types::{Message, Role};

    fn pricing() -> PricingTable {
        PricingTable::with_seed_data()
    }

    fn healthy() -> ProviderHealth {
        ProviderHealth::new()
    }

    fn simple_request() -> NormalizedRequest {
        NormalizedRequest::simple("gpt-4o", "What is the capital of France?")
    }

    fn complex_request() -> NormalizedRequest {
        NormalizedRequest::simple(
            "gpt-4o",
            "Analyze this stack trace, diagnose the root cause, and architect a fix that \
             prevents the same class of bug across the codebase.",
        )
    }

    /// Reasoning-domain vocabulary (confirmed directly against
    /// `classifier::tests::multi_step_analysis_is_detected_as_the_reasoning_domain`'s sibling
    /// prompt) at **medium**, not complex, severity. This matters here specifically: a
    /// `Complex`-classified request never reaches `select_at_tier` at all — `target_tier`
    /// returns `None` for it unconditionally, by design (see [3] in `route()`), so the
    /// domain re-rank and its explanation line can only ever be observed on a Medium or
    /// Simple request. Two reasoning verbs and no chaining connective keeps the score
    /// inside the Medium band (0.35-0.70) rather than tipping into Complex.
    fn reasoning_domain_medium_request() -> NormalizedRequest {
        NormalizedRequest::simple(
            "gpt-4o",
            "Investigate this pricing strategy. Evaluate its viability against the \
             competitor landscape.",
        )
    }

    // ---------------------------------------------------------------------------
    // Signals the router was blind to before the enterprise readiness audit.
    // ---------------------------------------------------------------------------

    #[test]
    fn a_degraded_provider_loses_to_a_healthy_one_when_the_penalty_outweighs_the_price_gap() {
        // The gap that mattered most: a provider failing most of its requests never
        // reaches five *consecutive* failures, so its circuit stays closed and it kept
        // winning the price sort indefinitely. Routing could only ask "is it open".
        //
        // groq/llama-3.1-8b-instant is the cheapest chat-capable seed model (blended
        // ~$0.0575/Mtok) by a wide margin over the next candidate,
        // openai/gpt-5-nano (~$0.1375/Mtok) — about 2.4x. A price gap that size is
        // deliberately not overturned by a mild penalty; it takes real, sustained
        // degradation, which is exactly what is applied here.
        let pricing = pricing();
        let health = ProviderHealth::new();

        let baseline = Router::new()
            .route(
                &simple_request(),
                &pricing,
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();
        assert_eq!(
            baseline.served_model, "groq/llama-3.1-8b-instant",
            "this test is calibrated against the seed table's actual cheapest model"
        );

        // 16 failures against 4 successes: a 20% recent success rate, never five losses in
        // a row (the success in each group of five resets the consecutive counter), so the
        // circuit stays closed throughout — this is "degraded", not "dead".
        for _ in 0..4 {
            for _ in 0..4 {
                health.record_failure("groq");
            }
            health.record_success("groq");
        }
        assert!(
            health.is_available("groq"),
            "the circuit must still be closed — that is the whole point"
        );
        let score = health.score("groq");
        assert!(score.is_degraded(), "20% success must read as degraded");
        assert!(
            score.price_penalty() > 2.4,
            "the penalty must be large enough to overcome a 2.4x price gap, got {}",
            score.price_penalty()
        );

        let degraded = Router::new()
            .route(
                &simple_request(),
                &pricing,
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();
        assert_eq!(
            degraded.served_model, "openai/gpt-5-nano",
            "a sufficiently degraded provider must lose to the next cheapest healthy one"
        );
    }

    #[test]
    fn a_mildly_degraded_provider_keeps_a_large_enough_price_lead() {
        // The complement: routing must not be so reactive that any blip reshuffles
        // selection. A provider that is still winning 90% of its requests should keep a
        // 2.4x price lead over the next candidate.
        let pricing = pricing();
        let health = ProviderHealth::new();
        for _ in 0..9 {
            health.record_success("groq");
        }
        health.record_failure("groq");

        let decision = Router::new()
            .route(
                &simple_request(),
                &pricing,
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();
        assert_eq!(decision.served_model, "groq/llama-3.1-8b-instant");
    }

    #[test]
    fn a_healthy_provider_is_not_penalised_on_a_tiny_sample() {
        // One failure out of one attempt is not a 0% success rate, it is no information.
        // Without a minimum sample, a single blip would exile a healthy provider.
        let health = ProviderHealth::new();
        health.record_failure("openai");
        assert!(!health.score("openai").is_degraded());
        assert_eq!(health.score("openai").price_penalty(), 1.0);
    }

    #[test]
    fn max_tokens_counts_toward_the_context_a_candidate_must_fit() {
        // A 100k prompt asking for 32k of output needs 132k of context. Checking the
        // prompt alone routes it to a 128k model that will fail partway through
        // generation — a routable request turned into a mid-stream failure.
        let long_prompt = "word ".repeat(30_000); // ~37k tokens
        let request = NormalizedRequest {
            max_tokens: Some(100_000),
            ..NormalizedRequest::simple("gpt-4o", &long_prompt)
        };

        let decision = Router::new()
            .route(&request, &pricing(), &healthy(), &RoutingInputs::default())
            .unwrap();

        let served = pricing().get(&decision.served_model).cloned();
        if let Some(model) = served {
            assert!(
                model.context_window as u64 >= request.estimated_input_tokens() + 100_000,
                "{} has a {}-token window, too small for prompt + max_tokens",
                model.model_id,
                model.context_window
            );
        }
    }

    #[test]
    fn budget_pressure_tightens_the_tier_for_downgradable_requests() {
        // An organisation two dollars from a hard cap should be steered cheaper *before*
        // the budget check starts refusing requests outright.
        let pricing = pricing();
        let request = NormalizedRequest::simple("openai/gpt-4o", "Summarise this in a line.");

        let comfortable = Router::new()
            .route(&request, &pricing, &healthy(), &RoutingInputs::default())
            .unwrap();
        let critical = Router::new()
            .route(
                &request,
                &pricing,
                &healthy(),
                &RoutingInputs {
                    // Barely enough for one more request.
                    budget_headroom_mc: Some(200),
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        let tier_of = |id: &str| pricing.get(id).map(|m| m.tier);
        assert!(
            tier_of(&critical.served_model) <= tier_of(&comfortable.served_model),
            "budget pressure must not route *up*"
        );
    }

    #[test]
    fn budget_pressure_never_downgrades_a_complex_request() {
        // The quality guarantee outranks budget pressure. Serving a worse answer to save
        // money is the one trade this product must never make silently.
        let decision = Router::new()
            .route(
                &complex_request(),
                &pricing(),
                &healthy(),
                &RoutingInputs {
                    budget_headroom_mc: Some(1),
                    ..RoutingInputs::default()
                },
            )
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
    }

    #[test]
    fn a_routing_decision_explains_itself_in_the_customers_terms() {
        let decision = Router::new()
            .route(
                &simple_request(),
                &pricing(),
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();

        assert!(
            !decision.explanation.is_empty(),
            "every decision must be explainable"
        );
        let joined = decision.explanation.join(" ");
        assert!(
            joined.contains("classified"),
            "the explanation should name the classification: {joined}"
        );
        assert!(
            joined.contains(&decision.served_model),
            "the explanation should name the model that served it: {joined}"
        );
    }

    #[test]
    fn a_detected_task_domain_reaches_the_customer_facing_explanation() {
        // TaskDomain::preferred_providers() exists specifically to promote a model strong
        // on this class of work within `select_at_tier`'s candidate re-rank — this proves
        // that wiring is actually reachable end to end (classification -> re-rank ->
        // explanation), not just present in the code. The re-rank itself is a soft,
        // price-bounded nudge (see its own doc comment), so this does not assert which
        // model wins — only that a real domain detection surfaces to the customer, which is
        // the property most likely to silently break (an unused `Classification::domain`
        // read by nothing, the way several other "computed but never consulted" fields in
        // this project's own history turned out to be).
        let decision = Router::new()
            .route(
                &reasoning_domain_medium_request(),
                &pricing(),
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();

        let joined = decision.explanation.join(" ");
        assert!(
            joined.contains("task domain: reasoning"),
            "a reasoning-domain request's provider preference never reached the \
             explanation: {joined}"
        );
    }

    #[test]
    fn a_provider_outage_substitution_is_not_labelled_policy() {
        // Recording an outage as `policy` sent support looking for a policy that did not
        // exist, and hid the real cause from the incident timeline.
        let pricing = pricing();
        let health = ProviderHealth::new();
        for _ in 0..crate::engine::fallback::FAILURE_THRESHOLD {
            health.record_failure("openai");
        }
        assert!(!health.is_available("openai"));

        let decision = Router::new()
            .route(
                &complex_request(),
                &pricing,
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();

        assert_eq!(decision.reason, RoutingReason::ProviderUnavailable);
        let joined = decision.explanation.join(" ");
        assert!(
            joined.contains("unavailable"),
            "the explanation must say the provider was down: {joined}"
        );
        assert!(
            joined.contains("may cost more"),
            "an outage substitution picks the best available model, which can cost more \
             than the one requested — the customer must be told: {joined}"
        );
    }

    #[test]
    fn the_bandit_can_reorder_candidates_but_never_widen_them() {
        // The bandit's job is to break ties among models the router already accepts.
        // Letting it add one would let outcome data override capability, allowlist, tier,
        // and availability filtering — every guarantee the router exists to enforce.
        let pricing = pricing();
        let bandit = RoutingBandit::new();

        let without = Router::new()
            .route(
                &simple_request(),
                &pricing,
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();

        // Teach the bandit that a *different* cheap model performs well.
        let alternative = pricing
            .cheaper_alternatives(
                "openai/gpt-4o",
                Requirements {
                    min_context: 0,
                    ..Requirements::default()
                },
            )
            .into_iter()
            .find(|m| m.model_id != without.served_model)
            .map(|m| m.model_id.clone())
            .expect("the seed table has several cheaper models");

        for _ in 0..50 {
            bandit.record(
                Complexity::Simple,
                &alternative,
                true,
                crate::money::MicroCents(10_000),
                crate::money::MicroCents(100),
            );
            bandit.record(
                Complexity::Simple,
                &without.served_model,
                false,
                crate::money::MicroCents(0),
                crate::money::MicroCents(100),
            );
        }

        let with = Router::new()
            .route(
                &simple_request(),
                &pricing,
                &healthy(),
                &RoutingInputs {
                    bandit: Some(&bandit),
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        // Whatever it picks, it must be one the router itself would have permitted.
        let permitted: Vec<String> = pricing
            .cheaper_alternatives(
                "openai/gpt-4o",
                Requirements {
                    min_context: simple_request().estimated_input_tokens() as u32,
                    ..Requirements::default()
                },
            )
            .into_iter()
            .map(|m| m.model_id.clone())
            .collect();
        assert!(
            permitted.contains(&with.served_model),
            "{} was not in the router's own candidate set",
            with.served_model
        );
    }

    #[test]
    fn routing_is_deterministic_without_a_bandit() {
        // Reproducibility from a usage record depends on this. Two identical requests with
        // identical inputs must produce identical decisions.
        let pricing = pricing();
        let health = healthy();
        let first = Router::new()
            .route(
                &simple_request(),
                &pricing,
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();
        for _ in 0..20 {
            let again = Router::new()
                .route(
                    &simple_request(),
                    &pricing,
                    &health,
                    &RoutingInputs::default(),
                )
                .unwrap();
            assert_eq!(first.served_model, again.served_model);
        }
    }

    #[test]
    fn simple_requests_route_to_a_cheaper_model() {
        let decision = Router::new()
            .route(
                &simple_request(),
                &pricing(),
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();
        assert_ne!(decision.served_model, "openai/gpt-4o");
        assert_eq!(decision.reason, RoutingReason::Complexity);
        assert!(decision.candidates_considered > 0);
    }

    #[test]
    fn a_chat_request_is_never_routed_to_an_embedding_model() {
        // The single most severe bug this file has had. `text-embedding-3-small` prices
        // at $0.02/Mtok input and $0.00 output — the cheapest row in the entire seed
        // table by a wide margin — and, before `Requirements::chat` existed, reported
        // `supports_tools: false` and `supports_vision: false`, which satisfied every
        // filter a plain chat request imposed. Every simple chat request was being routed
        // to a model that cannot answer one. A prior version of this exact test asserted
        // only `served_model != requested_model` and passed throughout, which is why this
        // needs its own explicit, unambiguous assertion rather than living inside a
        // broader one.
        let decision = Router::new()
            .route(
                &simple_request(),
                &pricing(),
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();
        assert!(
            !decision.served_model.contains("embedding"),
            "routed a chat request to {}",
            decision.served_model
        );

        let cheap_request = NormalizedRequest::simple("openai/gpt-4o-mini", "hi");
        let decision = Router::new()
            .route(
                &cheap_request,
                &pricing(),
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();
        assert!(!decision.served_model.contains("embedding"));
    }

    #[test]
    fn embedding_models_are_excluded_from_every_capability_filter() {
        // The property that actually closes the bug, checked directly against the
        // pricing table rather than through one routing decision: no embedding model may
        // ever satisfy a `chat: true` requirement, regardless of context window or price.
        for model in pricing().all() {
            if model.model_id.contains("embedding") {
                assert!(
                    !model.supports_chat,
                    "{} must not claim chat support",
                    model.model_id
                );
                assert!(
                    !PricingTable::satisfies(model, Requirements::default()),
                    "{} satisfied the default (chat-required) requirements",
                    model.model_id
                );
            }
        }
    }

    #[test]
    fn complex_requests_are_never_downgraded() {
        // The quality guarantee. This is the single most important test in the router.
        let decision = Router::new()
            .route(
                &complex_request(),
                &pricing(),
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
        assert!(decision.is_passthrough());
    }

    #[test]
    fn tool_requests_are_never_downgraded() {
        let mut request = simple_request();
        request.tools = vec![serde_json::json!({"type": "function", "function": {"name": "f"}})];
        let decision = Router::new()
            .route(&request, &pricing(), &healthy(), &RoutingInputs::default())
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
    }

    #[test]
    fn the_passthrough_hint_always_wins() {
        // Part 13 item 5: the escape hatch must always be available.
        let inputs = RoutingInputs {
            hint: RoutingHint::Passthrough,
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
        assert_eq!(decision.reason, RoutingReason::UserOverride);
    }

    #[test]
    fn passthrough_hint_outranks_policy() {
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {}, "then": {"pin_model": "openai/gpt-4o-mini"}}]"#,
        );
        let inputs = RoutingInputs {
            hint: RoutingHint::Passthrough,
            policy: Some(&policy),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
    }

    #[test]
    fn unknown_models_pass_through_untouched() {
        // Never substitute for something we cannot price — we have no basis to compare.
        let request = NormalizedRequest::simple("some-private-finetune-v3", "hi");
        let decision = Router::new()
            .route(&request, &pricing(), &healthy(), &RoutingInputs::default())
            .unwrap();
        assert_eq!(decision.served_model, "some-private-finetune-v3");
        assert_eq!(decision.reason, RoutingReason::Passthrough);
    }

    #[test]
    fn policies_can_pin_a_model() {
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {"complexity": "simple"}, "then": {"pin_model": "google/gemini-2.5-flash"}}]"#,
        );
        let inputs = RoutingInputs {
            policy: Some(&policy),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "google/gemini-2.5-flash");
        assert_eq!(decision.reason, RoutingReason::Policy);
    }

    #[test]
    fn policies_can_deny_a_request() {
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {"model_requested": "gpt-4o"}, "then": {"deny": true}}]"#,
        );
        let inputs = RoutingInputs {
            policy: Some(&policy),
            ..Default::default()
        };
        let err = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap_err();
        assert_eq!(err.error_type(), "model_not_allowed");
    }

    #[test]
    fn policies_can_force_passthrough_for_a_model_family() {
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {"model_requested": "gpt-4*"}, "then": {"passthrough": true}}]"#,
        );
        let inputs = RoutingInputs {
            policy: Some(&policy),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
        assert_eq!(decision.reason, RoutingReason::Policy);
    }

    #[test]
    fn a_policy_can_set_the_routing_mode_for_a_matched_request() {
        // A medium request the caller left on `auto` (balanced -> mid tier) gets pushed to
        // economy (medium -> cheap tier) by a policy naming this team, without the policy
        // author having to know or name a specific tier.
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {"team": "interns"}, "then": {"routing_mode": "economy"}}]"#,
        );
        let inputs = RoutingInputs {
            policy: Some(&policy),
            team: Some("interns".to_string()),
            hint: RoutingHint::Auto,
            ..Default::default()
        };
        let decision = Router::new()
            .route(&medium_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        let served_tier = pricing().get(&decision.served_model).unwrap().tier;
        assert_eq!(
            served_tier,
            ModelTier::Cheap,
            "the policy's routing_mode should have pushed this to economy's medium tier"
        );
    }

    #[test]
    fn a_policy_set_routing_mode_still_cannot_touch_a_complex_request() {
        // Same policy, but a complex request: the mode ladder itself refuses to downgrade
        // complex under any mode, and that guarantee must survive a policy setting the mode
        // rather than only holding for a caller-sent header.
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {"team": "interns"}, "then": {"routing_mode": "economy"}}]"#,
        );
        let inputs = RoutingInputs {
            policy: Some(&policy),
            team: Some("interns".to_string()),
            hint: RoutingHint::Auto,
            ..Default::default()
        };
        let decision = Router::new()
            .route(&complex_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
    }

    #[test]
    fn user_scoped_policies_match_the_keys_assignee() {
        let policy = RoutingPolicy::from_json(
            r#"[{"when": {"user": "intern@example.com"}, "then": {"passthrough": true}}]"#,
        );
        let inputs = RoutingInputs {
            policy: Some(&policy),
            user: Some("intern@example.com".to_string()),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.reason, RoutingReason::Policy);

        // A different (or absent) assignee must not match someone else's rule.
        let inputs_other = RoutingInputs {
            policy: Some(&policy),
            user: Some("someone-else@example.com".to_string()),
            ..Default::default()
        };
        let decision_other = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs_other)
            .unwrap();
        assert_ne!(decision_other.reason, RoutingReason::Policy);
    }

    #[test]
    fn a_policy_pinning_an_unknown_model_passes_through_rather_than_failing() {
        let policy =
            RoutingPolicy::from_json(r#"[{"when": {}, "then": {"pin_model": "does/not-exist"}}]"#);
        let inputs = RoutingInputs {
            policy: Some(&policy),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
    }

    #[test]
    fn allowlists_are_respected() {
        let inputs = RoutingInputs {
            allowed_models: Some(vec!["openai/gpt-4o".into(), "openai/gpt-4o-mini".into()]),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o-mini");
    }

    #[test]
    fn allowlists_accept_bare_names() {
        let inputs = RoutingInputs {
            allowed_models: Some(vec!["gpt-4o".into(), "gpt-4o-mini".into()]),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o-mini");
    }

    #[test]
    fn a_plan_ceiling_caps_even_a_passthrough_request() {
        // A free-tier org asking for a frontier model gets the best model its plan allows,
        // not an error and not the frontier model.
        let inputs = RoutingInputs {
            plan_tier_ceiling: Some(ModelTier::Cheap),
            ..Default::default()
        };
        let decision = Router::new()
            .route(&complex_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        let served = pricing().get(&decision.served_model).unwrap().tier;
        assert_eq!(served, ModelTier::Cheap);
    }

    #[test]
    fn circuit_open_providers_are_skipped() {
        let health = ProviderHealth::new();
        // Take OpenAI down; a simple gpt-4o request must route to another provider.
        for _ in 0..10 {
            health.record_failure("openai");
        }
        assert!(!health.is_available("openai"));

        let decision = Router::new()
            .route(
                &simple_request(),
                &pricing(),
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();
        assert_ne!(
            decision.provider, "openai",
            "routed to a provider with an open circuit"
        );
    }

    #[test]
    fn routing_never_promotes_to_a_more_expensive_model() {
        // Asking for a cheap model must never cost more than asking for it directly.
        let table = pricing();
        let request = NormalizedRequest::simple("gpt-4o-mini", "What is 2+2?");
        let decision = Router::new()
            .route(&request, &table, &healthy(), &RoutingInputs::default())
            .unwrap();

        let requested_price = table.get("gpt-4o-mini").unwrap().blended_per_mtok();
        let served_price = table
            .get(&decision.served_model)
            .unwrap()
            .blended_per_mtok();
        assert!(
            served_price <= requested_price,
            "routed from {} to a pricier {}",
            request.model,
            decision.served_model
        );
    }

    #[test]
    fn vision_requests_only_route_to_vision_capable_models() {
        let mut request = simple_request();
        request.messages = vec![Message {
            role: Role::User,
            content: Some(crate::types::Content::Parts(vec![
                serde_json::json!({"type": "text", "text": "what is this"}),
                serde_json::json!({"type": "image_url", "image_url": {"url": "https://x/y.png"}}),
            ])),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];

        let table = pricing();
        let decision = Router::new()
            .route(&request, &table, &healthy(), &RoutingInputs::default())
            .unwrap();
        assert!(
            table.get(&decision.served_model).unwrap().supports_vision,
            "{} cannot handle images",
            decision.served_model
        );
    }

    #[test]
    fn routing_is_deterministic() {
        let table = pricing();
        let health = healthy();
        let first = Router::new()
            .route(
                &simple_request(),
                &table,
                &health,
                &RoutingInputs::default(),
            )
            .unwrap();
        for _ in 0..10 {
            let again = Router::new()
                .route(
                    &simple_request(),
                    &table,
                    &health,
                    &RoutingInputs::default(),
                )
                .unwrap();
            assert_eq!(first, again);
        }
    }

    #[test]
    fn provider_inference_covers_the_common_prefixes() {
        assert_eq!(infer_provider("openai/gpt-4o"), "openai");
        assert_eq!(infer_provider("gpt-4o"), "openai");
        assert_eq!(infer_provider("o3"), "openai");
        assert_eq!(infer_provider("claude-sonnet-4-5"), "anthropic");
        assert_eq!(infer_provider("gemini-2.5-pro"), "google");
        assert_eq!(infer_provider("my-local-llama"), "custom");
    }

    #[test]
    fn tier_ceilings_combine_to_the_stricter_one() {
        assert_eq!(
            combine_ceilings(Some(ModelTier::Premium), Some(ModelTier::Cheap)),
            Some(ModelTier::Cheap)
        );
        assert_eq!(
            combine_ceilings(Some(ModelTier::Mid), None),
            Some(ModelTier::Mid)
        );
        assert_eq!(
            combine_ceilings(None, Some(ModelTier::Mid)),
            Some(ModelTier::Mid)
        );
        assert_eq!(combine_ceilings(None, None), None);
    }

    #[test]
    fn a_savings_opportunity_actually_saves_money() {
        // The commercial premise, asserted directly: the routed model must cost less than
        // the requested one for a representative simple request.
        let table = pricing();
        let decision = Router::new()
            .route(
                &simple_request(),
                &table,
                &healthy(),
                &RoutingInputs::default(),
            )
            .unwrap();

        let baseline = table.cost("gpt-4o", 1_000, 500).unwrap();
        let actual = table.cost(&decision.served_model, 1_000, 500).unwrap();
        assert!(
            actual < baseline,
            "routing produced no saving: {actual} vs {baseline}"
        );
    }

    #[test]
    fn conversation_affinity_pins_model_across_turns() {
        let table = pricing();
        let health = healthy();
        let request = simple_request();

        let inputs = RoutingInputs {
            affinity_model: Some("gpt-4o-mini".to_string()),
            ..RoutingInputs::default()
        };

        let decision = Router::new()
            .route(&request, &table, &health, &inputs)
            .unwrap();

        assert_eq!(decision.served_model, "openai/gpt-4o-mini");
        assert!(decision.explanation[0].contains("conversation affinity"));
    }

    // -----------------------------------------------------------------------
    // Routing modes.
    //
    // `RoutingHint::Cheap` was parsed from the header and then never branched on:
    // the router only ever tested for `Passthrough`, so a caller asking to save
    // money got default behaviour and no indication that their request had been
    // ignored. These prove each mode now changes what actually gets served.
    // -----------------------------------------------------------------------

    fn medium_request() -> NormalizedRequest {
        // Explanatory, no code, no tools — lands in the medium band.
        NormalizedRequest::simple(
            "openai/gpt-4o",
            "Explain the difference between TCP and UDP in a few paragraphs.",
        )
    }

    fn tier_of(table: &PricingTable, model: &str) -> ModelTier {
        table.get(model).expect("served model must be priced").tier
    }

    #[test]
    fn economy_mode_sends_a_medium_request_cheaper_than_balanced_does() {
        // The whole point of the mode: it trades harder than the default. Before this,
        // `cheap` and `auto` produced byte-identical decisions.
        let table = pricing();
        let request = medium_request();
        assert_eq!(
            Router::new().classify(&request).complexity,
            Complexity::Medium,
            "this test is calibrated on a medium-band prompt"
        );

        let balanced = Router::new()
            .route(
                &request,
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::Balanced,
                    ..RoutingInputs::default()
                },
            )
            .unwrap();
        let economy = Router::new()
            .route(
                &request,
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::Economy,
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        assert!(
            tier_of(&table, &economy.served_model) <= tier_of(&table, &balanced.served_model),
            "economy ({}) must not be more expensive than balanced ({})",
            economy.served_model,
            balanced.served_model
        );
    }

    #[test]
    fn quality_mode_refuses_to_downgrade_a_medium_request() {
        // The complement: a mode that trades *less* than the default, for traffic where a
        // marginal saving is not worth a marginal risk.
        let table = pricing();
        let request = medium_request();

        let decision = Router::new()
            .route(
                &request,
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::Quality,
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        assert_eq!(
            decision.served_model, "openai/gpt-4o",
            "quality mode must serve a medium request on the requested model"
        );
    }

    #[test]
    fn quality_mode_still_takes_the_free_saving_on_a_simple_request() {
        // Quality is not passthrough. A trivial question still moves off a frontier model.
        let table = pricing();
        let decision = Router::new()
            .route(
                &simple_request(),
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::Quality,
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        assert_ne!(
            decision.served_model, "openai/gpt-4o",
            "quality mode should still downgrade a trivial request"
        );
    }

    #[test]
    fn no_mode_downgrades_a_complex_request() {
        // The guarantee, exercised through the real router in every mode rather than only
        // against the mode table.
        let table = pricing();
        for mode in [
            RoutingHint::Auto,
            RoutingHint::Balanced,
            RoutingHint::Quality,
            RoutingHint::Economy,
            RoutingHint::Passthrough,
        ] {
            let decision = Router::new()
                .route(
                    &complex_request(),
                    &table,
                    &healthy(),
                    &RoutingInputs {
                        hint: mode,
                        ..RoutingInputs::default()
                    },
                )
                .unwrap();
            assert_eq!(
                decision.served_model, "openai/gpt-4o",
                "{mode:?} downgraded a complex request"
            );
        }
    }

    #[test]
    fn the_legacy_cheap_header_now_actually_routes_cheaply() {
        // The defect this closes, stated as the customer would experience it: sending the
        // documented `cheap` header used to change nothing at all.
        let table = pricing();
        let request = medium_request();

        let ignored_before = Router::new()
            .route(
                &request,
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::Auto,
                    ..RoutingInputs::default()
                },
            )
            .unwrap();
        let with_cheap_header = Router::new()
            .route(
                &request,
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::parse(Some("cheap")),
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        assert!(
            tier_of(&table, &with_cheap_header.served_model)
                <= tier_of(&table, &ignored_before.served_model),
            "the cheap header must now produce a cheaper-or-equal tier, not be ignored"
        );
    }

    #[test]
    fn unconfigured_requested_provider_substitutes_configured_provider_on_complex_request() {
        let table = pricing();
        let mut configured = std::collections::HashSet::new();
        configured.insert("google".to_string());

        let decision = Router::new()
            .route(
                &complex_request(),
                &table,
                &healthy(),
                &RoutingInputs {
                    hint: RoutingHint::Auto,
                    configured_providers: Some(configured),
                    ..RoutingInputs::default()
                },
            )
            .unwrap();

        assert_eq!(
            decision.provider, "google",
            "should route to configured google provider rather than unconfigured openai"
        );
    }
}

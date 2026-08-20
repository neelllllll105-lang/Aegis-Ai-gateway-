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
//! 3. Classifier complexity band.
//! 4. Passthrough.

use crate::engine::classifier::{Classification, Classifier};
use crate::engine::fallback::ProviderHealth;
use crate::engine::policy::{PolicyContext, RoutingPolicy};
use crate::error::{AegisError, Result};
use crate::metering::pricing::{PricingTable, Requirements};
use crate::types::{Complexity, ModelTier, NormalizedRequest, RoutingHint, RoutingReason};

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
    /// Models this org may use at all. `None` means no restriction.
    pub allowed_models: Option<Vec<String>>,
    /// Hard tier ceiling from the org's plan (the free tier is capped at cheap models).
    pub plan_tier_ceiling: Option<ModelTier>,
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
        let requirements = Requirements {
            tools: request.requires_tools(),
            vision: request.requires_vision(),
            min_context: request.estimated_input_tokens().min(u32::MAX as u64) as u32,
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
            });
        };

        // [1] The caller's escape hatch outranks everything we might prefer.
        if inputs.hint == RoutingHint::Passthrough {
            return Ok(self.passthrough(requested, RoutingReason::UserOverride));
        }

        let classification = self.classifier.classify(request);

        // [2] Organisation policy.
        if let Some(policy) = inputs.policy {
            let context = PolicyContext {
                complexity: Some(classification.complexity),
                model_requested: request.model.clone(),
                team: inputs.team.clone(),
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
                        Some(classification.score),
                    ) {
                        return Ok(decision);
                    }
                }
                return Ok(self.passthrough(requested, RoutingReason::Policy));
            }
        }

        // [3] Complexity-driven selection.
        let target_tier = match classification.complexity {
            // Never downgrade a hard request. This is the line that protects quality.
            Complexity::Complex => {
                return Ok(self.passthrough_capped(
                    requested,
                    pricing,
                    health,
                    requirements,
                    inputs,
                    Some(classification.score),
                ))
            }
            Complexity::Medium => ModelTier::Mid,
            Complexity::Simple => ModelTier::Cheap,
        };

        let ceiling = combine_ceilings(Some(target_tier), inputs.plan_tier_ceiling)
            .unwrap_or(target_tier);

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
            Some(classification.score),
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

        if allowed_by_plan && allowed_by_list && provider_up {
            return RoutingDecision {
                served_model: requested.model_id.clone(),
                provider: requested.provider.clone(),
                reason: RoutingReason::Passthrough,
                complexity_score: score,
                candidates_considered: 0,
            };
        }

        let ceiling = inputs.plan_tier_ceiling.unwrap_or(ModelTier::Frontier);
        let fallback = pricing
            .all()
            .filter(|m| m.tier <= ceiling)
            .filter(|m| PricingTable::satisfies(m, requirements))
            .filter(|m| is_allowed(&m.model_id, inputs))
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
                reason: RoutingReason::Policy,
                complexity_score: score,
                candidates_considered: 1,
            },
            // Nothing at all is available. Passing through gives the provider a chance to
            // answer and the caller a real upstream error rather than one we invented.
            None => RoutingDecision {
                served_model: requested.model_id.clone(),
                provider: requested.provider.clone(),
                reason: RoutingReason::Passthrough,
                complexity_score: score,
                candidates_considered: 0,
            },
        }
    }

    /// Cheapest capable, available, permitted model at or below `tier`, strictly cheaper
    /// than the requested model.
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
        score: Option<f32>,
    ) -> Option<RoutingDecision> {
        let candidates: Vec<_> = pricing
            .cheaper_alternatives(&requested.model_id, requirements)
            .into_iter()
            .filter(|m| m.tier <= tier)
            .filter(|m| is_allowed(&m.model_id, inputs))
            .filter(|m| health.is_available(&m.provider))
            .collect();

        let considered = candidates.len();
        candidates.first().map(|model| RoutingDecision {
            served_model: model.model_id.clone(),
            provider: model.provider.clone(),
            reason,
            complexity_score: score,
            candidates_considered: considered,
        })
    }
}

/// The stricter (lower) of two optional tier ceilings.
fn combine_ceilings(a: Option<ModelTier>, b: Option<ModelTier>) -> Option<ModelTier> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
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

    #[test]
    fn simple_requests_route_to_a_cheaper_model() {
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &RoutingInputs::default())
            .unwrap();
        assert_ne!(decision.served_model, "openai/gpt-4o");
        assert_eq!(decision.reason, RoutingReason::Complexity);
        assert!(decision.candidates_considered > 0);
    }

    #[test]
    fn complex_requests_are_never_downgraded() {
        // The quality guarantee. This is the single most important test in the router.
        let decision = Router::new()
            .route(&complex_request(), &pricing(), &healthy(), &RoutingInputs::default())
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
        let inputs = RoutingInputs { hint: RoutingHint::Passthrough, ..Default::default() };
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
        let inputs = RoutingInputs { policy: Some(&policy), ..Default::default() };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "google/gemini-2.5-flash");
        assert_eq!(decision.reason, RoutingReason::Policy);
    }

    #[test]
    fn policies_can_deny_a_request() {
        let policy =
            RoutingPolicy::from_json(r#"[{"when": {"model_requested": "gpt-4o"}, "then": {"deny": true}}]"#);
        let inputs = RoutingInputs { policy: Some(&policy), ..Default::default() };
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
        let inputs = RoutingInputs { policy: Some(&policy), ..Default::default() };
        let decision = Router::new()
            .route(&simple_request(), &pricing(), &healthy(), &inputs)
            .unwrap();
        assert_eq!(decision.served_model, "openai/gpt-4o");
        assert_eq!(decision.reason, RoutingReason::Policy);
    }

    #[test]
    fn a_policy_pinning_an_unknown_model_passes_through_rather_than_failing() {
        let policy =
            RoutingPolicy::from_json(r#"[{"when": {}, "then": {"pin_model": "does/not-exist"}}]"#);
        let inputs = RoutingInputs { policy: Some(&policy), ..Default::default() };
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
            .route(&simple_request(), &pricing(), &health, &RoutingInputs::default())
            .unwrap();
        assert_ne!(decision.provider, "openai", "routed to a provider with an open circuit");
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
        let served_price = table.get(&decision.served_model).unwrap().blended_per_mtok();
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
            .route(&simple_request(), &table, &health, &RoutingInputs::default())
            .unwrap();
        for _ in 0..10 {
            let again = Router::new()
                .route(&simple_request(), &table, &health, &RoutingInputs::default())
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
        assert_eq!(combine_ceilings(Some(ModelTier::Mid), None), Some(ModelTier::Mid));
        assert_eq!(combine_ceilings(None, Some(ModelTier::Mid)), Some(ModelTier::Mid));
        assert_eq!(combine_ceilings(None, None), None);
    }

    #[test]
    fn a_savings_opportunity_actually_saves_money() {
        // The commercial premise, asserted directly: the routed model must cost less than
        // the requested one for a representative simple request.
        let table = pricing();
        let decision = Router::new()
            .route(&simple_request(), &table, &healthy(), &RoutingInputs::default())
            .unwrap();

        let baseline = table.cost("gpt-4o", 1_000, 500).unwrap();
        let actual = table.cost(&decision.served_model, 1_000, 500).unwrap();
        assert!(actual < baseline, "routing produced no saving: {actual} vs {baseline}");
    }
}

//! Routing policies — the governance layer above the classifier.
//!
//! A policy is an ordered list of `when -> then` rules stored as JSONB in
//! `routing_policies.rules`. Policies exist because organisations need routing to be
//! *predictable*, not merely cheap: a team that has decided its production traffic never
//! leaves a premium model must be able to say so, and have that outranked by nothing.
//!
//! # Precedence
//!
//! Rules are evaluated in order and the **first match wins**, so a specific rule placed
//! above a general one overrides it. Within the pipeline, an explicit policy match
//! outranks the classifier entirely — a human decision beats a heuristic.
//!
//! ```json
//! [
//!   {"when": {"team": "interns"},              "then": {"max_model_tier": "mid"}},
//!   {"when": {"model_requested": "gpt-4o*"},   "then": {"pin_model": "openai/gpt-4o"}},
//!   {"when": {"complexity": "simple"},         "then": {"model_tier": "cheap"}}
//! ]
//! ```

use crate::types::{Complexity, ModelTier};
use serde::{Deserialize, Serialize};

/// Conditions a rule matches on. All present conditions must hold (logical AND); absent
/// conditions are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    /// Match a complexity band.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<String>,
    /// Match the requested model. Supports a single trailing `*` wildcard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_requested: Option<String>,
    /// Match a team name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Match when the request needs tool calling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_tools: Option<bool>,
    /// Match when estimated input tokens exceed a threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_input_tokens: Option<u64>,
}

/// What to do when a rule matches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// Route to the cheapest capable model at this tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_tier: Option<String>,
    /// Never route above this tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_model_tier: Option<String>,
    /// Route to exactly this model, overriding everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_model: Option<String>,
    /// Reject the request outright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<bool>,
    /// Send the requested model through untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passthrough: Option<bool>,
}

/// One `when -> then` rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(rename = "when", default)]
    pub condition: Condition,
    #[serde(rename = "then")]
    pub action: Action,
}

/// An organisation's ordered rule set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutingPolicy {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// The facts a rule is evaluated against.
#[derive(Debug, Clone, Default)]
pub struct PolicyContext {
    pub complexity: Option<Complexity>,
    pub model_requested: String,
    pub team: Option<String>,
    pub requires_tools: bool,
    pub estimated_input_tokens: u64,
}

impl RoutingPolicy {
    /// Parse a policy from stored JSON.
    ///
    /// A malformed policy yields an empty policy rather than an error: a bad rule set
    /// must not take an organisation's traffic offline. The gateway falls back to
    /// classifier-driven routing, which is safe, and the parse failure is logged for the
    /// operator to fix.
    pub fn from_json(raw: &str) -> RoutingPolicy {
        match serde_json::from_str::<Vec<Rule>>(raw) {
            Ok(rules) => RoutingPolicy { rules },
            Err(e) => {
                tracing::warn!(error = %e, "invalid routing policy; falling back to defaults");
                RoutingPolicy::default()
            }
        }
    }

    /// True when there are no rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The action from the first matching rule, if any.
    pub fn evaluate(&self, context: &PolicyContext) -> Option<&Action> {
        self.rules
            .iter()
            .find(|rule| rule.condition.matches(context))
            .map(|rule| &rule.action)
    }
}

impl Condition {
    /// True when every specified condition holds.
    pub fn matches(&self, context: &PolicyContext) -> bool {
        if let Some(expected) = &self.complexity {
            match context.complexity {
                Some(actual) if actual.as_str() == expected.to_ascii_lowercase() => {}
                _ => return false,
            }
        }
        if let Some(pattern) = &self.model_requested {
            if !matches_pattern(pattern, &context.model_requested) {
                return false;
            }
        }
        if let Some(expected) = &self.team {
            match &context.team {
                Some(actual) if actual.eq_ignore_ascii_case(expected) => {}
                _ => return false,
            }
        }
        if let Some(expected) = self.requires_tools {
            if context.requires_tools != expected {
                return false;
            }
        }
        if let Some(threshold) = self.min_input_tokens {
            if context.estimated_input_tokens < threshold {
                return false;
            }
        }
        true
    }
}

impl Action {
    /// The tier to target, if this action sets one.
    pub fn target_tier(&self) -> Option<ModelTier> {
        self.model_tier.as_deref().map(ModelTier::parse)
    }

    /// The tier ceiling, if this action sets one.
    pub fn tier_ceiling(&self) -> Option<ModelTier> {
        self.max_model_tier.as_deref().map(ModelTier::parse)
    }

    /// True when the action forbids the request.
    pub fn is_deny(&self) -> bool {
        self.deny.unwrap_or(false)
    }

    /// True when the action forces passthrough.
    pub fn is_passthrough(&self) -> bool {
        self.passthrough.unwrap_or(false)
    }
}

/// Match a model name against a pattern with an optional single trailing `*`.
///
/// Deliberately not a full glob or regex: policies are written by humans in a text box,
/// and a prefix wildcard covers the real cases (`claude-opus-*`, `gpt-4*`) without
/// exposing a way to write an expression that hangs the router.
fn matches_pattern(pattern: &str, value: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> PolicyContext {
        PolicyContext {
            complexity: Some(Complexity::Simple),
            model_requested: "gpt-4o".to_string(),
            team: Some("engineering".to_string()),
            requires_tools: false,
            estimated_input_tokens: 500,
        }
    }

    #[test]
    fn empty_conditions_match_everything() {
        assert!(Condition::default().matches(&context()));
    }

    #[test]
    fn complexity_conditions_match_the_band() {
        let condition = Condition {
            complexity: Some("simple".into()),
            ..Default::default()
        };
        assert!(condition.matches(&context()));

        let mut other = context();
        other.complexity = Some(Complexity::Complex);
        assert!(!condition.matches(&other));

        // An unclassified request cannot satisfy a complexity condition.
        other.complexity = None;
        assert!(!condition.matches(&other));
    }

    #[test]
    fn model_patterns_support_a_trailing_wildcard() {
        let exact = Condition {
            model_requested: Some("gpt-4o".into()),
            ..Default::default()
        };
        assert!(exact.matches(&context()));

        let wildcard = Condition {
            model_requested: Some("gpt-4*".into()),
            ..Default::default()
        };
        assert!(wildcard.matches(&context()));

        let miss = Condition {
            model_requested: Some("claude-*".into()),
            ..Default::default()
        };
        assert!(!miss.matches(&context()));
    }

    #[test]
    fn pattern_matching_is_case_insensitive_and_trimmed() {
        assert!(matches_pattern(" GPT-4O ", "gpt-4o"));
        assert!(matches_pattern("Claude-Opus-*", "claude-opus-4-5"));
        assert!(!matches_pattern("gpt-4o", "gpt-4o-mini"));
        // A wildcard is only honoured at the end; a leading one is a literal.
        assert!(!matches_pattern("*-mini", "gpt-4o-mini"));
    }

    #[test]
    fn team_conditions_are_case_insensitive() {
        let condition = Condition {
            team: Some("ENGINEERING".into()),
            ..Default::default()
        };
        assert!(condition.matches(&context()));

        let mut no_team = context();
        no_team.team = None;
        assert!(!condition.matches(&no_team));
    }

    #[test]
    fn token_threshold_conditions_work() {
        let condition = Condition {
            min_input_tokens: Some(1_000),
            ..Default::default()
        };
        assert!(!condition.matches(&context()));

        let mut large = context();
        large.estimated_input_tokens = 5_000;
        assert!(condition.matches(&large));
    }

    #[test]
    fn all_conditions_must_hold() {
        let condition = Condition {
            complexity: Some("simple".into()),
            team: Some("engineering".into()),
            model_requested: Some("gpt-4*".into()),
            ..Default::default()
        };
        assert!(condition.matches(&context()));

        let mut wrong_team = context();
        wrong_team.team = Some("marketing".into());
        assert!(
            !condition.matches(&wrong_team),
            "conditions must AND, not OR"
        );
    }

    #[test]
    fn first_matching_rule_wins() {
        // Order is the whole mechanism: a specific rule above a general one overrides it.
        let policy = RoutingPolicy::from_json(
            r#"[
                {"when": {"team": "engineering"}, "then": {"pin_model": "openai/gpt-4o"}},
                {"when": {"complexity": "simple"}, "then": {"model_tier": "cheap"}}
            ]"#,
        );
        let action = policy.evaluate(&context()).unwrap();
        assert_eq!(action.pin_model.as_deref(), Some("openai/gpt-4o"));
        assert!(
            action.model_tier.is_none(),
            "the second rule must not also apply"
        );
    }

    #[test]
    fn non_matching_policies_yield_nothing() {
        let policy =
            RoutingPolicy::from_json(r#"[{"when": {"team": "finance"}, "then": {"deny": true}}]"#);
        assert!(policy.evaluate(&context()).is_none());
    }

    #[test]
    fn the_documented_example_parses_and_behaves() {
        let policy = RoutingPolicy::from_json(
            r#"[
                {"when": {"team": "interns"}, "then": {"max_model_tier": "mid"}},
                {"when": {"model_requested": "claude-opus-*"}, "then": {"passthrough": true}},
                {"when": {"complexity": "simple"}, "then": {"model_tier": "cheap"}}
            ]"#,
        );
        assert_eq!(policy.rules.len(), 3);

        let mut intern = context();
        intern.team = Some("interns".into());
        assert_eq!(
            policy.evaluate(&intern).unwrap().tier_ceiling(),
            Some(ModelTier::Mid)
        );

        let mut opus = context();
        opus.team = Some("engineering".into());
        opus.model_requested = "claude-opus-4-5".into();
        assert!(policy.evaluate(&opus).unwrap().is_passthrough());

        // Falls through to the complexity rule.
        assert_eq!(
            policy.evaluate(&context()).unwrap().target_tier(),
            Some(ModelTier::Cheap)
        );
    }

    #[test]
    fn malformed_policy_json_does_not_take_traffic_offline() {
        // A broken rule set must degrade to default routing, never to an error page.
        for broken in ["not json at all", "{}", "[{\"when\": 5}]", ""] {
            let policy = RoutingPolicy::from_json(broken);
            assert!(
                policy.is_empty(),
                "{broken:?} should have produced an empty policy"
            );
            assert!(policy.evaluate(&context()).is_none());
        }
    }

    #[test]
    fn policies_round_trip_through_json() {
        let policy = RoutingPolicy {
            rules: vec![Rule {
                condition: Condition {
                    complexity: Some("complex".into()),
                    ..Default::default()
                },
                action: Action {
                    passthrough: Some(true),
                    ..Default::default()
                },
            }],
        };
        let json = serde_json::to_string(&policy.rules).unwrap();
        assert_eq!(RoutingPolicy::from_json(&json), policy);
        // Absent fields must not serialize as nulls, which would bloat every stored row.
        assert!(!json.contains("null"), "{json}");
    }

    #[test]
    fn tier_parsing_from_actions() {
        let action = Action {
            model_tier: Some("cheap".into()),
            max_model_tier: Some("premium".into()),
            ..Default::default()
        };
        assert_eq!(action.target_tier(), Some(ModelTier::Cheap));
        assert_eq!(action.tier_ceiling(), Some(ModelTier::Premium));
        assert!(!action.is_deny());
        assert!(!action.is_passthrough());
    }
}

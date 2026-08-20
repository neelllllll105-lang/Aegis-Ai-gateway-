//! The internal request/response vocabulary.
//!
//! Every inbound request — OpenAI-shaped or Anthropic-shaped — is normalised into
//! [`NormalizedRequest`] at pipeline stage [4], and every provider response is normalised
//! back into [`NormalizedResponse`] at stage [8]. Adapters translate at the edges; the
//! engine only ever sees these types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Who produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    /// Anthropic's name for a system-adjacent developer message.
    Developer,
}

impl Role {
    /// The wire value used by OpenAI-compatible providers.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::Developer => "developer",
        }
    }
}

/// Message content: either plain text or an array of typed parts (vision, audio).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    /// The overwhelmingly common case.
    Text(String),
    /// Multimodal parts. Kept as raw JSON so a new part type from a provider does not
    /// require a gateway release to pass through.
    Parts(Vec<serde_json::Value>),
}

impl Content {
    /// Flatten to text for classification, fingerprinting, and token estimation.
    /// Non-text parts contribute their `text` field when present.
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(t) => t.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// True when any part is an image. Drives the vision capability requirement.
    pub fn has_image(&self) -> bool {
        match self {
            Content::Text(_) => false,
            Content::Parts(parts) => parts.iter().any(|p| {
                p.get("type")
                    .and_then(|t| t.as_str())
                    .is_some_and(|t| t.contains("image"))
            }),
        }
    }

    /// Character length of the textual content.
    pub fn text_len(&self) -> usize {
        self.as_text().len()
    }
}

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// Construct a simple text message.
    pub fn text(role: Role, content: impl Into<String>) -> Message {
        Message {
            role,
            content: Some(Content::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Textual content, or an empty string.
    pub fn text_content(&self) -> String {
        self.content.as_ref().map(|c| c.as_text()).unwrap_or_default()
    }
}

/// A normalised request, the single form the engine operates on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedRequest {
    /// The model the caller asked for. Never mutated — it is the savings baseline.
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
    /// Caller-supplied end-user identifier, passed through for provider abuse tooling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Parameters we do not model explicitly, preserved so provider-specific options
    /// survive the round trip.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl NormalizedRequest {
    /// A minimal request, for tests and internal probes.
    pub fn simple(model: &str, prompt: &str) -> NormalizedRequest {
        NormalizedRequest {
            model: model.to_string(),
            messages: vec![Message::text(Role::User, prompt)],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: false,
            tools: Vec::new(),
            tool_choice: None,
            stop: None,
            response_format: None,
            user: None,
            extra: BTreeMap::new(),
        }
    }

    /// Concatenated text of every message. Used for classification and estimation.
    pub fn all_text(&self) -> String {
        self.messages
            .iter()
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The final user message — the strongest signal of what is actually being asked.
    pub fn last_user_message(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.text_content())
    }

    /// Combined system and developer prompt text.
    pub fn system_text(&self) -> String {
        self.messages
            .iter()
            .filter(|m| matches!(m.role, Role::System | Role::Developer))
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// True when the request needs tool calling.
    pub fn requires_tools(&self) -> bool {
        !self.tools.is_empty()
    }

    /// True when any message carries an image.
    pub fn requires_vision(&self) -> bool {
        self.messages
            .iter()
            .filter_map(|m| m.content.as_ref())
            .any(|c| c.has_image())
    }

    /// Rough input token count.
    ///
    /// Deliberately an estimate: running a real tokenizer for every candidate model on
    /// the hot path would blow the sub-millisecond budget, and this figure is used only
    /// for routing and pre-flight budget projection. Actual billing always uses the token
    /// counts the provider reports. Four characters per token is the long-standing
    /// approximation for English text and code; message framing adds a few tokens each.
    pub fn estimated_input_tokens(&self) -> u64 {
        let text_tokens = self.all_text().chars().count() as u64 / 4;
        let framing = self.messages.len() as u64 * 4;
        let tool_tokens = self
            .tools
            .iter()
            .map(|t| t.to_string().chars().count() as u64 / 4)
            .sum::<u64>();
        text_tokens + framing + tool_tokens
    }
}

/// Token counts for one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// True when the provider did not report usage and we estimated it. Surfaced in the
    /// usage record so a customer disputing an invoice can see which figures were exact.
    #[serde(default)]
    pub estimated: bool,
}

impl TokenUsage {
    /// Total tokens across input and output.
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// A normalised provider response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedResponse {
    pub id: String,
    /// The model that actually served the request, as reported by the provider.
    pub model: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    pub usage: TokenUsage,
    /// The provider's own response body, preserved for faithful passthrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// One chunk of a streaming response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamChunk {
    /// Incremental text.
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Usage, present only on the final chunk for most providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// The raw SSE `data:` payload, for byte-faithful passthrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Capability tier of a model. Ordered: `Cheap < Mid < Premium < Frontier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    Cheap,
    Mid,
    Premium,
    Frontier,
}

impl ModelTier {
    /// Parse a tier from its database representation, defaulting to `Mid` for unknown
    /// values — the conservative choice, since it neither over- nor under-promises.
    pub fn parse(raw: &str) -> ModelTier {
        match raw.to_ascii_lowercase().as_str() {
            "cheap" => ModelTier::Cheap,
            "premium" => ModelTier::Premium,
            "frontier" => ModelTier::Frontier,
            _ => ModelTier::Mid,
        }
    }

    /// Wire/database representation.
    pub fn as_str(self) -> &'static str {
        match self {
            ModelTier::Cheap => "cheap",
            ModelTier::Mid => "mid",
            ModelTier::Premium => "premium",
            ModelTier::Frontier => "frontier",
        }
    }
}

/// Complexity band assigned by the classifier at stage [6a].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    Simple,
    Medium,
    Complex,
}

impl Complexity {
    /// Band a raw 0.0–1.0 score using the thresholds in `MASTER_BUILD.md` Part 5 [6a].
    pub fn from_score(score: f32) -> Complexity {
        if score < 0.35 {
            Complexity::Simple
        } else if score <= 0.7 {
            Complexity::Medium
        } else {
            Complexity::Complex
        }
    }

    /// Wire representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Complexity::Simple => "simple",
            Complexity::Medium => "medium",
            Complexity::Complex => "complex",
        }
    }
}

/// Why a request was routed the way it was. Recorded on every usage record so any
/// routing decision can be explained after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingReason {
    /// An explicit org policy matched.
    Policy,
    /// The classifier selected a cheaper model.
    Complexity,
    /// The primary choice failed and we fell back.
    Fallback,
    /// Served from cache.
    Cache,
    /// Sent to the requested model unchanged.
    Passthrough,
    /// The caller forced passthrough via `X-Aegis-Routing-Hint`.
    UserOverride,
}

impl RoutingReason {
    /// Wire representation, stored in `usage_records.routing_reason`.
    pub fn as_str(self) -> &'static str {
        match self {
            RoutingReason::Policy => "policy",
            RoutingReason::Complexity => "complexity",
            RoutingReason::Fallback => "fallback",
            RoutingReason::Cache => "cache",
            RoutingReason::Passthrough => "passthrough",
            RoutingReason::UserOverride => "user_override",
        }
    }
}

/// Cache lookup outcome at stage [5].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheOutcome {
    Exact,
    Semantic,
    Miss,
    /// Caching was not attempted (non-deterministic request, or zero-retention org).
    Skipped,
}

impl CacheOutcome {
    /// Wire representation, used in the `X-Aegis-Cache` header.
    pub fn as_str(self) -> &'static str {
        match self {
            CacheOutcome::Exact => "exact",
            CacheOutcome::Semantic => "semantic",
            CacheOutcome::Miss => "miss",
            CacheOutcome::Skipped => "skipped",
        }
    }

    /// True when the response was served without an upstream call.
    pub fn is_hit(self) -> bool {
        matches!(self, CacheOutcome::Exact | CacheOutcome::Semantic)
    }
}

/// Caller-supplied routing hint from the `X-Aegis-Routing-Hint` header.
///
/// [`RoutingHint::Passthrough`] is the escape hatch that Part 13 item 5 says must always
/// be available: trust outranks savings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutingHint {
    /// Let the engine decide.
    #[default]
    Auto,
    /// Use exactly the requested model. Never downgrade.
    Passthrough,
    /// Prefer the cheapest capable model.
    Cheap,
}

impl RoutingHint {
    /// Parse the header value. Unknown values fall back to `Auto` rather than erroring —
    /// a typo in a hint should not fail a paid request.
    pub fn parse(raw: Option<&str>) -> RoutingHint {
        match raw.map(|r| r.trim().to_ascii_lowercase()).as_deref() {
            Some("passthrough") => RoutingHint::Passthrough,
            Some("cheap") => RoutingHint::Cheap,
            _ => RoutingHint::Auto,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complexity_bands_match_the_specified_thresholds() {
        assert_eq!(Complexity::from_score(0.0), Complexity::Simple);
        assert_eq!(Complexity::from_score(0.34), Complexity::Simple);
        assert_eq!(Complexity::from_score(0.35), Complexity::Medium);
        assert_eq!(Complexity::from_score(0.70), Complexity::Medium);
        assert_eq!(Complexity::from_score(0.71), Complexity::Complex);
        assert_eq!(Complexity::from_score(1.0), Complexity::Complex);
    }

    #[test]
    fn tiers_are_ordered_by_capability() {
        assert!(ModelTier::Cheap < ModelTier::Mid);
        assert!(ModelTier::Mid < ModelTier::Premium);
        assert!(ModelTier::Premium < ModelTier::Frontier);
    }

    #[test]
    fn unknown_tier_is_treated_as_mid() {
        // Conservative: an unrecognised tier must not be assumed cheap and substituted in.
        assert_eq!(ModelTier::parse("who-knows"), ModelTier::Mid);
        assert_eq!(ModelTier::parse("CHEAP"), ModelTier::Cheap);
    }

    #[test]
    fn routing_hint_parsing_is_forgiving() {
        assert_eq!(RoutingHint::parse(Some("passthrough")), RoutingHint::Passthrough);
        assert_eq!(RoutingHint::parse(Some("  PASSTHROUGH ")), RoutingHint::Passthrough);
        assert_eq!(RoutingHint::parse(Some("cheap")), RoutingHint::Cheap);
        assert_eq!(RoutingHint::parse(Some("nonsense")), RoutingHint::Auto);
        assert_eq!(RoutingHint::parse(None), RoutingHint::Auto);
    }

    #[test]
    fn openai_request_shape_deserializes() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "You are terse."},
                {"role": "user", "content": "What is 2+2?"}
            ],
            "temperature": 0.2,
            "stream": false
        });
        let req: NormalizedRequest = serde_json::from_value(body).unwrap();
        assert_eq!(req.model, "gpt-4o");
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.last_user_message().as_deref(), Some("What is 2+2?"));
        assert_eq!(req.system_text(), "You are terse.");
        assert!(!req.stream);
    }

    #[test]
    fn multimodal_content_is_understood() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What is in this picture?"},
                    {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}}
                ]
            }]
        });
        let req: NormalizedRequest = serde_json::from_value(body).unwrap();
        assert!(req.requires_vision());
        assert_eq!(req.last_user_message().as_deref(), Some("What is in this picture?"));
    }

    #[test]
    fn unknown_provider_parameters_survive_the_round_trip() {
        // A provider adds a parameter tomorrow; we must pass it through, not drop it.
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "seed": 42,
            "logit_bias": {"50256": -100}
        });
        let req: NormalizedRequest = serde_json::from_value(body).unwrap();
        assert_eq!(req.extra.get("seed"), Some(&serde_json::json!(42)));
        assert!(req.extra.contains_key("logit_bias"));
    }

    #[test]
    fn tool_requests_are_detected() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "weather?"}],
            "tools": [{"type": "function", "function": {"name": "get_weather"}}]
        });
        let req: NormalizedRequest = serde_json::from_value(body).unwrap();
        assert!(req.requires_tools());
        assert!(!req.requires_vision());
    }

    #[test]
    fn token_estimation_scales_with_content() {
        let short = NormalizedRequest::simple("gpt-4o", "hi");
        let long = NormalizedRequest::simple("gpt-4o", &"word ".repeat(1_000));
        assert!(long.estimated_input_tokens() > short.estimated_input_tokens() * 100);
        // ~5000 characters at 4 chars/token, plus framing.
        assert!((1_200..1_400).contains(&long.estimated_input_tokens()));
    }

    #[test]
    fn empty_request_estimates_do_not_panic() {
        let req = NormalizedRequest {
            messages: vec![],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        assert_eq!(req.estimated_input_tokens(), 0);
        assert_eq!(req.last_user_message(), None);
        assert_eq!(req.all_text(), "");
    }

    #[test]
    fn cache_outcome_hit_detection() {
        assert!(CacheOutcome::Exact.is_hit());
        assert!(CacheOutcome::Semantic.is_hit());
        assert!(!CacheOutcome::Miss.is_hit());
        assert!(!CacheOutcome::Skipped.is_hit());
    }
}

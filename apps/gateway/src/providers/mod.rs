//! Provider adapters.
//!
//! Each upstream (OpenAI, Anthropic, Google, and anything OpenAI-compatible) implements
//! [`Provider`]. `MASTER_BUILD.md` Part 8 defines the trait and the rollout order.
//!
//! # Why translation is split from transport
//!
//! Every adapter exposes two **pure** functions — [`Provider::build_body`] and
//! [`Provider::parse_response`] — alongside the async `chat`. Request and response
//! translation is where provider bugs actually live, and keeping it free of I/O means the
//! golden-file tests in `tests/` can pin the exact JSON we send and the exact parsing of
//! what we get back, with no network and no mocking framework.

pub mod anthropic;
pub mod compat;
pub mod custom;
pub mod deepseek;
pub mod google;
pub mod groq;
pub mod mistral;
pub mod mock;
pub mod moonshot;
pub mod openai;
pub mod openrouter;
pub mod pool;
pub mod sse;

use crate::error::{AegisError, Result};
use crate::types::{NormalizedRequest, NormalizedResponse, StreamChunk};
use async_trait::async_trait;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// A stream of response chunks.
pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>;

/// Credentials for one upstream call.
///
/// Constructed per request from either the org's decrypted BYOK credential or a key from
/// our shared free-tier pool. Never logged, never serialized.
#[derive(Clone)]
pub struct Credential {
    /// The provider API key in plaintext. Lives only in memory, only for this request.
    pub api_key: String,
    /// Override base URL, for custom endpoints and self-hosted deployments.
    pub base_url: Option<String>,
}

impl Credential {
    /// Construct from a plaintext key.
    pub fn new(api_key: impl Into<String>) -> Credential {
        Credential { api_key: api_key.into(), base_url: None }
    }

    /// Construct with a custom base URL.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Credential {
        Credential { api_key: api_key.into(), base_url: Some(base_url.into()) }
    }
}

// A manual Debug impl: the derived one would print the key, and a stray `{:?}` in an
// error path is exactly how credentials end up in logs.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// One upstream provider.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable provider identifier, matching `provider_credentials.provider`.
    fn id(&self) -> &'static str;

    /// Models this adapter can serve, as bare names.
    fn supported_models(&self) -> &[&'static str];

    /// Default API base URL.
    fn default_base_url(&self) -> &'static str;

    /// Translate a normalised request into this provider's request body. Pure.
    fn build_body(&self, request: &NormalizedRequest, model: &str) -> serde_json::Value;

    /// Translate a provider response body back into our normalised form. Pure.
    fn parse_response(&self, body: &serde_json::Value) -> Result<NormalizedResponse>;

    /// Parse one SSE `data:` line from a streaming response. Pure.
    ///
    /// Returns `Ok(None)` for lines that carry no content (keep-alives, terminators).
    fn parse_stream_chunk(&self, data: &str) -> Result<Option<StreamChunk>>;

    /// The chat completions path appended to the base URL.
    fn chat_path(&self, model: &str) -> String;

    /// Authentication headers for this provider.
    fn auth_headers(&self, credential: &Credential) -> Vec<(String, String)>;

    /// True when this adapter can serve `model`.
    fn supports(&self, model: &str) -> bool {
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        self.supported_models().iter().any(|m| *m == bare)
    }

    /// Execute a non-streaming chat completion.
    async fn chat(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
    ) -> Result<NormalizedResponse> {
        let base = credential
            .base_url
            .as_deref()
            .unwrap_or_else(|| self.default_base_url());
        let url = format!("{}{}", base.trim_end_matches('/'), self.chat_path(model));

        let mut builder = http
            .post(&url)
            .timeout(timeout)
            .json(&self.build_body(request, model));
        for (name, value) in self.auth_headers(credential) {
            builder = builder.header(name, value);
        }

        let response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                AegisError::ProviderTimeout(timeout.as_secs())
            } else {
                AegisError::Provider {
                    provider: self.id().to_string(),
                    status: 502,
                    // `e` renders the URL but never headers, so no key can appear here.
                    message: e.to_string(),
                }
            }
        })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(AegisError::Provider {
                provider: self.id().to_string(),
                status: status.as_u16(),
                message: extract_provider_error(&body),
            });
        }

        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| AegisError::Provider {
            provider: self.id().to_string(),
            status: 502,
            message: format!("unparseable response: {e}"),
        })?;

        self.parse_response(&json)
    }

    /// Execute a streaming chat completion.
    async fn chat_stream(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
    ) -> Result<ChunkStream>;
}

/// Pull a human-readable message out of a provider error body.
///
/// Providers disagree on the shape (`error.message`, `message`, `error` as a bare
/// string), and a caller debugging a 400 needs the actual reason, not `"see upstream"`.
pub fn extract_provider_error(body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        for path in [
            json.pointer("/error/message"),
            json.pointer("/error"),
            json.pointer("/message"),
            json.pointer("/detail"),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(text) = path.as_str() {
                return crate::telemetry::redact(text);
            }
        }
    }
    // Fall back to a bounded, redacted slice of the raw body.
    let trimmed: String = body.chars().take(500).collect();
    crate::telemetry::redact(&trimmed)
}

/// Registry of available adapters.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// An empty registry.
    pub fn new() -> ProviderRegistry {
        ProviderRegistry::default()
    }

    /// Register an adapter.
    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.id().to_string(), provider);
    }

    /// Look up by provider id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).cloned()
    }

    /// The adapter for a canonical `provider/model` id.
    pub fn for_model(&self, model_id: &str) -> Option<Arc<dyn Provider>> {
        if let Some((provider_id, _)) = model_id.split_once('/') {
            if let Some(provider) = self.get(provider_id) {
                return Some(provider);
            }
        }
        // Bare model name: find any adapter claiming it.
        self.providers
            .values()
            .find(|p| p.supports(model_id))
            .cloned()
    }

    /// Registered provider ids, sorted.
    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.providers.keys().map(|k| k.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    /// Number of registered adapters.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// True when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Registry with every shipped adapter.
    ///
    /// Phase 1 adapters (openai, anthropic, google, custom) plus the Phase 5 expansion
    /// (openrouter, moonshot, deepseek, mistral, groq).
    pub fn with_builtins() -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(openai::OpenAiProvider));
        registry.register(Arc::new(anthropic::AnthropicProvider));
        registry.register(Arc::new(google::GoogleProvider));
        registry.register(Arc::new(custom::CustomProvider));
        registry.register(Arc::new(openrouter::OpenRouterProvider));
        registry.register(Arc::new(moonshot::MoonshotProvider));
        registry.register(Arc::new(deepseek::DeepSeekProvider));
        registry.register(Arc::new(mistral::MistralProvider));
        registry.register(Arc::new(groq::GroqProvider));
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_every_shipped_provider() {
        let registry = ProviderRegistry::with_builtins();
        for expected in [
            "openai", "anthropic", "google", "custom", "openrouter", "moonshot", "deepseek",
            "mistral", "groq",
        ] {
            assert!(registry.get(expected).is_some(), "missing provider: {expected}");
        }
        assert_eq!(registry.len(), 9);
    }

    #[test]
    fn canonical_model_ids_route_to_their_provider() {
        let registry = ProviderRegistry::with_builtins();
        assert_eq!(registry.for_model("openai/gpt-4o").unwrap().id(), "openai");
        assert_eq!(
            registry.for_model("anthropic/claude-sonnet-4-5").unwrap().id(),
            "anthropic"
        );
        assert_eq!(registry.for_model("google/gemini-2.5-flash").unwrap().id(), "google");
    }

    #[test]
    fn bare_model_names_resolve_to_a_capable_provider() {
        let registry = ProviderRegistry::with_builtins();
        assert_eq!(registry.for_model("gpt-4o").unwrap().id(), "openai");
    }

    #[test]
    fn unknown_models_resolve_to_nothing() {
        let registry = ProviderRegistry::with_builtins();
        assert!(registry.for_model("nonexistent/model-x").is_none());
    }

    #[test]
    fn credential_debug_never_prints_the_key() {
        // The whole reason for the manual Debug impl.
        let credential = Credential::new("sk-proj-supersecret-value-here");
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("supersecret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn provider_errors_are_extracted_from_every_common_shape() {
        assert_eq!(
            extract_provider_error(r#"{"error":{"message":"invalid model"}}"#),
            "invalid model"
        );
        assert_eq!(extract_provider_error(r#"{"error":"quota exceeded"}"#), "quota exceeded");
        assert_eq!(extract_provider_error(r#"{"message":"bad request"}"#), "bad request");
        assert_eq!(extract_provider_error(r#"{"detail":"not found"}"#), "not found");
    }

    #[test]
    fn provider_error_extraction_redacts_leaked_credentials() {
        // Providers sometimes echo the offending key back in the error message.
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-proj-abcdefghijklmnop123456"}}"#;
        let extracted = extract_provider_error(body);
        assert!(!extracted.contains("abcdefghijklmnop"), "{extracted}");
        assert!(extracted.contains("[REDACTED]"));
    }

    #[test]
    fn unparseable_error_bodies_are_bounded_and_safe() {
        let huge = "x".repeat(10_000);
        let extracted = extract_provider_error(&huge);
        assert!(extracted.len() <= 500);

        assert_eq!(extract_provider_error(""), "");
        assert_eq!(extract_provider_error("<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");
    }

    #[test]
    fn supports_matches_bare_and_qualified_names() {
        let provider = openai::OpenAiProvider;
        assert!(provider.supports("gpt-4o"));
        assert!(provider.supports("openai/gpt-4o"));
        assert!(!provider.supports("claude-sonnet-4-5"));
    }
}

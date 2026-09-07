//! DeepSeek adapter — OpenAI-compatible everywhere except cache-token accounting.
//!
//! This provider used to be declared with [`crate::openai_compatible_provider!`], like
//! Mistral, Groq, and Moonshot. That was wrong in one specific way: DeepSeek reports
//! prompt-cache accounting as top-level `prompt_cache_hit_tokens` /
//! `prompt_cache_miss_tokens` fields, not OpenAI's `prompt_tokens_details.cached_tokens`.
//! Routed through the macro, `prompt_tokens_details` is never present in a DeepSeek
//! response, so `cached` always parsed as zero — every request DeepSeek itself served
//! (partly) from cache was billed as a full-rate miss. DeepSeek's cache discount is roughly
//! 90% off the input rate, so this silently over-billed every cache-hit request by close to
//! the full input cost. Found auditing this guide's metering section (`IG-1` §2.4), not
//! from a support ticket — no live DeepSeek traffic has ever been served.
//!
//! Everything else about the wire format — request body, response shape, SSE framing,
//! bearer auth — is identical to OpenAI's, so this hand-written adapter still delegates to
//! [`crate::providers::openai`] for all of it, per the pattern that module's own doc
//! comment describes: "A provider that later diverges simply stops using the macro and
//! gets a hand-written adapter."

use super::openai;
use super::{ChunkStream, Credential, Provider};
use crate::error::Result;
use crate::types::{NormalizedRequest, NormalizedResponse, StreamChunk, TokenUsage};
use async_trait::async_trait;
use std::time::Duration;

/// DeepSeek — very low cost chat and reasoning models.
pub struct DeepSeekProvider;

const MODELS: &[&str] = &["deepseek-chat", "deepseek-reasoner"];
const BASE_URL: &str = "https://api.deepseek.com/v1";

/// DeepSeek's own usage shape.
///
/// Unlike OpenAI, the cached and uncached portions are each reported directly — no
/// subtraction needed, and so no risk of a cached count larger than the total wrapping the
/// input figure the way OpenAI's `saturating_sub` guards against.
fn parse_usage(u: &serde_json::Value) -> TokenUsage {
    let field = |name: &str| u.get(name).and_then(|v| v.as_u64()).unwrap_or(0);
    TokenUsage {
        input_tokens: field("prompt_cache_miss_tokens"),
        output_tokens: field("completion_tokens"),
        cached_input_tokens: field("prompt_cache_hit_tokens"),
        // DeepSeek does not charge a separate premium for populating its cache.
        cache_write_tokens: 0,
        estimated: false,
    }
}

#[async_trait]
impl Provider for DeepSeekProvider {
    fn id(&self) -> &'static str {
        "deepseek"
    }

    fn supported_models(&self) -> &[&'static str] {
        MODELS
    }

    fn default_base_url(&self) -> &'static str {
        BASE_URL
    }

    fn build_body(&self, request: &NormalizedRequest, model: &str) -> serde_json::Value {
        // Compatible providers reject a provider-qualified id; send the bare name.
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        openai::build_body(request, bare)
    }

    fn parse_response(&self, body: &serde_json::Value) -> Result<NormalizedResponse> {
        openai::parse_response_with_usage(body, parse_usage)
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Option<StreamChunk>> {
        openai::parse_stream_chunk_with_usage(data, parse_usage)
    }

    fn chat_path(&self, _model: &str) -> String {
        "/chat/completions".to_string()
    }

    fn auth_headers(&self, credential: &Credential) -> Vec<(String, String)> {
        vec![(
            "authorization".to_string(),
            format!("Bearer {}", credential.api_key),
        )]
    }

    async fn chat_stream(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
        idempotency_key: Option<&str>,
    ) -> Result<ChunkStream> {
        let base = credential
            .base_url
            .as_deref()
            .unwrap_or_else(|| self.default_base_url());
        let url = format!("{}{}", base.trim_end_matches('/'), self.chat_path(model));
        let mut body = self.build_body(request, model);
        if let Some(map) = body.as_object_mut() {
            map.insert("stream".into(), serde_json::json!(true));
        }
        openai::open_stream(
            http,
            &url,
            body,
            super::with_idempotency_key(self.auth_headers(credential), idempotency_key),
            "deepseek",
            timeout,
            |data| openai::parse_stream_chunk_with_usage(data, parse_usage),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_identity_is_unchanged_by_the_rewrite() {
        let provider = DeepSeekProvider;
        assert_eq!(provider.id(), "deepseek");
        assert_eq!(provider.default_base_url(), BASE_URL);
        assert!(provider.supports("deepseek-chat"));
        assert!(provider.supports("deepseek/deepseek-reasoner"));
        assert!(!provider.supports("gpt-4o"));
    }

    #[test]
    fn strips_the_provider_prefix_from_the_body() {
        let provider = DeepSeekProvider;
        let request = NormalizedRequest::simple("deepseek-chat", "hi");
        let body = provider.build_body(&request, "deepseek/deepseek-chat");
        assert_eq!(body["model"], "deepseek-chat");
    }

    #[test]
    fn a_cache_hit_is_read_from_deepseeks_own_field_names() {
        // Before this fix, this exact response parsed to cached_input_tokens: 0 because the
        // adapter looked for `prompt_tokens_details.cached_tokens`, which DeepSeek never
        // sends — the whole 900 cached tokens billed as a full-rate miss.
        let body = serde_json::json!({
            "id": "x", "model": "deepseek-chat",
            "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 1000,
                "prompt_cache_hit_tokens": 900,
                "prompt_cache_miss_tokens": 100,
                "completion_tokens": 50
            }
        });
        let parsed = DeepSeekProvider.parse_response(&body).unwrap();
        assert_eq!(parsed.usage.input_tokens, 100);
        assert_eq!(parsed.usage.cached_input_tokens, 900);
        assert_eq!(parsed.usage.cache_write_tokens, 0);
        assert_eq!(parsed.usage.output_tokens, 50);
        assert!(!parsed.usage.estimated);
    }

    #[test]
    fn a_full_cache_miss_reports_zero_cached_tokens() {
        let body = serde_json::json!({
            "id": "x", "model": "deepseek-chat",
            "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 200,
                "prompt_cache_hit_tokens": 0,
                "prompt_cache_miss_tokens": 200,
                "completion_tokens": 10
            }
        });
        let parsed = DeepSeekProvider.parse_response(&body).unwrap();
        assert_eq!(parsed.usage.input_tokens, 200);
        assert_eq!(parsed.usage.cached_input_tokens, 0);
    }

    #[test]
    fn missing_usage_is_estimated_not_zero_billed() {
        let body = serde_json::json!({
            "id": "x", "model": "deepseek-chat",
            "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]
        });
        let parsed = DeepSeekProvider.parse_response(&body).unwrap();
        assert!(parsed.usage.estimated);
    }

    #[test]
    fn streaming_usage_chunk_also_reads_deepseeks_field_names() {
        let chunk = DeepSeekProvider
            .parse_stream_chunk(
                &serde_json::json!({
                    "choices": [{"delta": {}, "finish_reason": "stop"}],
                    "usage": {
                        "prompt_tokens": 500,
                        "prompt_cache_hit_tokens": 400,
                        "prompt_cache_miss_tokens": 100,
                        "completion_tokens": 20
                    }
                })
                .to_string(),
            )
            .unwrap()
            .expect("finish_reason + usage makes this a non-empty chunk");
        let usage = chunk.usage.expect("usage present on this chunk");
        assert_eq!(usage.cached_input_tokens, 400);
        assert_eq!(usage.input_tokens, 100);
    }

    #[test]
    fn auth_uses_bearer_scheme() {
        let headers = DeepSeekProvider.auth_headers(&Credential::new("sk-test"));
        assert_eq!(
            headers[0],
            ("authorization".to_string(), "Bearer sk-test".to_string())
        );
    }
}

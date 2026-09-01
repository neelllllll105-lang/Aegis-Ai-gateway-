//! Google Gemini adapter.
//!
//! Gemini departs from the OpenAI shape more than any other provider we support:
//!
//! * Messages are `contents`, each with `parts`, and the assistant role is `model`.
//! * The system prompt is `systemInstruction`, a parts object rather than a string.
//! * Sampling parameters live under `generationConfig`, with different names
//!   (`maxOutputTokens`, `stopSequences`).
//! * The model name is part of the **URL path**, not the body, and streaming is a
//!   different endpoint entirely.
//! * Token counts come back as `usageMetadata.promptTokenCount` /
//!   `candidatesTokenCount`.

use super::sse::is_done;
use super::{ChunkStream, Credential, Provider};
use crate::error::{AegisError, Result};
use crate::types::{NormalizedRequest, NormalizedResponse, Role, StreamChunk, TokenUsage};
use async_trait::async_trait;
use std::time::Duration;

/// Models served by Google.
const GOOGLE_MODELS: &[&str] = &[
    "gemini-2.5-flash",
    "gemini-2.5-pro",
    "gemini-2.0-flash",
    "gemini-1.5-flash",
    "gemini-1.5-pro",
    "gemini-2.5-flash-lite",
    "gemini-3.6-flash",
    "gemini-3.1-pro-preview",
];

/// Build a Gemini `generateContent` body.
pub fn build_body(request: &NormalizedRequest, _model: &str) -> serde_json::Value {
    let contents: Vec<serde_json::Value> = request
        .messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System | Role::Developer))
        .map(|m| {
            serde_json::json!({
                "role": if m.role == Role::Assistant { "model" } else { "user" },
                "parts": [{"text": m.text_content()}],
            })
        })
        .collect();

    let mut body = serde_json::json!({ "contents": contents });
    let map = body.as_object_mut().expect("constructed as an object");

    let system_text = request.system_text();
    if !system_text.is_empty() {
        map.insert(
            "systemInstruction".into(),
            serde_json::json!({"parts": [{"text": system_text}]}),
        );
    }

    let mut generation_config = serde_json::Map::new();
    if let Some(temperature) = request.temperature {
        generation_config.insert("temperature".into(), serde_json::json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        generation_config.insert("topP".into(), serde_json::json!(top_p));
    }
    if let Some(max_tokens) = request.max_tokens {
        generation_config.insert("maxOutputTokens".into(), serde_json::json!(max_tokens));
    }
    if let Some(stop) = &request.stop {
        let sequences = match stop {
            serde_json::Value::String(s) => serde_json::json!([s]),
            other => other.clone(),
        };
        generation_config.insert("stopSequences".into(), sequences);
    }
    if !generation_config.is_empty() {
        map.insert(
            "generationConfig".into(),
            serde_json::Value::Object(generation_config),
        );
    }

    if !request.tools.is_empty() {
        map.insert(
            "tools".into(),
            serde_json::json!([{
                "functionDeclarations": request
                    .tools
                    .iter()
                    .map(|t| t.get("function").cloned().unwrap_or_else(|| t.clone()))
                    .collect::<Vec<_>>()
            }]),
        );
    }

    body
}
/// Read Google's `usageMetadata` into normalised token counts.
///
/// Google follows OpenAI's convention: `promptTokenCount` **includes** the cached portion,
/// which is reported separately as `cachedContentTokenCount`. Same subtraction, same
/// reasoning, same `min` guard as the OpenAI adapter.
fn parse_usage(u: &serde_json::Value) -> TokenUsage {
    let field = |name: &str| u.get(name).and_then(|v| v.as_u64()).unwrap_or(0);
    let prompt_tokens = field("promptTokenCount");
    let cached = field("cachedContentTokenCount").min(prompt_tokens);

    TokenUsage {
        input_tokens: prompt_tokens.saturating_sub(cached),
        output_tokens: field("candidatesTokenCount"),
        cached_input_tokens: cached,
        // Context caching on Gemini is billed by storage duration, not per write.
        cache_write_tokens: 0,
        estimated: false,
    }
}

/// Parse a Gemini response.
pub fn parse_response(body: &serde_json::Value) -> Result<NormalizedResponse> {
    let candidate = body
        .pointer("/candidates/0")
        .ok_or_else(|| malformed("response contained no candidates"))?;

    let content = candidate
        .pointer("/content/parts")
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let usage = body
        .get("usageMetadata")
        .map(parse_usage)
        .unwrap_or(TokenUsage {
            estimated: true,
            ..Default::default()
        });

    Ok(NormalizedResponse {
        id: body
            .get("responseId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        model: body
            .get("modelVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        content,
        finish_reason: candidate
            .get("finishReason")
            .and_then(|v| v.as_str())
            .map(normalize_finish_reason),
        tool_calls: candidate
            .pointer("/content/parts")
            .and_then(|p| p.as_array())
            .filter(|parts| parts.iter().any(|p| p.get("functionCall").is_some()))
            .map(|parts| {
                serde_json::Value::Array(
                    parts
                        .iter()
                        .filter(|p| p.get("functionCall").is_some())
                        .cloned()
                        .collect(),
                )
            }),
        usage,
        raw: Some(body.clone()),
    })
}

/// Map Gemini's SCREAMING_CASE finish reasons onto the OpenAI vocabulary our clients
/// already branch on.
fn normalize_finish_reason(raw: &str) -> String {
    match raw {
        "STOP" => "stop".to_string(),
        "MAX_TOKENS" => "length".to_string(),
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" => {
            "content_filter".to_string()
        }
        other => other.to_ascii_lowercase(),
    }
}

/// Parse one Gemini SSE payload.
pub fn parse_stream_chunk(data: &str) -> Result<Option<StreamChunk>> {
    if data.is_empty() || is_done(data) {
        return Ok(None);
    }
    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
        return Ok(None);
    };

    let delta = json
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let finish_reason = json
        .pointer("/candidates/0/finishReason")
        .and_then(|v| v.as_str())
        .map(normalize_finish_reason);

    let usage = json.get("usageMetadata").map(parse_usage);

    if delta.is_empty() && finish_reason.is_none() && usage.is_none() {
        return Ok(None);
    }

    Ok(Some(StreamChunk {
        delta,
        finish_reason,
        usage,
        raw: Some(data.to_string()),
    }))
}

fn malformed(message: &str) -> AegisError {
    AegisError::Provider {
        provider: "google".to_string(),
        status: 502,
        message: message.to_string(),
    }
}

/// The Google adapter.
pub struct GoogleProvider;

#[async_trait]
impl Provider for GoogleProvider {
    fn id(&self) -> &'static str {
        "google"
    }

    fn supported_models(&self) -> &[&'static str] {
        GOOGLE_MODELS
    }

    fn default_base_url(&self) -> &'static str {
        "https://generativelanguage.googleapis.com/v1beta"
    }

    fn build_body(&self, request: &NormalizedRequest, model: &str) -> serde_json::Value {
        build_body(request, model)
    }

    fn parse_response(&self, body: &serde_json::Value) -> Result<NormalizedResponse> {
        parse_response(body)
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Option<StreamChunk>> {
        parse_stream_chunk(data)
    }

    /// Gemini puts the model in the path, so the bare name must be extracted from a
    /// canonical `google/gemini-...` id.
    fn chat_path(&self, model: &str) -> String {
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        format!("/models/{bare}:generateContent")
    }

    fn auth_headers(&self, credential: &Credential) -> Vec<(String, String)> {
        // Header auth rather than ?key=: a query parameter would land in access logs and
        // proxy caches. Part 9 forbids secrets in logs, including someone else's.
        vec![("x-goog-api-key".to_string(), credential.api_key.clone())]
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
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        let url = format!(
            "{}/models/{bare}:streamGenerateContent?alt=sse",
            base.trim_end_matches('/')
        );
        super::openai::open_stream(
            http,
            &url,
            self.build_body(request, model),
            super::with_idempotency_key(self.auth_headers(credential), idempotency_key),
            "google",
            timeout,
            parse_stream_chunk,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_content_tokens_are_subtracted_from_the_prompt_total() {
        // Google follows OpenAI's convention: promptTokenCount includes the cached part.
        let body = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "hi"}]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5_000,
                "candidatesTokenCount": 20,
                "cachedContentTokenCount": 4_500
            }
        });

        let usage = parse_response(&body).unwrap().usage;
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.cached_input_tokens, 4_500);
        assert_eq!(usage.total_input(), 5_000);
    }
    use crate::types::{Message, Role};

    fn request() -> NormalizedRequest {
        NormalizedRequest {
            messages: vec![
                Message::text(Role::System, "Be brief."),
                Message::text(Role::User, "Hello"),
                Message::text(Role::Assistant, "Hi"),
                Message::text(Role::User, "2+2?"),
            ],
            temperature: Some(0.4),
            max_tokens: Some(512),
            ..NormalizedRequest::simple("gemini-2.5-flash", "")
        }
    }

    #[test]
    fn messages_become_contents_with_parts() {
        let body = build_body(&request(), "gemini-2.5-flash");
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(
            contents.len(),
            3,
            "the system message must not appear in contents"
        );
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn assistant_role_is_renamed_to_model() {
        // Gemini rejects "assistant"; this rename is not cosmetic.
        let body = build_body(&request(), "gemini-2.5-flash");
        assert_eq!(body["contents"][1]["role"], "model");
    }

    #[test]
    fn system_prompt_becomes_system_instruction() {
        let body = build_body(&request(), "gemini-2.5-flash");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Be brief.");
    }

    #[test]
    fn sampling_parameters_move_into_generation_config_with_google_names() {
        let body = build_body(&request(), "gemini-2.5-flash");
        assert_eq!(body["generationConfig"]["temperature"], 0.4);
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 512);
        // The OpenAI spellings must not appear at the top level.
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn generation_config_is_omitted_when_nothing_is_set() {
        let plain = NormalizedRequest::simple("gemini-2.5-flash", "hi");
        let body = build_body(&plain, "gemini-2.5-flash");
        assert!(body.get("generationConfig").is_none());
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn model_travels_in_the_url_not_the_body() {
        let provider = GoogleProvider;
        assert_eq!(
            provider.chat_path("google/gemini-2.5-flash"),
            "/models/gemini-2.5-flash:generateContent"
        );
        assert_eq!(
            provider.chat_path("gemini-2.5-pro"),
            "/models/gemini-2.5-pro:generateContent"
        );
        let body = build_body(&request(), "gemini-2.5-flash");
        assert!(
            body.get("model").is_none(),
            "Gemini rejects a model field in the body"
        );
    }

    #[test]
    fn response_parsing_extracts_text_and_usage() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "4"}], "role": "model"},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 11, "candidatesTokenCount": 1, "totalTokenCount": 12
            },
            "modelVersion": "gemini-2.5-flash"
        });
        let parsed = parse_response(&body).unwrap();
        assert_eq!(parsed.content, "4");
        assert_eq!(parsed.usage.input_tokens, 11);
        assert_eq!(parsed.usage.output_tokens, 1);
        assert_eq!(parsed.model, "gemini-2.5-flash");
    }

    #[test]
    fn finish_reasons_are_normalized_to_the_openai_vocabulary() {
        // Clients branch on these strings; leaking Gemini's spelling breaks them.
        assert_eq!(normalize_finish_reason("STOP"), "stop");
        assert_eq!(normalize_finish_reason("MAX_TOKENS"), "length");
        assert_eq!(normalize_finish_reason("SAFETY"), "content_filter");
        assert_eq!(
            normalize_finish_reason("PROHIBITED_CONTENT"),
            "content_filter"
        );
        assert_eq!(normalize_finish_reason("OTHER"), "other");
    }

    #[test]
    fn multi_part_responses_are_concatenated() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "part one "}, {"text": "part two"}]},
                "finishReason": "STOP"
            }]
        });
        assert_eq!(parse_response(&body).unwrap().content, "part one part two");
    }

    #[test]
    fn missing_usage_metadata_is_flagged_estimated() {
        let body = serde_json::json!({
            "candidates": [{"content": {"parts": [{"text": "hi"}]}, "finishReason": "STOP"}]
        });
        assert!(parse_response(&body).unwrap().usage.estimated);
    }

    #[test]
    fn responses_without_candidates_are_rejected() {
        assert!(parse_response(&serde_json::json!({"usageMetadata": {}})).is_err());
    }

    #[test]
    fn stream_chunks_decode_text_and_final_usage() {
        let chunk =
            parse_stream_chunk(r#"{"candidates":[{"content":{"parts":[{"text":"Hel"}]}}]}"#)
                .unwrap()
                .unwrap();
        assert_eq!(chunk.delta, "Hel");

        let final_chunk = parse_stream_chunk(
            r#"{"candidates":[{"content":{"parts":[{"text":"lo"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(final_chunk.delta, "lo");
        assert_eq!(final_chunk.finish_reason.as_deref(), Some("stop"));
        assert_eq!(final_chunk.usage.unwrap().output_tokens, 2);
    }

    #[test]
    fn empty_stream_chunks_yield_nothing() {
        assert!(parse_stream_chunk(r#"{"candidates":[]}"#)
            .unwrap()
            .is_none());
        assert!(parse_stream_chunk("").unwrap().is_none());
        assert!(parse_stream_chunk("{bad json").unwrap().is_none());
    }

    #[test]
    fn api_key_travels_in_a_header_not_the_query_string() {
        // A ?key= parameter would be captured by every access log and proxy in the path.
        let headers = GoogleProvider.auth_headers(&Credential::new("AIzaTestKey"));
        assert_eq!(headers[0].0, "x-goog-api-key");
        assert_eq!(headers[0].1, "AIzaTestKey");
        assert!(!GoogleProvider
            .chat_path("gemini-2.5-flash")
            .contains("key="));
    }

    #[test]
    fn tools_are_wrapped_in_function_declarations() {
        let mut req = request();
        req.tools = vec![serde_json::json!({
            "type": "function",
            "function": {"name": "lookup", "parameters": {"type": "object"}}
        })];
        let body = build_body(&req, "gemini-2.5-flash");
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "lookup"
        );
    }
}

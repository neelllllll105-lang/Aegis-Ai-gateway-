//! OpenAI adapter, and the shared OpenAI-compatible translation layer.
//!
//! Six of our nine providers speak the OpenAI wire format (OpenAI itself, plus
//! OpenRouter, DeepSeek, Mistral, Groq, Moonshot, and any custom endpoint). The
//! translation functions here are `pub` so those adapters delegate rather than duplicate —
//! one place to fix a translation bug, one set of golden files to keep honest.

use super::sse::{is_done, SseDecoder};
use super::{ChunkStream, Credential, Provider};
use crate::error::{AegisError, Result};
use crate::types::{
    NormalizedRequest, NormalizedResponse, StreamChunk, TokenUsage, ToolCallDelta, WireShape,
};
use async_trait::async_trait;
use futures::StreamExt;
use std::time::Duration;

/// Build an OpenAI-format chat completion body.
///
/// `model` is passed separately from `request.model`: the requested model is the savings
/// baseline and must never be mutated, so the routing decision arrives as an argument.
pub fn build_body(request: &NormalizedRequest, model: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": request.messages,
    });

    let map = body.as_object_mut().expect("constructed as an object");

    if let Some(temperature) = request.temperature {
        map.insert("temperature".into(), serde_json::json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        map.insert("top_p".into(), serde_json::json!(top_p));
    }
    if let Some(max_tokens) = request.max_tokens {
        map.insert("max_tokens".into(), serde_json::json!(max_tokens));
    }
    if !request.tools.is_empty() {
        map.insert("tools".into(), serde_json::json!(request.tools));
    }
    if let Some(tool_choice) = &request.tool_choice {
        map.insert("tool_choice".into(), tool_choice.clone());
    }
    if let Some(stop) = &request.stop {
        map.insert("stop".into(), stop.clone());
    }
    if let Some(response_format) = &request.response_format {
        map.insert("response_format".into(), response_format.clone());
    }
    if let Some(user) = &request.user {
        map.insert("user".into(), serde_json::json!(user));
    }
    if request.stream {
        map.insert("stream".into(), serde_json::json!(true));
        // Without this, OpenAI omits usage from streamed responses entirely and we would
        // have to estimate every streamed request's tokens — and therefore its bill.
        map.insert(
            "stream_options".into(),
            serde_json::json!({"include_usage": true}),
        );
    }

    // Parameters we do not model explicitly (seed, logit_bias, presence_penalty, and
    // whatever ships next) pass through untouched.
    for (key, value) in &request.extra {
        map.entry(key.clone()).or_insert_with(|| value.clone());
    }

    body
}
/// Read OpenAI's usage block into normalised token counts.
///
/// OpenAI reports `prompt_tokens` with the cached portion **already inside it**, and
/// breaks the cached count out under `prompt_tokens_details.cached_tokens`. So the
/// uncached remainder is the difference, and treating `prompt_tokens` as full-rate input —
/// which is what this adapter did before the enterprise readiness audit — **over-counts**,
/// because OpenAI bills the cached portion at roughly a quarter of the input rate.
///
/// Note this is the exact opposite direction of Anthropic's error in the same code. Two
/// providers, two opposite biases, both invisible; normalising here is what makes a single
/// pricing rule correct for both.
///
/// `saturating_sub` rather than a subtraction: a provider reporting a cached count larger
/// than the prompt would otherwise wrap into an enormous token figure and an enormous bill.
fn parse_usage(u: &serde_json::Value) -> TokenUsage {
    let field = |name: &str| u.get(name).and_then(|v| v.as_u64()).unwrap_or(0);
    let prompt_tokens = field("prompt_tokens");
    let cached = u
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(prompt_tokens);

    TokenUsage {
        input_tokens: prompt_tokens.saturating_sub(cached),
        output_tokens: field("completion_tokens"),
        cached_input_tokens: cached,
        // OpenAI populates its cache automatically and does not charge separately for it.
        cache_write_tokens: 0,
        estimated: false,
    }
}

/// Parse an OpenAI-format chat completion response.
pub fn parse_response(body: &serde_json::Value) -> Result<NormalizedResponse> {
    let choice = body
        .pointer("/choices/0")
        .ok_or_else(|| malformed("response contained no choices"))?;

    let content = choice
        .pointer("/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();

    let tool_calls = choice.pointer("/message/tool_calls").cloned();

    // Usage may be absent (some compatible providers omit it). The caller estimates in
    // that case and marks the record `estimated`, so an invoice can always be explained.
    let usage = body.get("usage").map(parse_usage).unwrap_or(TokenUsage {
        estimated: true,
        ..Default::default()
    });

    Ok(NormalizedResponse {
        id: body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        model: body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        content,
        finish_reason: choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        tool_calls,
        usage,
        raw: Some(body.clone()),
    })
}

/// Parse one OpenAI-format SSE payload.
pub fn parse_stream_chunk(data: &str) -> Result<Option<StreamChunk>> {
    if data.is_empty() || is_done(data) {
        return Ok(None);
    }

    let json: serde_json::Value = match serde_json::from_str(data) {
        Ok(json) => json,
        // A malformed chunk mid-stream must not kill the stream: the client has already
        // received tokens, and dropping the connection loses them.
        Err(_) => return Ok(None),
    };

    let delta = json
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();

    let finish_reason = json
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let usage = json.get("usage").filter(|u| !u.is_null()).map(parse_usage);

    // A tool-call delta carries its payload under `delta.tool_calls`, not
    // `delta.content` — a streamed function call is typically *all* tool-call chunks
    // with an empty or absent `content` on every one of them. `delta` above only ever
    // looks at `.content`, so without this the loop below is essential: every tool-call
    // chunk in a streamed response looked exactly like the harmless role-announcement
    // chunk the emptiness check further down is designed to drop, and was silently
    // discarded — the client streamed a response with the tool call missing from it, no
    // error, nothing to explain why. Found by re-deriving the real OpenAI streaming wire
    // format from scratch while auditing this gateway's IDE/SDK compatibility claims, not
    // from a bug report.
    //
    // OpenAI streams at most one tool-call fragment per chunk in practice; `id`/`name`
    // arrive only on the fragment that opens a call, every later fragment for the same
    // call carries just `index` and the next slice of `arguments`.
    let tool_call = json
        .pointer("/choices/0/delta/tool_calls/0")
        .and_then(|tc| {
            let index = tc.get("index").and_then(|v| v.as_u64())? as u32;
            Some(ToolCallDelta {
                index,
                id: tc.get("id").and_then(|v| v.as_str()).map(str::to_string),
                name: tc
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                arguments_fragment: tc
                    .pointer("/function/arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
        });

    // A chunk with no delta, no tool-call payload, no finish reason, and no usage
    // carries nothing — this is what correctly drops OpenAI's role-only opening chunk.
    if delta.is_empty() && tool_call.is_none() && finish_reason.is_none() && usage.is_none() {
        return Ok(None);
    }

    Ok(Some(StreamChunk {
        delta,
        finish_reason,
        usage,
        raw: Some(data.to_string()),
        source_shape: Some(WireShape::OpenAiCompatible),
        tool_call,
    }))
}

fn malformed(message: &str) -> AegisError {
    AegisError::Provider {
        provider: "openai".to_string(),
        status: 502,
        message: message.to_string(),
    }
}

/// Open a streaming connection and decode it into [`StreamChunk`]s.
///
/// Shared by every OpenAI-compatible adapter.
pub async fn open_stream(
    http: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
    headers: Vec<(String, String)>,
    provider_id: &'static str,
    timeout: Duration,
    parse: fn(&str) -> Result<Option<StreamChunk>>,
) -> Result<ChunkStream> {
    let mut builder = http.post(url).timeout(timeout).json(&body);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }

    let response = builder.send().await.map_err(|e| {
        if e.is_timeout() {
            AegisError::ProviderTimeout(timeout.as_secs())
        } else {
            AegisError::Provider {
                provider: provider_id.to_string(),
                status: 502,
                message: e.to_string(),
            }
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AegisError::Provider {
            provider: provider_id.to_string(),
            status: status.as_u16(),
            message: super::extract_provider_error(&body),
        });
    }

    let mut decoder = SseDecoder::new();
    let byte_stream = response.bytes_stream();

    let chunk_stream = byte_stream.flat_map(move |result| {
        let chunks = match result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                decoder
                    .push(&text)
                    .into_iter()
                    .filter_map(|payload| parse(&payload).transpose())
                    .collect::<Vec<_>>()
            }
            Err(e) => vec![Err(AegisError::Provider {
                provider: provider_id.to_string(),
                status: 502,
                message: format!("stream interrupted: {e}"),
            })],
        };
        futures::stream::iter(chunks)
    });

    Ok(Box::pin(chunk_stream))
}

/// Models served directly by OpenAI.
const OPENAI_MODELS: &[&str] = &[
    "gpt-4o-mini",
    "gpt-4o",
    "gpt-3.5-turbo",
    "gpt-4-turbo",
    "gpt-4",
    "o3-mini",
    "o1-mini",
    "o1",
    "gpt-4.1-mini",
    "gpt-4.1",
    "gpt-5-mini",
    "gpt-5",
    "gpt-5-nano",
    "gpt-4.1-nano",
    "text-embedding-3-small",
    "text-embedding-3-large",
];

/// The OpenAI adapter.
pub struct OpenAiProvider;

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn supported_models(&self) -> &[&'static str] {
        OPENAI_MODELS
    }

    fn default_base_url(&self) -> &'static str {
        "https://api.openai.com/v1"
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
        open_stream(
            http,
            &url,
            body,
            super::with_idempotency_key(self.auth_headers(credential), idempotency_key),
            "openai",
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
    fn cached_tokens_are_subtracted_from_the_prompt_total() {
        // OpenAI reports `prompt_tokens` with the cached portion already inside it. Billing
        // the whole figure at the full input rate -- which this adapter did before the
        // enterprise readiness audit -- over-charges, because OpenAI discounts the cached
        // part to roughly a quarter. Note this is the exact opposite of Anthropic's bias
        // in the same code, which is why normalising here rather than in the pricing layer
        // is what makes one pricing rule correct for both.
        let body = serde_json::json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"},
                         "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 10_000,
                "completion_tokens": 20,
                "prompt_tokens_details": {"cached_tokens": 9_000}
            }
        });

        let usage = parse_response(&body).unwrap().usage;
        assert_eq!(usage.input_tokens, 1_000, "the uncached remainder");
        assert_eq!(usage.cached_input_tokens, 9_000);
        assert_eq!(
            usage.total_input(),
            10_000,
            "the parts must still sum to what the provider reported"
        );
    }

    #[test]
    fn a_cached_count_larger_than_the_prompt_cannot_wrap() {
        // Defensive: an OpenAI-compatible provider reporting nonsense must not produce an
        // enormous token count through unsigned wraparound, which would produce an
        // enormous invoice line from a single malformed response.
        let body = serde_json::json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"},
                         "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 5,
                "prompt_tokens_details": {"cached_tokens": 999_999}
            }
        });

        let usage = parse_response(&body).unwrap().usage;
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(
            usage.cached_input_tokens, 100,
            "clamped to the prompt total"
        );
        assert_eq!(usage.total_input(), 100);
    }
    use crate::types::{Message, Role};

    fn request() -> NormalizedRequest {
        NormalizedRequest {
            temperature: Some(0.2),
            max_tokens: Some(256),
            ..NormalizedRequest::simple("gpt-4o", "What is 2+2?")
        }
    }

    #[test]
    fn body_carries_the_served_model_not_the_requested_one() {
        // The core routing invariant: we send the model we chose, while the request keeps
        // the original for baseline pricing.
        let req = request();
        let body = build_body(&req, "gpt-4o-mini");
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(req.model, "gpt-4o", "the request must not be mutated");
    }

    #[test]
    fn body_includes_supplied_parameters_only() {
        let body = build_body(&request(), "gpt-4o");
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["max_tokens"], 256);
        // Never send a parameter the caller did not set — defaults differ per provider.
        assert!(body.get("top_p").is_none());
        assert!(body.get("stop").is_none());
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn streaming_requests_ask_for_usage() {
        // Without stream_options.include_usage we cannot bill a streamed request from
        // reported tokens and would have to estimate every one.
        let req = NormalizedRequest {
            stream: true,
            ..request()
        };
        let body = build_body(&req, "gpt-4o");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn unknown_parameters_pass_through() {
        let mut req = request();
        req.extra.insert("seed".into(), serde_json::json!(42));
        req.extra
            .insert("presence_penalty".into(), serde_json::json!(0.5));
        let body = build_body(&req, "gpt-4o");
        assert_eq!(body["seed"], 42);
        assert_eq!(body["presence_penalty"], 0.5);
    }

    #[test]
    fn explicit_parameters_win_over_extras() {
        // If a caller somehow supplies both, the modelled field is authoritative.
        let mut req = request();
        req.extra
            .insert("temperature".into(), serde_json::json!(0.99));
        let body = build_body(&req, "gpt-4o");
        assert_eq!(body["temperature"], 0.2);
    }

    #[test]
    fn messages_serialize_in_openai_shape() {
        let req = NormalizedRequest {
            messages: vec![
                Message::text(Role::System, "Be terse."),
                Message::text(Role::User, "Hi"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let body = build_body(&req, "gpt-4o");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "Be terse.");
        assert_eq!(body["messages"][1]["role"], "user");
        // Absent optional fields must not be emitted as nulls.
        assert!(body["messages"][0].get("name").is_none());
        assert!(body["messages"][0].get("tool_calls").is_none());
    }

    #[test]
    fn tools_are_forwarded_with_their_choice() {
        let mut req = request();
        req.tools = vec![serde_json::json!({"type": "function", "function": {"name": "f"}})];
        req.tool_choice = Some(serde_json::json!("auto"));
        let body = build_body(&req, "gpt-4o");
        assert_eq!(body["tools"][0]["function"]["name"], "f");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn response_parsing_extracts_content_and_usage() {
        let body = serde_json::json!({
            "id": "chatcmpl-123",
            "model": "gpt-4o-mini-2024-07-18",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "4"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 1, "total_tokens": 13}
        });
        let parsed = parse_response(&body).unwrap();
        assert_eq!(parsed.id, "chatcmpl-123");
        assert_eq!(parsed.content, "4");
        assert_eq!(parsed.model, "gpt-4o-mini-2024-07-18");
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.usage.input_tokens, 12);
        assert_eq!(parsed.usage.output_tokens, 1);
        assert!(!parsed.usage.estimated);
    }

    #[test]
    fn missing_usage_is_flagged_as_estimated() {
        // Compatible providers sometimes omit usage. We must mark it, never invent it.
        let body = serde_json::json!({
            "id": "x",
            "model": "some-model",
            "choices": [{"message": {"content": "hi"}, "finish_reason": "stop"}]
        });
        let parsed = parse_response(&body).unwrap();
        assert!(parsed.usage.estimated);
        assert_eq!(parsed.usage.input_tokens, 0);
    }

    #[test]
    fn tool_call_responses_are_preserved() {
        let body = serde_json::json!({
            "id": "x", "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{"id": "call_1", "type": "function",
                                    "function": {"name": "get_weather", "arguments": "{}"}}]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 5}
        });
        let parsed = parse_response(&body).unwrap();
        assert_eq!(parsed.content, "", "null content must not panic");
        assert_eq!(
            parsed.tool_calls.as_ref().unwrap()[0]["function"]["name"],
            "get_weather"
        );
        assert_eq!(parsed.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn responses_without_choices_are_rejected() {
        let body = serde_json::json!({"id": "x", "model": "m", "choices": []});
        assert!(parse_response(&body).is_err());
    }

    #[test]
    fn raw_body_is_retained_for_faithful_passthrough() {
        let body = serde_json::json!({
            "id": "x", "model": "m",
            "choices": [{"message": {"content": "hi"}, "finish_reason": "stop"}],
            "system_fingerprint": "fp_abc"
        });
        let parsed = parse_response(&body).unwrap();
        // Clients depending on fields we do not model must still receive them.
        assert_eq!(parsed.raw.unwrap()["system_fingerprint"], "fp_abc");
    }

    #[test]
    fn stream_chunks_decode_content_deltas() {
        let chunk = parse_stream_chunk(r#"{"choices":[{"delta":{"content":"Hello"},"index":0}]}"#)
            .unwrap()
            .unwrap();
        assert_eq!(chunk.delta, "Hello");
        assert!(chunk.finish_reason.is_none());
    }

    #[test]
    fn done_sentinel_and_empty_payloads_yield_nothing() {
        assert!(parse_stream_chunk("[DONE]").unwrap().is_none());
        assert!(parse_stream_chunk("").unwrap().is_none());
    }

    #[test]
    fn malformed_chunks_are_skipped_rather_than_killing_the_stream() {
        // The client has already received tokens; dropping the connection loses them.
        assert!(parse_stream_chunk("{not json").unwrap().is_none());
        assert!(parse_stream_chunk("null").unwrap().is_none());
    }

    #[test]
    fn role_only_first_chunk_is_ignored() {
        // OpenAI opens every stream with a delta carrying only the role.
        assert!(
            parse_stream_chunk(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_streamed_tool_call_chunk_is_not_silently_dropped() {
        // Streamed function calling looks exactly like this on the wire: `content` is
        // absent from every one of these chunks, only `tool_calls` carries data. Before
        // the fix, this chunk was indistinguishable from the harmless role-only opener
        // above and was dropped the same way — the client's tool call simply never
        // arrived, no error raised anywhere.
        let chunk = parse_stream_chunk(
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc123","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
        )
        .unwrap()
        .expect("a tool-call delta must be kept, not treated as content-free");

        // The forwarding loop in routes/openai_compat.rs prefers `raw` verbatim over
        // reconstructing from `delta` *when the caller connected via the OpenAI-shaped
        // endpoint*, specifically so a shape this module doesn't model explicitly reaches
        // that client byte-for-byte. It also has to reconstruct correctly when the
        // caller connected via /v1/messages instead, which is what the structured
        // `tool_call` field below exists for.
        let raw = chunk
            .raw
            .clone()
            .expect("raw payload must be preserved for passthrough");
        assert!(raw.contains("tool_calls"));
        assert!(raw.contains("get_weather"));
        assert_eq!(chunk.source_shape, Some(WireShape::OpenAiCompatible));

        let tc = chunk
            .tool_call
            .expect("must carry a normalised ToolCallDelta too");
        assert_eq!(tc.index, 0);
        assert_eq!(tc.id.as_deref(), Some("call_abc123"));
        assert_eq!(tc.name.as_deref(), Some("get_weather"));
        assert_eq!(tc.arguments_fragment, "");
    }

    #[test]
    fn a_streamed_tool_call_argument_fragment_is_also_kept() {
        // Arguments arrive as a stream of partial JSON string fragments across many
        // chunks, each with an empty function.arguments except for that one fragment —
        // exactly the shape that looked content-free under the old check.
        let chunk = parse_stream_chunk(
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"loc"}}]}}]}"#,
        )
        .unwrap()
        .expect("an argument-fragment chunk must be kept");
        assert!(chunk.raw.unwrap().contains("loc"));
    }

    #[test]
    fn final_chunk_usage_is_captured_for_billing() {
        let chunk = parse_stream_chunk(
            r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":50}}"#,
        )
        .unwrap()
        .unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert!(!usage.estimated);
    }

    #[test]
    fn finish_reason_chunk_is_emitted() {
        let chunk = parse_stream_chunk(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#)
            .unwrap()
            .unwrap();
        assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
        assert_eq!(chunk.delta, "");
    }

    #[test]
    fn auth_header_uses_bearer_scheme() {
        let headers = OpenAiProvider.auth_headers(&Credential::new("sk-test-key"));
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "authorization");
        assert_eq!(headers[0].1, "Bearer sk-test-key");
    }

    #[test]
    fn adapter_metadata_is_correct() {
        let provider = OpenAiProvider;
        assert_eq!(provider.id(), "openai");
        assert_eq!(provider.default_base_url(), "https://api.openai.com/v1");
        assert_eq!(provider.chat_path("gpt-4o"), "/chat/completions");
        assert!(provider.supports("gpt-4o"));
        assert!(!provider.supports("claude-opus-4-5"));
    }
}

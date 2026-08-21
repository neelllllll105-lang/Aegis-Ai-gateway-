//! Anthropic Messages API adapter.
//!
//! Anthropic's wire format differs from OpenAI's in four ways that each cause a real bug
//! if missed, so each is handled explicitly below:
//!
//! 1. The system prompt is a **top-level `system` field**, not a message with
//!    `role: "system"`. Leaving it in the array is rejected outright.
//! 2. `max_tokens` is **required**. A request that omits it — perfectly legal against
//!    OpenAI — fails with a 400 unless we supply a default.
//! 3. Response content is an **array of typed blocks**, not a string.
//! 4. Streaming uses **named event types** (`content_block_delta`, `message_delta`)
//!    rather than OpenAI's uniform choice deltas, and usage arrives split across the
//!    opening and closing events.

use super::sse::is_done;
use super::{ChunkStream, Credential, Provider};
use crate::error::{AegisError, Result};
use crate::types::{Message, NormalizedRequest, NormalizedResponse, Role, StreamChunk, TokenUsage};
use async_trait::async_trait;
use std::time::Duration;

/// API version header value. Anthropic requires it on every request.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fallback for `max_tokens` when the caller does not set one.
///
/// Anthropic rejects requests without it. 4096 is generous enough not to truncate normal
/// completions and small enough not to invite runaway cost on a caller's behalf.
pub const DEFAULT_MAX_TOKENS: u32 = 4_096;

/// Models served by Anthropic.
const ANTHROPIC_MODELS: &[&str] = &[
    "claude-opus-4-5",
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
    "claude-opus-4-1",
    "claude-3-5-haiku",
];

/// Build an Anthropic Messages request body.
pub fn build_body(request: &NormalizedRequest, model: &str) -> serde_json::Value {
    // System and developer messages are hoisted out of the array into `system`.
    let system_text = request.system_text();
    let conversation: Vec<&Message> = request
        .messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System | Role::Developer))
        .collect();

    let messages: Vec<serde_json::Value> = conversation
        .iter()
        .map(|m| {
            serde_json::json!({
                // Anthropic accepts only user and assistant roles; tool results arrive
                // as user turns.
                "role": match m.role {
                    Role::Assistant => "assistant",
                    _ => "user",
                },
                "content": m.text_content(),
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
    });

    let map = body.as_object_mut().expect("constructed as an object");

    if !system_text.is_empty() {
        map.insert("system".into(), serde_json::json!(system_text));
    }
    if let Some(temperature) = request.temperature {
        map.insert("temperature".into(), serde_json::json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        map.insert("top_p".into(), serde_json::json!(top_p));
    }
    if !request.tools.is_empty() {
        map.insert(
            "tools".into(),
            serde_json::json!(translate_tools(&request.tools)),
        );
    }
    if let Some(stop) = &request.stop {
        // Anthropic calls this stop_sequences and requires an array.
        let sequences = match stop {
            serde_json::Value::String(s) => serde_json::json!([s]),
            other => other.clone(),
        };
        map.insert("stop_sequences".into(), sequences);
    }
    if request.stream {
        map.insert("stream".into(), serde_json::json!(true));
    }

    body
}

/// Translate OpenAI-style tool definitions into Anthropic's shape.
///
/// OpenAI nests the definition under `function` with `parameters`; Anthropic flattens it
/// and calls the schema `input_schema`.
fn translate_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            if let Some(function) = tool.get("function") {
                serde_json::json!({
                    "name": function.get("name").cloned().unwrap_or_default(),
                    "description": function.get("description").cloned().unwrap_or_default(),
                    "input_schema": function
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({"type": "object"})),
                })
            } else {
                // Already in Anthropic shape.
                tool.clone()
            }
        })
        .collect()
}

/// Parse an Anthropic Messages response.
pub fn parse_response(body: &serde_json::Value) -> Result<NormalizedResponse> {
    let blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| malformed("response contained no content blocks"))?;

    // Concatenate every text block; ignore thinking and other block types for the
    // normalised text view (the raw body is preserved for passthrough).
    let content = blocks
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("");

    let tool_calls = blocks
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        .then(|| {
            serde_json::Value::Array(
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
                    .cloned()
                    .collect(),
            )
        });

    let usage = body
        .get("usage")
        .map(|u| TokenUsage {
            input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            estimated: false,
        })
        .unwrap_or(TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            estimated: true,
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
            .unwrap_or("")
            .to_string(),
        content,
        finish_reason: body
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        tool_calls,
        usage,
        raw: Some(body.clone()),
    })
}

/// Parse one Anthropic SSE payload.
pub fn parse_stream_chunk(data: &str) -> Result<Option<StreamChunk>> {
    if data.is_empty() || is_done(data) {
        return Ok(None);
    }
    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
        return Ok(None);
    };

    match json.get("type").and_then(|t| t.as_str()) {
        Some("content_block_delta") => {
            let delta = json
                .pointer("/delta/text")
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string();
            if delta.is_empty() {
                return Ok(None);
            }
            Ok(Some(StreamChunk {
                delta,
                finish_reason: None,
                usage: None,
                raw: Some(data.to_string()),
            }))
        }
        // The closing event carries the stop reason and the output token count. Input
        // tokens arrived back on message_start, so the caller stitches them together.
        Some("message_delta") => Ok(Some(StreamChunk {
            delta: String::new(),
            finish_reason: json
                .pointer("/delta/stop_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            usage: json.get("usage").map(|u| TokenUsage {
                input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                estimated: false,
            }),
            raw: Some(data.to_string()),
        })),
        Some("message_start") => {
            // Input tokens are only ever reported here.
            let usage = json.pointer("/message/usage").map(|u| TokenUsage {
                input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                estimated: false,
            });
            Ok(usage.map(|usage| StreamChunk {
                delta: String::new(),
                finish_reason: None,
                usage: Some(usage),
                raw: Some(data.to_string()),
            }))
        }
        _ => Ok(None),
    }
}

fn malformed(message: &str) -> AegisError {
    AegisError::Provider {
        provider: "anthropic".to_string(),
        status: 502,
        message: message.to_string(),
    }
}

/// The Anthropic adapter.
pub struct AnthropicProvider;

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn supported_models(&self) -> &[&'static str] {
        ANTHROPIC_MODELS
    }

    fn default_base_url(&self) -> &'static str {
        "https://api.anthropic.com/v1"
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
        "/messages".to_string()
    }

    fn auth_headers(&self, credential: &Credential) -> Vec<(String, String)> {
        vec![
            ("x-api-key".to_string(), credential.api_key.clone()),
            (
                "anthropic-version".to_string(),
                ANTHROPIC_VERSION.to_string(),
            ),
        ]
    }

    async fn chat_stream(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
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
        super::openai::open_stream(
            http,
            &url,
            body,
            self.auth_headers(credential),
            "anthropic",
            timeout,
            parse_stream_chunk,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    fn request_with_system() -> NormalizedRequest {
        NormalizedRequest {
            messages: vec![
                Message::text(Role::System, "You are terse."),
                Message::text(Role::User, "Hello"),
                Message::text(Role::Assistant, "Hi."),
                Message::text(Role::User, "What is 2+2?"),
            ],
            temperature: Some(0.3),
            ..NormalizedRequest::simple("claude-sonnet-4-5", "")
        }
    }

    #[test]
    fn system_prompt_is_hoisted_out_of_the_message_array() {
        // Anthropic rejects a system-role message outright, so this is a hard requirement.
        let body = build_body(&request_with_system(), "claude-sonnet-4-5");
        assert_eq!(body["system"], "You are terse.");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert!(
            messages.iter().all(|m| m["role"] != "system"),
            "a system role leaked into the message array"
        );
    }

    #[test]
    fn max_tokens_is_always_present() {
        // Required by Anthropic. Omitting it turns a valid OpenAI-shaped request into a
        // 400 the caller never made a mistake to earn.
        let mut req = request_with_system();
        req.max_tokens = None;
        let body = build_body(&req, "claude-sonnet-4-5");
        assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS);

        req.max_tokens = Some(100);
        assert_eq!(build_body(&req, "claude-sonnet-4-5")["max_tokens"], 100);
    }

    #[test]
    fn only_user_and_assistant_roles_are_emitted() {
        let req = NormalizedRequest {
            messages: vec![
                Message::text(Role::User, "q"),
                Message::text(Role::Assistant, "a"),
                Message::text(Role::Tool, "tool result"),
            ],
            ..NormalizedRequest::simple("claude-sonnet-4-5", "")
        };
        let body = build_body(&req, "claude-sonnet-4-5");
        for message in body["messages"].as_array().unwrap() {
            let role = message["role"].as_str().unwrap();
            assert!(
                role == "user" || role == "assistant",
                "illegal role: {role}"
            );
        }
    }

    #[test]
    fn requests_with_no_system_prompt_omit_the_field() {
        let req = NormalizedRequest::simple("claude-sonnet-4-5", "hi");
        let body = build_body(&req, "claude-sonnet-4-5");
        assert!(
            body.get("system").is_none(),
            "empty system must be omitted, not sent as ''"
        );
    }

    #[test]
    fn stop_string_is_promoted_to_an_array() {
        let mut req = request_with_system();
        req.stop = Some(serde_json::json!("END"));
        let body = build_body(&req, "claude-sonnet-4-5");
        assert_eq!(body["stop_sequences"], serde_json::json!(["END"]));

        req.stop = Some(serde_json::json!(["A", "B"]));
        assert_eq!(
            build_body(&req, "claude-sonnet-4-5")["stop_sequences"],
            serde_json::json!(["A", "B"])
        );
    }

    #[test]
    fn openai_tool_definitions_are_translated() {
        let mut req = request_with_system();
        req.tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
            }
        })];
        let body = build_body(&req, "claude-sonnet-4-5");
        let tool = &body["tools"][0];
        assert_eq!(tool["name"], "get_weather");
        assert_eq!(tool["description"], "Get weather");
        assert_eq!(tool["input_schema"]["properties"]["city"]["type"], "string");
        assert!(
            tool.get("function").is_none(),
            "OpenAI nesting must be flattened"
        );
    }

    #[test]
    fn native_anthropic_tools_pass_through_unchanged() {
        let mut req = request_with_system();
        req.tools = vec![serde_json::json!({
            "name": "native", "input_schema": {"type": "object"}
        })];
        let body = build_body(&req, "claude-sonnet-4-5");
        assert_eq!(body["tools"][0]["name"], "native");
    }

    #[test]
    fn response_content_blocks_are_concatenated() {
        let body = serde_json::json!({
            "id": "msg_01",
            "model": "claude-sonnet-4-5",
            "content": [
                {"type": "text", "text": "The answer "},
                {"type": "text", "text": "is 4."}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 15, "output_tokens": 8}
        });
        let parsed = parse_response(&body).unwrap();
        assert_eq!(parsed.content, "The answer is 4.");
        assert_eq!(parsed.id, "msg_01");
        assert_eq!(parsed.finish_reason.as_deref(), Some("end_turn"));
        assert_eq!(parsed.usage.input_tokens, 15);
        assert_eq!(parsed.usage.output_tokens, 8);
        assert!(!parsed.usage.estimated);
    }

    #[test]
    fn non_text_blocks_are_excluded_from_the_text_view() {
        let body = serde_json::json!({
            "id": "msg_02", "model": "claude-opus-4-5",
            "content": [
                {"type": "thinking", "thinking": "internal reasoning"},
                {"type": "text", "text": "visible answer"}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        });
        let parsed = parse_response(&body).unwrap();
        assert_eq!(parsed.content, "visible answer");
        assert!(!parsed.content.contains("internal reasoning"));
    }

    #[test]
    fn tool_use_blocks_are_surfaced() {
        let body = serde_json::json!({
            "id": "msg_03", "model": "claude-sonnet-4-5",
            "content": [{
                "type": "tool_use", "id": "toolu_1", "name": "get_weather",
                "input": {"city": "Paris"}
            }],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 30, "output_tokens": 12}
        });
        let parsed = parse_response(&body).unwrap();
        assert_eq!(
            parsed.tool_calls.as_ref().unwrap()[0]["name"],
            "get_weather"
        );
        assert_eq!(parsed.content, "");
    }

    #[test]
    fn malformed_responses_are_rejected() {
        assert!(parse_response(&serde_json::json!({"id": "x"})).is_err());
    }

    #[test]
    fn streaming_input_tokens_come_from_message_start() {
        let chunk = parse_stream_chunk(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":42,"output_tokens":0}}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(chunk.usage.unwrap().input_tokens, 42);
        assert_eq!(chunk.delta, "");
    }

    #[test]
    fn streaming_text_deltas_decode() {
        let chunk = parse_stream_chunk(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(chunk.delta, "Hello");
    }

    #[test]
    fn streaming_output_tokens_come_from_message_delta() {
        let chunk = parse_stream_chunk(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":25}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(chunk.finish_reason.as_deref(), Some("end_turn"));
        assert_eq!(chunk.usage.unwrap().output_tokens, 25);
    }

    #[test]
    fn structural_stream_events_produce_nothing() {
        for event in [
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_start","index":0}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ] {
            assert!(parse_stream_chunk(event).unwrap().is_none(), "{event}");
        }
    }

    #[test]
    fn a_full_streamed_conversation_reconstructs_the_text_and_usage() {
        // End to end over a realistic event sequence: the concatenated deltas must equal
        // the intended message, and both token counts must be recoverable.
        let events = [
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#,
            r#"{"type":"content_block_start","index":0}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"2"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"text":" + "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"2 = 4"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
            r#"{"type":"message_stop"}"#,
        ];

        let mut text = String::new();
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut finish = None;
        for event in events {
            if let Some(chunk) = parse_stream_chunk(event).unwrap() {
                text.push_str(&chunk.delta);
                if let Some(usage) = chunk.usage {
                    if usage.input_tokens > 0 {
                        input_tokens = usage.input_tokens;
                    }
                    if usage.output_tokens > 0 {
                        output_tokens = usage.output_tokens;
                    }
                }
                if chunk.finish_reason.is_some() {
                    finish = chunk.finish_reason;
                }
            }
        }
        assert_eq!(text, "2 + 2 = 4");
        assert_eq!(input_tokens, 10);
        assert_eq!(output_tokens, 7);
        assert_eq!(finish.as_deref(), Some("end_turn"));
    }

    #[test]
    fn auth_uses_x_api_key_and_a_version_header() {
        let headers = AnthropicProvider.auth_headers(&Credential::new("sk-ant-test"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-ant-test"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "anthropic-version" && v == ANTHROPIC_VERSION));
        // Anthropic does not use bearer auth; sending one silently fails to authenticate.
        assert!(!headers.iter().any(|(k, _)| k == "authorization"));
    }

    #[test]
    fn adapter_metadata_is_correct() {
        let provider = AnthropicProvider;
        assert_eq!(provider.id(), "anthropic");
        assert_eq!(provider.chat_path("claude-sonnet-4-5"), "/messages");
        assert!(provider.supports("claude-sonnet-4-5"));
        assert!(provider.supports("anthropic/claude-opus-4-5"));
        assert!(!provider.supports("gpt-4o"));
    }
}

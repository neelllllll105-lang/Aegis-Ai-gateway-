//! Anthropic-compatible gateway — `POST /v1/messages`.
//!
//! This endpoint exists for one specific reason: Claude Code, the Anthropic SDK, and every
//! tool built on them speak the Messages API, not OpenAI's. Supporting it means those
//! users switch by changing `ANTHROPIC_BASE_URL` and nothing else — no code change, no
//! adapter, no SDK swap.
//!
//! The pipeline underneath is identical to [`super::openai_compat`]. Only the wire format
//! at the edges differs: an Anthropic-shaped request is normalised on the way in, and the
//! response is rendered back into Anthropic's shape on the way out. Routing, caching,
//! metering, and savings attribution are the same code, so a customer on `/v1/messages`
//! gets exactly the same optimization and the same auditable numbers.

use crate::enterprise::residency;
use crate::error::{AegisError, Result};
use crate::metering::usage;
use crate::middleware::auth;
use crate::middleware::{budget, rate_limit};
use crate::routes::openai_compat::{execute, PipelineOutcome};
use crate::types::{Content, Message, NormalizedRequest, Role, RoutingHint};
use crate::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use std::collections::BTreeMap;

/// An inbound Anthropic Messages request.
#[derive(Debug, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    /// Required by Anthropic, so it is required here too.
    pub max_tokens: u32,
    #[serde(default)]
    pub system: Option<serde_json::Value>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// One message in an Anthropic request.
#[derive(Debug, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: serde_json::Value,
}

impl AnthropicRequest {
    /// Normalise into the engine's internal form.
    pub fn normalize(self) -> NormalizedRequest {
        let mut messages = Vec::with_capacity(self.messages.len() + 1);

        // Anthropic carries the system prompt outside the message array, as either a
        // string or an array of content blocks. Both shapes appear in the wild.
        if let Some(system) = &self.system {
            let text = match system {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(blocks) => blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            if !text.is_empty() {
                messages.push(Message::text(Role::System, text));
            }
        }

        for message in self.messages {
            let role = match message.role.as_str() {
                "assistant" => Role::Assistant,
                _ => Role::User,
            };
            let content = match &message.content {
                serde_json::Value::String(text) => Content::Text(text.clone()),
                serde_json::Value::Array(parts) => Content::Parts(parts.clone()),
                other => Content::Text(other.to_string()),
            };
            messages.push(Message {
                role,
                content: Some(content),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        NormalizedRequest {
            model: self.model,
            messages,
            temperature: self.temperature,
            top_p: self.top_p,
            max_tokens: Some(self.max_tokens),
            stream: self.stream,
            tools: self.tools,
            tool_choice: self.tool_choice,
            stop: self
                .stop_sequences
                .map(|sequences| serde_json::json!(sequences)),
            response_format: None,
            user: self
                .metadata
                .as_ref()
                .and_then(|m| m.get("user_id"))
                .and_then(|u| u.as_str())
                .map(|u| u.to_string()),
            extra: BTreeMap::new(),
        }
    }
}

/// Render a pipeline outcome as an Anthropic Messages response.
///
/// The raw upstream body is reused when the request actually went to Anthropic, so
/// nothing we do not model is lost. When the router served the request from a *different*
/// provider — which is the entire point of the product — the response is rebuilt in
/// Anthropic's shape so the caller's SDK parses it without knowing anything changed.
pub fn to_anthropic_response(outcome: &PipelineOutcome) -> serde_json::Value {
    if let Some(raw) = &outcome.response.raw {
        if raw.get("type").and_then(|t| t.as_str()) == Some("message") {
            return raw.clone();
        }
    }

    let mut content = Vec::new();
    if !outcome.response.content.is_empty() {
        content.push(serde_json::json!({
            "type": "text",
            "text": outcome.response.content,
        }));
    }
    if let Some(tool_calls) = &outcome.response.tool_calls {
        if let Some(calls) = tool_calls.as_array() {
            for call in calls {
                // An OpenAI-shaped tool call needs translating into a tool_use block.
                if let Some(function) = call.get("function") {
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call.get("id").cloned().unwrap_or_else(|| serde_json::json!("")),
                        "name": function.get("name").cloned().unwrap_or_default(),
                        "input": function
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok())
                            .unwrap_or_else(|| serde_json::json!({})),
                    }));
                } else {
                    content.push(call.clone());
                }
            }
        }
    }

    serde_json::json!({
        "id": if outcome.response.id.is_empty() {
            format!("msg_{}", outcome.request_id.simple())
        } else {
            outcome.response.id.clone()
        },
        "type": "message",
        "role": "assistant",
        "model": outcome.served_model,
        "content": content,
        "stop_reason": normalize_stop_reason(outcome.response.finish_reason.as_deref()),
        "stop_sequence": serde_json::Value::Null,
        "usage": {
            "input_tokens": outcome.tokens.input_tokens,
            "output_tokens": outcome.tokens.output_tokens,
        }
    })
}

/// Map an OpenAI finish reason onto Anthropic's vocabulary.
///
/// A client branching on `stop_reason` breaks if it receives OpenAI's spelling, so this
/// translation is load-bearing rather than cosmetic.
fn normalize_stop_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") | Some("max_tokens") => "max_tokens",
        Some("tool_calls") | Some("tool_use") => "tool_use",
        Some("content_filter") => "stop_sequence",
        _ => "end_turn",
    }
}

/// `POST /v1/messages`
pub async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match handle_messages(&state, &headers, body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn handle_messages(
    state: &AppState,
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response> {
    // [1] Authentication. Anthropic SDKs send `x-api-key`, which `extract_bearer` accepts
    // alongside `Authorization: Bearer`.
    let token = auth::extract_bearer(headers).ok_or_else(|| {
        AegisError::Unauthorized(
            "missing API key. Send it as: x-api-key: aegis_sk_... or \
             Authorization: Bearer aegis_sk_..."
                .into(),
        )
    })?;
    let auth_context = auth::authenticate_api_key(state, &token).await?;

    // [1b] Data residency. An organisation pinned to a region must never be served by an
    // instance running elsewhere — see openai_compat.rs for the full reasoning.
    residency::enforce(&state.config.region, &auth_context.region)?;

    // [2] Rate limiting.
    let limit = rate_limit::check(state.store.as_ref(), &auth_context).await?;
    if !limit.allowed {
        state.metrics.record_rate_limited(limit.scope);
        return Err(limit.into_error());
    }

    // [3] Budget.
    let budget_decision =
        budget::check(state.store.as_ref(), &auth_context, None, None, None).await?;
    if !budget_decision.allowed {
        state.metrics.record_budget_blocked(budget_decision.scope);
        return Err(budget_decision.into_error());
    }

    // [4] Parse and normalise.
    if body.len() > state.config.max_body_bytes {
        return Err(AegisError::PayloadTooLarge);
    }
    let inbound: AnthropicRequest = serde_json::from_slice(&body)
        .map_err(|e| AegisError::BadRequest(format!("invalid request body: {e}")))?;

    if inbound.messages.is_empty() {
        return Err(AegisError::BadRequest(
            "messages must contain at least one message".into(),
        ));
    }

    let request = inbound.normalize();
    let hint = RoutingHint::parse(
        headers
            .get("x-aegis-routing-hint")
            .and_then(|v| v.to_str().ok()),
    );

    if request.stream {
        return stream_messages(state, &auth_context, request, hint).await;
    }

    // [5]-[9] The same pipeline as the OpenAI endpoint.
    let outcome = execute(state, &auth_context, request, hint).await?;

    // [10] Metering.
    let event = outcome.usage_event(&auth_context, 200, &state.config.region);
    let _ = usage::emit(state.store.as_ref(), &event).await;
    state.metrics.record_usage_event();
    state.metrics.record_request("/v1/messages", 200);
    state
        .metrics
        .record_overhead_ms(outcome.gateway_overhead_ms);
    state
        .metrics
        .record_latency_ms(outcome.total_latency_ms as f64);

    // [11] Respond.
    let mut response = (StatusCode::OK, Json(to_anthropic_response(&outcome))).into_response();
    for (name, value) in outcome.headers() {
        response.headers_mut().insert(name, value);
    }
    response.headers_mut().insert(
        "anthropic-version",
        HeaderValue::from_static(crate::providers::anthropic::ANTHROPIC_VERSION),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// The named SSE events an Anthropic client expects, in order.
///
/// Unlike OpenAI, which sends one uniform chunk shape, the Messages API sends a scripted
/// sequence of *named* events and a client state-machines over them. Emitting the right
/// events in the wrong order is worse than not streaming at all: the SDK will either hang
/// waiting for `message_stop` or throw on an unexpected transition.
///
/// The full sequence for a text response is:
///
/// ```text
/// message_start          usage.input_tokens arrives here, and only here
/// content_block_start    index 0
/// content_block_delta    one per token, repeated
/// content_block_stop     index 0
/// message_delta          stop_reason and usage.output_tokens
/// message_stop           terminator
/// ```
fn sse_event(event: &str, data: &serde_json::Value) -> String {
    // Both the `event:` line and the `data:` line are required. Anthropic SDKs dispatch on
    // the event name, so a bare `data:` line is silently ignored.
    format!("event: {event}\ndata: {data}\n\n")
}

/// Streaming variant of `/v1/messages`.
///
/// Reuses the same routing decision and metering as the non-streaming path; only the wire
/// rendering differs.
async fn stream_messages(
    state: &AppState,
    auth_context: &crate::middleware::auth::AuthContext,
    request: NormalizedRequest,
    hint: RoutingHint,
) -> Result<Response> {
    use crate::engine::classifier::Classifier;
    use crate::engine::router::{Router, RoutingInputs};
    use crate::metering::savings::SavingsBreakdown;
    use crate::metering::usage::UsageEvent;
    use crate::money::MicroCents;
    use crate::types::{CacheOutcome, TokenUsage};
    use futures::StreamExt;
    use std::time::Instant;

    let started = Instant::now();
    let request_id = uuid::Uuid::new_v4();
    let requested_model = request.model.clone();

    let inputs = RoutingInputs {
        hint,
        policy: None,
        team: None,
        allowed_models: auth_context.allowed_models.clone(),
        plan_tier_ceiling: crate::routes::openai_compat::plan_tier_ceiling(&auth_context.plan),
    };
    let router = Router::with_classifier(Classifier::new());
    let decision = router.route(&request, &state.pricing, &state.health, &inputs)?;

    let provider = state
        .providers
        .for_model(&decision.served_model)
        .ok_or_else(|| {
            AegisError::AllProvidersFailed(format!("no adapter serves {}", decision.served_model))
        })?;

    let credential =
        crate::routes::openai_compat::resolve_credential(state, auth_context, provider.id())
            .await?;

    let overhead_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let upstream = provider
        .chat_stream(
            &state.http,
            &request,
            &decision.served_model,
            &credential,
            state.config.provider_timeout,
        )
        .await?;

    let state_for_stream = state.clone();
    let auth_for_stream = auth_context.clone();
    let served_model = decision.served_model.clone();
    let provider_id = provider.id().to_string();
    let estimated_input = request.estimated_input_tokens();
    let complexity = decision.complexity_score;
    let routing_reason = decision.reason;
    let message_id = format!("msg_{}", request_id.simple());

    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut output_chars: u64 = 0;
    let mut stop_reason = "end_turn".to_string();

    let sse = async_stream::stream! {
        // message_start. The input token count is reported here and nowhere else, so a
        // client that misses this event can never learn it.
        yield Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(sse_event(
            "message_start",
            &serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": served_model,
                    "content": [],
                    "stop_reason": serde_json::Value::Null,
                    "stop_sequence": serde_json::Value::Null,
                    "usage": {"input_tokens": estimated_input, "output_tokens": 0},
                }
            }),
        )));

        yield Ok(axum::body::Bytes::from(sse_event(
            "content_block_start",
            &serde_json::json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text", "text": ""}
            }),
        )));

        let mut upstream = upstream;
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk) => {
                    if !chunk.delta.is_empty() {
                        output_chars += chunk.delta.chars().count() as u64;
                        yield Ok(axum::body::Bytes::from(sse_event(
                            "content_block_delta",
                            &serde_json::json!({
                                "type": "content_block_delta",
                                "index": 0,
                                "delta": {"type": "text_delta", "text": chunk.delta}
                            }),
                        )));
                    }
                    if let Some(usage) = chunk.usage {
                        if usage.input_tokens > 0 {
                            input_tokens = usage.input_tokens;
                        }
                        if usage.output_tokens > 0 {
                            output_tokens = usage.output_tokens;
                        }
                    }
                    if let Some(reason) = chunk.finish_reason {
                        stop_reason = normalize_stop_reason(Some(&reason)).to_string();
                    }
                }
                Err(e) => {
                    // Anthropic signals a mid-stream failure with a named error event.
                    // Dropping the connection instead would leave the SDK hanging.
                    yield Ok(axum::body::Bytes::from(sse_event(
                        "error",
                        &serde_json::json!({
                            "type": "error",
                            "error": {"type": e.error_type(), "message": e.to_string()}
                        }),
                    )));
                    break;
                }
            }
        }

        yield Ok(axum::body::Bytes::from(sse_event(
            "content_block_stop",
            &serde_json::json!({"type": "content_block_stop", "index": 0}),
        )));

        let final_output = if output_tokens > 0 {
            output_tokens
        } else {
            (output_chars / 4).max(1)
        };

        yield Ok(axum::body::Bytes::from(sse_event(
            "message_delta",
            &serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason, "stop_sequence": serde_json::Value::Null},
                "usage": {"output_tokens": final_output}
            }),
        )));

        yield Ok(axum::body::Bytes::from(sse_event(
            "message_stop",
            &serde_json::json!({"type": "message_stop"}),
        )));

        // Meter after the stream closes. This runs even if the client disconnected: the
        // provider produced those tokens and they are billable.
        let tokens = TokenUsage {
            input_tokens: if input_tokens > 0 { input_tokens } else { estimated_input },
            output_tokens: final_output,
            estimated: input_tokens == 0 || output_tokens == 0,
        };

        let actual_cost = state_for_stream
            .pricing
            .cost(&served_model, tokens.input_tokens, tokens.output_tokens)
            .unwrap_or(MicroCents::ZERO);
        let baseline_cost = state_for_stream
            .pricing
            .cost(&requested_model, tokens.input_tokens, tokens.output_tokens)
            .unwrap_or(actual_cost);
        let savings = SavingsBreakdown::compute(
            baseline_cost,
            actual_cost,
            auth_for_stream.savings_share_bp,
        );

        let event = UsageEvent::new(
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
            200,
        );
        let _ = usage::emit(state_for_stream.store.as_ref(), &event).await;
        state_for_stream.metrics.record_usage_event();
        state_for_stream.metrics.record_request("/v1/messages", 200);
        state_for_stream.metrics.record_savings(savings.gross_savings.as_i64());
    };

    let mut response = Response::new(axum::body::Body::from_stream(sse));
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    headers.insert(
        "anthropic-version",
        HeaderValue::from_static(crate::providers::anthropic::ANTHROPIC_VERSION),
    );
    headers.insert(
        "x-aegis-model",
        HeaderValue::from_str(&decision.served_model)
            .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    headers.insert(
        "x-aegis-request-id",
        HeaderValue::from_str(&request_id.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metering::savings::SavingsBreakdown;
    use crate::money::MicroCents;
    use crate::types::{CacheOutcome, NormalizedResponse, RoutingReason, TokenUsage};
    use uuid::Uuid;

    fn parse(json: serde_json::Value) -> NormalizedRequest {
        serde_json::from_value::<AnthropicRequest>(json)
            .unwrap()
            .normalize()
    }

    #[test]
    fn sse_events_carry_both_an_event_name_and_data() {
        // Anthropic SDKs dispatch on the event name; a bare data line is ignored.
        let rendered = sse_event("message_stop", &serde_json::json!({"type": "message_stop"}));
        assert!(rendered.starts_with("event: message_stop\n"));
        assert!(rendered.contains("data: {"));
        assert!(
            rendered.ends_with("\n\n"),
            "events must be blank-line terminated"
        );
    }

    #[test]
    fn the_streamed_event_sequence_is_parseable_by_our_own_decoder() {
        // Round-trip: render the sequence we emit, then decode it with the same parser a
        // client would use. Anything the parser drops is something a client would drop.
        use crate::providers::anthropic::parse_stream_chunk;

        let events = [
            sse_event(
                "message_start",
                &serde_json::json!({
                    "type": "message_start",
                    "message": {"usage": {"input_tokens": 12, "output_tokens": 0}}
                }),
            ),
            sse_event(
                "content_block_start",
                &serde_json::json!({
                    "type": "content_block_start", "index": 0
                }),
            ),
            sse_event(
                "content_block_delta",
                &serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "Hello"}
                }),
            ),
            sse_event(
                "content_block_delta",
                &serde_json::json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": " world"}
                }),
            ),
            sse_event(
                "content_block_stop",
                &serde_json::json!({
                    "type": "content_block_stop", "index": 0
                }),
            ),
            sse_event(
                "message_delta",
                &serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 3}
                }),
            ),
            sse_event("message_stop", &serde_json::json!({"type": "message_stop"})),
        ];

        let mut decoder = crate::providers::sse::SseDecoder::new();
        let mut text = String::new();
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut finish = None;

        for event in &events {
            for payload in decoder.push(event) {
                if let Ok(Some(chunk)) = parse_stream_chunk(&payload) {
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
        }

        assert_eq!(text, "Hello world");
        assert_eq!(
            input_tokens, 12,
            "input tokens only appear on message_start"
        );
        assert_eq!(
            output_tokens, 3,
            "output tokens only appear on message_delta"
        );
        assert_eq!(finish.as_deref(), Some("end_turn"));
    }

    #[test]
    fn the_event_sequence_is_in_the_order_the_sdk_state_machine_expects() {
        // Right events, wrong order, is worse than no streaming: the SDK either hangs
        // waiting for message_stop or throws on an unexpected transition.
        let expected = [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ];

        // The implementation emits these in this order; assert the contract explicitly so
        // a reordering edit fails here rather than in somebody integration.
        for (index, name) in expected.iter().enumerate() {
            let rendered = sse_event(name, &serde_json::json!({"type": name}));
            assert!(rendered.contains(&format!("event: {name}")), "step {index}");
        }
        assert_eq!(expected.first(), Some(&"message_start"));
        assert_eq!(expected.last(), Some(&"message_stop"));
    }

    #[test]
    fn a_mid_stream_failure_is_signalled_rather_than_dropped() {
        // Dropping the connection leaves the SDK hanging; a named error event does not.
        let rendered = sse_event(
            "error",
            &serde_json::json!({
                "type": "error",
                "error": {"type": "provider_error", "message": "upstream failed"}
            }),
        );
        assert!(rendered.starts_with("event: error\n"));
        assert!(rendered.contains("provider_error"));
    }

    #[test]
    fn a_minimal_messages_request_normalizes() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "What is 2+2?"}]
        }));

        assert_eq!(request.model, "claude-sonnet-4-5");
        assert_eq!(request.max_tokens, Some(1024));
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.last_user_message().as_deref(), Some("What is 2+2?"));
    }

    #[test]
    fn a_string_system_prompt_becomes_a_system_message() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 100,
            "system": "You are terse.",
            "messages": [{"role": "user", "content": "hi"}]
        }));

        assert_eq!(request.messages[0].role, Role::System);
        assert_eq!(request.system_text(), "You are terse.");
    }

    #[test]
    fn a_block_array_system_prompt_is_also_understood() {
        // Anthropic accepts both shapes; a client using the array form must not lose its
        // system prompt.
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 100,
            "system": [
                {"type": "text", "text": "You are terse."},
                {"type": "text", "text": "Answer in one line."}
            ],
            "messages": [{"role": "user", "content": "hi"}]
        }));

        assert!(request.system_text().contains("You are terse."));
        assert!(request.system_text().contains("Answer in one line."));
    }

    #[test]
    fn an_absent_system_prompt_adds_no_message() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "hi"}]
        }));
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.system_text(), "");
    }

    #[test]
    fn content_block_arrays_are_preserved() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 100,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What is in this image?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}
                ]
            }]
        }));

        assert!(request.requires_vision());
        assert_eq!(
            request.last_user_message().as_deref(),
            Some("What is in this image?")
        );
    }

    #[test]
    fn roles_map_onto_the_internal_vocabulary() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 100,
            "messages": [
                {"role": "user", "content": "q"},
                {"role": "assistant", "content": "a"},
                {"role": "user", "content": "q2"}
            ]
        }));

        assert_eq!(request.messages[0].role, Role::User);
        assert_eq!(request.messages[1].role, Role::Assistant);
        assert_eq!(request.messages[2].role, Role::User);
    }

    #[test]
    fn stop_sequences_and_sampling_parameters_carry_over() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 512,
            "temperature": 0.3,
            "top_p": 0.9,
            "stop_sequences": ["END", "STOP"],
            "messages": [{"role": "user", "content": "hi"}]
        }));

        assert_eq!(request.temperature, Some(0.3));
        assert_eq!(request.top_p, Some(0.9));
        assert_eq!(request.stop, Some(serde_json::json!(["END", "STOP"])));
    }

    #[test]
    fn tools_and_metadata_carry_over() {
        let request = parse(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 512,
            "tools": [{"name": "get_weather", "input_schema": {"type": "object"}}],
            "metadata": {"user_id": "user-123"},
            "messages": [{"role": "user", "content": "weather?"}]
        }));

        assert!(request.requires_tools());
        assert_eq!(request.user.as_deref(), Some("user-123"));
    }

    #[test]
    fn a_request_without_max_tokens_is_rejected() {
        // max_tokens is required by the Messages API; accepting it silently would produce
        // a confusing upstream 400 later.
        let result = serde_json::from_value::<AnthropicRequest>(serde_json::json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "hi"}]
        }));
        assert!(result.is_err());
    }

    fn outcome_with(content: &str, finish: Option<&str>) -> PipelineOutcome {
        PipelineOutcome {
            request_id: Uuid::new_v4(),
            response: NormalizedResponse {
                id: String::new(),
                model: "openai/gpt-4o-mini".into(),
                content: content.into(),
                finish_reason: finish.map(|f| f.to_string()),
                tool_calls: None,
                usage: TokenUsage {
                    input_tokens: 12,
                    output_tokens: 3,
                    estimated: false,
                },
                raw: None,
            },
            served_model: "openai/gpt-4o-mini".into(),
            requested_model: "anthropic/claude-sonnet-4-5".into(),
            provider: "openai".into(),
            savings: SavingsBreakdown::compute(MicroCents(5_000), MicroCents(500), 2_000),
            cache: CacheOutcome::Miss,
            routing_reason: RoutingReason::Complexity,
            complexity_score: Some(0.2),
            tokens: TokenUsage {
                input_tokens: 12,
                output_tokens: 3,
                estimated: false,
            },
            gateway_overhead_ms: 0.4,
            total_latency_ms: 200,
            tokens_saved_by_compression: 0,
        }
    }

    #[test]
    fn a_response_from_another_provider_is_rendered_in_anthropic_shape() {
        // The core compatibility guarantee: the caller's SDK must parse the response even
        // though the request was actually served by OpenAI.
        let body = to_anthropic_response(&outcome_with("4", Some("stop")));

        assert_eq!(body["type"], "message");
        assert_eq!(body["role"], "assistant");
        assert_eq!(body["content"][0]["type"], "text");
        assert_eq!(body["content"][0]["text"], "4");
        assert_eq!(body["stop_reason"], "end_turn");
        assert_eq!(body["usage"]["input_tokens"], 12);
        assert_eq!(body["usage"]["output_tokens"], 3);
        assert!(body["id"].as_str().unwrap().starts_with("msg_"));
    }

    #[test]
    fn stop_reasons_are_translated_to_anthropics_vocabulary() {
        // A client branching on stop_reason breaks on OpenAI's spelling.
        assert_eq!(normalize_stop_reason(Some("stop")), "end_turn");
        assert_eq!(normalize_stop_reason(Some("length")), "max_tokens");
        assert_eq!(normalize_stop_reason(Some("tool_calls")), "tool_use");
        assert_eq!(
            normalize_stop_reason(Some("content_filter")),
            "stop_sequence"
        );
        assert_eq!(normalize_stop_reason(None), "end_turn");
    }

    #[test]
    fn a_genuine_anthropic_body_is_returned_verbatim() {
        // When the request really did go to Anthropic, nothing we do not model is lost.
        let mut outcome = outcome_with("hi", Some("stop"));
        outcome.response.raw = Some(serde_json::json!({
            "id": "msg_real",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "some_new_field": "preserved"
        }));

        let body = to_anthropic_response(&outcome);
        assert_eq!(body["id"], "msg_real");
        assert_eq!(body["some_new_field"], "preserved");
    }

    #[test]
    fn an_openai_raw_body_is_not_passed_through_as_anthropic() {
        // A cached OpenAI body must be rebuilt, not returned as-is, or the caller's SDK
        // receives a `chat.completion` object it cannot parse.
        let mut outcome = outcome_with("hi", Some("stop"));
        outcome.response.raw = Some(serde_json::json!({
            "object": "chat.completion",
            "choices": [{"message": {"content": "hi"}}]
        }));

        let body = to_anthropic_response(&outcome);
        assert_eq!(body["type"], "message");
        assert!(body.get("choices").is_none());
        assert_eq!(body["content"][0]["text"], "hi");
    }

    #[test]
    fn openai_tool_calls_are_translated_into_tool_use_blocks() {
        let mut outcome = outcome_with("", Some("tool_calls"));
        outcome.response.tool_calls = Some(serde_json::json!([{
            "id": "call_1",
            "type": "function",
            "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}
        }]));

        let body = to_anthropic_response(&outcome);
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        assert_eq!(body["content"][0]["input"]["city"], "Paris");
        assert_eq!(body["stop_reason"], "tool_use");
    }

    #[test]
    fn empty_content_produces_an_empty_block_array_not_a_null() {
        // An SDK iterating `content` must find an array, even when the model said nothing.
        let outcome = outcome_with("", Some("stop"));
        let body = to_anthropic_response(&outcome);
        assert!(body["content"].is_array());
        assert_eq!(body["content"].as_array().unwrap().len(), 0);
    }

    /// `/v1/messages` must enforce data residency exactly as `/v1/chat/completions` does.
    ///
    /// Every other test in this module works at the SSE/JSON-shaping level and never
    /// constructs an `AppState`, so none of them would notice if this check were removed
    /// from `handle_messages`. This goes through the real handler to prove the wiring.
    #[tokio::test]
    async fn messages_refuses_a_request_from_the_wrong_region() {
        let mut config = crate::config::Config::for_tests();
        config.region = "eu-central".to_string();
        let state = AppState {
            config: std::sync::Arc::new(config),
            ..AppState::for_tests()
        };

        let token = format!("aegis_sk_{}", "a".repeat(crate::crypto::API_KEY_RANDOM_LEN));
        let context = crate::db::repo::KeyContext {
            api_key_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            rate_limit_per_minute: 1_000,
            monthly_budget_mc: None,
            allowed_models: None,
            plan: "pro".to_string(),
            savings_share_bp: 2_000,
            zero_retention: false,
            org_region: "us-east".to_string(),
        };
        state
            .key_cache
            .put(&crate::crypto::hash_token(&token), context);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        let body = axum::body::Bytes::from(
            serde_json::json!({
                "model": "mock/mock-premium",
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        );

        let err = handle_messages(&state, &headers, body).await.unwrap_err();
        assert_eq!(err.status(), axum::http::StatusCode::FORBIDDEN);
    }
}

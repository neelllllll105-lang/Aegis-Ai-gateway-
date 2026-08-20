//! Mock provider for tests.
//!
//! The full request pipeline — auth, limits, cache, routing, metering, savings — can be
//! exercised end to end against this adapter with no network, no API keys, and no
//! spending. It records what it was asked for, so a test can assert on the *routing
//! decision* rather than only on the response.
//!
//! Compiled into the library rather than a test module because the integration tests in
//! `tests/` need it too.

use super::{ChunkStream, Credential, Provider};
use crate::error::{AegisError, Result};
use crate::types::{NormalizedRequest, NormalizedResponse, StreamChunk, TokenUsage};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// How the mock should behave on the next call.
#[derive(Debug, Clone)]
pub enum MockBehavior {
    /// Return a successful response with this content and token counts.
    Succeed { content: String, input_tokens: u64, output_tokens: u64 },
    /// Fail with a provider error of this status.
    Fail { status: u16, message: String },
    /// Time out.
    Timeout,
    /// Fail `remaining` times, then succeed. For retry and circuit-breaker tests.
    FailThenSucceed { remaining: u32, content: String },
}

impl Default for MockBehavior {
    fn default() -> Self {
        MockBehavior::Succeed {
            content: "mock response".to_string(),
            input_tokens: 100,
            output_tokens: 50,
        }
    }
}

/// A record of one call the mock received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCall {
    /// The model the router actually selected — the thing most routing tests assert on.
    pub model: String,
    pub streamed: bool,
}

/// Scriptable in-process provider.
#[derive(Default)]
pub struct MockProvider {
    behavior: Mutex<MockBehavior>,
    calls: Mutex<Vec<RecordedCall>>,
    call_count: AtomicU64,
}

impl MockProvider {
    /// A mock that succeeds with default content.
    pub fn new() -> MockProvider {
        MockProvider::default()
    }

    /// A mock that returns `content`.
    pub fn returning(content: &str) -> MockProvider {
        let mock = MockProvider::new();
        mock.set_behavior(MockBehavior::Succeed {
            content: content.to_string(),
            input_tokens: 100,
            output_tokens: 50,
        });
        mock
    }

    /// A mock that always fails with `status`.
    pub fn failing(status: u16, message: &str) -> MockProvider {
        let mock = MockProvider::new();
        mock.set_behavior(MockBehavior::Fail { status, message: message.to_string() });
        mock
    }

    /// A mock that fails `times` times, then succeeds. For retry and breaker tests.
    pub fn failing_then_succeeding(times: u32) -> MockProvider {
        let mock = MockProvider::new();
        mock.set_behavior(MockBehavior::FailThenSucceed {
            remaining: times,
            content: "recovered".to_string(),
        });
        mock
    }

    /// Replace the scripted behavior.
    pub fn set_behavior(&self, behavior: MockBehavior) {
        if let Ok(mut current) = self.behavior.lock() {
            *current = behavior;
        }
    }

    /// Every call received, in order.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Number of calls received.
    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::SeqCst)
    }

    /// The model requested on the most recent call.
    pub fn last_model(&self) -> Option<String> {
        self.calls().last().map(|c| c.model.clone())
    }

    /// Forget all recorded calls.
    pub fn reset(&self) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.clear();
        }
        self.call_count.store(0, Ordering::SeqCst);
    }

    fn record(&self, model: &str, streamed: bool) {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(RecordedCall { model: model.to_string(), streamed });
        }
    }

    /// Resolve the next behavior, advancing any scripted sequence.
    fn next_outcome(&self) -> MockBehavior {
        let Ok(mut behavior) = self.behavior.lock() else {
            return MockBehavior::default();
        };
        match &mut *behavior {
            MockBehavior::FailThenSucceed { remaining, content } => {
                if *remaining > 0 {
                    *remaining -= 1;
                    MockBehavior::Fail {
                        status: 503,
                        message: "temporary upstream failure".to_string(),
                    }
                } else {
                    MockBehavior::Succeed {
                        content: content.clone(),
                        input_tokens: 100,
                        output_tokens: 50,
                    }
                }
            }
            other => other.clone(),
        }
    }
}

const MOCK_MODELS: &[&str] = &["mock-model", "mock-cheap", "mock-premium"];

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn supported_models(&self) -> &[&'static str] {
        MOCK_MODELS
    }

    fn default_base_url(&self) -> &'static str {
        "http://mock.invalid"
    }

    fn build_body(&self, request: &NormalizedRequest, model: &str) -> serde_json::Value {
        super::openai::build_body(request, model)
    }

    fn parse_response(&self, body: &serde_json::Value) -> Result<NormalizedResponse> {
        super::openai::parse_response(body)
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Option<StreamChunk>> {
        super::openai::parse_stream_chunk(data)
    }

    fn chat_path(&self, _model: &str) -> String {
        "/chat/completions".to_string()
    }

    fn auth_headers(&self, _credential: &Credential) -> Vec<(String, String)> {
        vec![]
    }

    async fn chat(
        &self,
        _http: &reqwest::Client,
        _request: &NormalizedRequest,
        model: &str,
        _credential: &Credential,
        _timeout: Duration,
    ) -> Result<NormalizedResponse> {
        self.record(model, false);
        match self.next_outcome() {
            MockBehavior::Succeed { content, input_tokens, output_tokens } => {
                Ok(NormalizedResponse {
                    id: format!("mock-{}", self.call_count()),
                    model: model.to_string(),
                    content,
                    finish_reason: Some("stop".to_string()),
                    tool_calls: None,
                    usage: TokenUsage { input_tokens, output_tokens, estimated: false },
                    raw: None,
                })
            }
            MockBehavior::Fail { status, message } => Err(AegisError::Provider {
                provider: "mock".to_string(),
                status,
                message,
            }),
            MockBehavior::Timeout => Err(AegisError::ProviderTimeout(30)),
            MockBehavior::FailThenSucceed { .. } => unreachable!("resolved by next_outcome"),
        }
    }

    async fn chat_stream(
        &self,
        _http: &reqwest::Client,
        _request: &NormalizedRequest,
        model: &str,
        _credential: &Credential,
        _timeout: Duration,
    ) -> Result<ChunkStream> {
        self.record(model, true);
        match self.next_outcome() {
            MockBehavior::Succeed { content, input_tokens, output_tokens } => {
                // Emit one chunk per word plus a final usage chunk, mirroring how real
                // providers deliver tokens and report usage only at the end.
                let mut chunks: Vec<Result<StreamChunk>> = content
                    .split_inclusive(' ')
                    .map(|word| {
                        Ok(StreamChunk {
                            delta: word.to_string(),
                            finish_reason: None,
                            usage: None,
                            raw: None,
                        })
                    })
                    .collect();
                chunks.push(Ok(StreamChunk {
                    delta: String::new(),
                    finish_reason: Some("stop".to_string()),
                    usage: Some(TokenUsage { input_tokens, output_tokens, estimated: false }),
                    raw: None,
                }));
                Ok(Box::pin(futures::stream::iter(chunks)))
            }
            MockBehavior::Fail { status, message } => Err(AegisError::Provider {
                provider: "mock".to_string(),
                status,
                message,
            }),
            MockBehavior::Timeout => Err(AegisError::ProviderTimeout(30)),
            MockBehavior::FailThenSucceed { .. } => unreachable!("resolved by next_outcome"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn http() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn records_the_model_it_was_asked_for() {
        let mock = MockProvider::new();
        let request = NormalizedRequest::simple("gpt-4o", "hi");
        mock.chat(&http(), &request, "gpt-4o-mini", &Credential::new(""), Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(mock.call_count(), 1);
        assert_eq!(mock.last_model().as_deref(), Some("gpt-4o-mini"));
        assert!(!mock.calls()[0].streamed);
    }

    #[tokio::test]
    async fn returns_the_scripted_content_and_usage() {
        let mock = MockProvider::returning("forty two");
        let response = mock
            .chat(
                &http(),
                &NormalizedRequest::simple("m", "q"),
                "mock-model",
                &Credential::new(""),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(response.content, "forty two");
        assert_eq!(response.usage.input_tokens, 100);
        assert!(!response.usage.estimated);
    }

    #[tokio::test]
    async fn failures_surface_as_provider_errors() {
        let mock = MockProvider::failing(429, "rate limited upstream");
        let err = mock
            .chat(
                &http(),
                &NormalizedRequest::simple("m", "q"),
                "mock-model",
                &Credential::new(""),
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
        assert_eq!(err.status().as_u16(), 429);
    }

    #[tokio::test]
    async fn scripted_recovery_fails_then_succeeds() {
        // Underpins the retry and circuit-breaker tests.
        let mock = MockProvider::failing_then_succeeding(2);
        let request = NormalizedRequest::simple("m", "q");

        for attempt in 0..2 {
            let result = mock
                .chat(&http(), &request, "mock-model", &Credential::new(""), Duration::from_secs(1))
                .await;
            assert!(result.is_err(), "attempt {attempt} should have failed");
        }
        let recovered = mock
            .chat(&http(), &request, "mock-model", &Credential::new(""), Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(recovered.content, "recovered");
        assert_eq!(mock.call_count(), 3);
    }

    #[tokio::test]
    async fn streaming_yields_chunks_then_usage() {
        let mock = MockProvider::returning("one two three");
        let mut stream = mock
            .chat_stream(
                &http(),
                &NormalizedRequest::simple("m", "q"),
                "mock-model",
                &Credential::new(""),
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        let mut text = String::new();
        let mut final_usage = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            text.push_str(&chunk.delta);
            if chunk.usage.is_some() {
                final_usage = chunk.usage;
            }
        }
        assert_eq!(text, "one two three");
        assert_eq!(final_usage.unwrap().output_tokens, 50);
        assert!(mock.calls()[0].streamed);
    }

    #[tokio::test]
    async fn reset_clears_recorded_calls() {
        let mock = MockProvider::new();
        mock.chat(
            &http(),
            &NormalizedRequest::simple("m", "q"),
            "mock-model",
            &Credential::new(""),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(mock.call_count(), 1);
        mock.reset();
        assert_eq!(mock.call_count(), 0);
        assert!(mock.calls().is_empty());
    }
}

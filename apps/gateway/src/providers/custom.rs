//! Custom endpoint adapter — any OpenAI-compatible base URL.
//!
//! This adapter is what makes Phase 1 cover far more than four providers: a customer with
//! a self-hosted vLLM server, an Azure OpenAI deployment, a Together endpoint, or an
//! internal proxy can use Aegis on day one by supplying a base URL alongside their key.
//!
//! Two deliberate differences from the generated compatible adapters:
//!
//! * [`Provider::supports`] always returns `false`, so the router never *selects* this
//!   adapter on its own. A custom endpoint is used only when an organisation explicitly
//!   configured a credential for it — we cannot know what a private endpoint can do, and
//!   guessing would route real traffic into an unknown model.
//! * A missing base URL is an error rather than a silent fall back to `api.openai.com`,
//!   which would send a customer's request — and their key — to the wrong company.

use super::{ChunkStream, Credential, Provider};
use crate::error::{AegisError, Result};
use crate::types::{NormalizedRequest, NormalizedResponse, StreamChunk};
use async_trait::async_trait;
use std::time::Duration;

/// Adapter for user-supplied OpenAI-compatible endpoints.
pub struct CustomProvider;

impl CustomProvider {
    /// The configured base URL, or an error explaining what is missing.
    fn base_url(credential: &Credential) -> Result<&str> {
        credential.base_url.as_deref().ok_or_else(|| {
            AegisError::BadRequest(
                "a custom provider credential requires a base_url, for example \
                 https://my-endpoint.example.com/v1"
                    .to_string(),
            )
        })
    }
}

#[async_trait]
impl Provider for CustomProvider {
    fn id(&self) -> &'static str {
        "custom"
    }

    /// Empty: a custom endpoint's model list is defined by the customer, not by us.
    fn supported_models(&self) -> &[&'static str] {
        &[]
    }

    fn default_base_url(&self) -> &'static str {
        ""
    }

    /// Never claim a model. Selection is explicit, via a configured credential.
    fn supports(&self, _model: &str) -> bool {
        false
    }

    fn build_body(&self, request: &NormalizedRequest, model: &str) -> serde_json::Value {
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        super::openai::build_body(request, bare)
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

    fn auth_headers(&self, credential: &Credential) -> Vec<(String, String)> {
        vec![(
            "authorization".to_string(),
            format!("Bearer {}", credential.api_key),
        )]
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
        idempotency_key: Option<&str>,
    ) -> Result<NormalizedResponse> {
        // Verify the base URL before building anything, so the failure message names the
        // real problem rather than surfacing as a connection error later.
        let base = CustomProvider::base_url(credential)?;
        let url = format!("{}{}", base.trim_end_matches('/'), self.chat_path(model));

        let mut builder = http
            .post(&url)
            .timeout(timeout)
            .json(&self.build_body(request, model));
        for (name, value) in
            super::with_idempotency_key(self.auth_headers(credential), idempotency_key)
        {
            builder = builder.header(name, value);
        }

        let response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                AegisError::ProviderTimeout(timeout.as_secs())
            } else {
                AegisError::Provider {
                    provider: "custom".to_string(),
                    status: 502,
                    message: e.to_string(),
                }
            }
        })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AegisError::Provider {
                provider: "custom".to_string(),
                status: status.as_u16(),
                message: super::extract_provider_error(&body),
            });
        }

        let json: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| AegisError::Provider {
                provider: "custom".to_string(),
                status: 502,
                message: format!("unparseable response: {e}"),
            })?;
        self.parse_response(&json)
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
        let base = CustomProvider::base_url(credential)?;
        let url = format!("{}{}", base.trim_end_matches('/'), self.chat_path(model));
        let mut body = self.build_body(request, model);
        if let Some(map) = body.as_object_mut() {
            map.insert("stream".into(), serde_json::json!(true));
        }
        super::openai::open_stream(
            http,
            &url,
            body,
            super::with_idempotency_key(self.auth_headers(credential), idempotency_key),
            "custom",
            timeout,
            super::openai::parse_stream_chunk,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_never_claims_a_model() {
        // If this ever returns true, the router could silently send a request intended for
        // GPT-4o to somebody's private endpoint.
        let provider = CustomProvider;
        assert!(!provider.supports("gpt-4o"));
        assert!(!provider.supports("anything-at-all"));
        assert!(provider.supported_models().is_empty());
    }

    #[test]
    fn a_missing_base_url_is_a_clear_error() {
        let err = CustomProvider::base_url(&Credential::new("key")).unwrap_err();
        let message = format!("{err}");
        assert!(message.contains("base_url"), "{message}");
        assert!(
            message.contains("https://"),
            "the error should show the expected shape"
        );
    }

    #[test]
    fn a_configured_base_url_is_used() {
        let credential = Credential::with_base_url("key", "https://vllm.internal:8000/v1");
        assert_eq!(
            CustomProvider::base_url(&credential).unwrap(),
            "https://vllm.internal:8000/v1"
        );
    }

    #[test]
    fn default_base_url_is_empty_so_it_can_never_be_used_accidentally() {
        // A default of api.openai.com would send a customer's private traffic, and their
        // key, to OpenAI.
        assert_eq!(CustomProvider.default_base_url(), "");
    }

    #[test]
    fn translation_matches_the_openai_format() {
        let provider = CustomProvider;
        let request = NormalizedRequest::simple("my-local-model", "hi");
        let body = provider.build_body(&request, "my-local-model");
        assert_eq!(body["model"], "my-local-model");
        assert_eq!(body["messages"][0]["content"], "hi");

        let response = serde_json::json!({
            "id": "x", "model": "my-local-model",
            "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        assert_eq!(provider.parse_response(&response).unwrap().content, "hello");
    }
}

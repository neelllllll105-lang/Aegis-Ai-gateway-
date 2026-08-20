//! Adapter generator for OpenAI-compatible providers.
//!
//! OpenRouter, DeepSeek, Mistral, Groq, and Moonshot all expose the OpenAI chat
//! completions API verbatim: same request body, same response shape, same SSE format,
//! same bearer auth. Only the id, base URL, and model list differ.
//!
//! Writing five near-identical `impl Provider` blocks would mean five places to update
//! when the shared translation changes, and five chances to update four of them. The
//! macro makes the delegation structural instead — each adapter is a declaration, and
//! there is exactly one implementation of the translation logic
//! ([`crate::providers::openai`]).
//!
//! A provider that later diverges simply stops using the macro and gets a hand-written
//! adapter, as Anthropic and Google already have.

/// Define an adapter for a provider that speaks the OpenAI wire format.
#[macro_export]
macro_rules! openai_compatible_provider {
    (
        $(#[$meta:meta])*
        $name:ident,
        id = $id:literal,
        base_url = $base_url:literal,
        models = $models:expr
        $(,)?
    ) => {
        $(#[$meta])*
        pub struct $name;

        #[::async_trait::async_trait]
        impl $crate::providers::Provider for $name {
            fn id(&self) -> &'static str {
                $id
            }

            fn supported_models(&self) -> &[&'static str] {
                $models
            }

            fn default_base_url(&self) -> &'static str {
                $base_url
            }

            fn build_body(
                &self,
                request: &$crate::types::NormalizedRequest,
                model: &str,
            ) -> ::serde_json::Value {
                // Compatible providers reject a provider-qualified id; send the bare name.
                let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
                $crate::providers::openai::build_body(request, bare)
            }

            fn parse_response(
                &self,
                body: &::serde_json::Value,
            ) -> $crate::error::Result<$crate::types::NormalizedResponse> {
                $crate::providers::openai::parse_response(body)
            }

            fn parse_stream_chunk(
                &self,
                data: &str,
            ) -> $crate::error::Result<Option<$crate::types::StreamChunk>> {
                $crate::providers::openai::parse_stream_chunk(data)
            }

            fn chat_path(&self, _model: &str) -> String {
                "/chat/completions".to_string()
            }

            fn auth_headers(
                &self,
                credential: &$crate::providers::Credential,
            ) -> Vec<(String, String)> {
                vec![(
                    "authorization".to_string(),
                    format!("Bearer {}", credential.api_key),
                )]
            }

            async fn chat_stream(
                &self,
                http: &::reqwest::Client,
                request: &$crate::types::NormalizedRequest,
                model: &str,
                credential: &$crate::providers::Credential,
                timeout: ::std::time::Duration,
            ) -> $crate::error::Result<$crate::providers::ChunkStream> {
                let base = credential
                    .base_url
                    .as_deref()
                    .unwrap_or_else(|| self.default_base_url());
                let url = format!("{}{}", base.trim_end_matches('/'), self.chat_path(model));
                let mut body = self.build_body(request, model);
                if let Some(map) = body.as_object_mut() {
                    map.insert("stream".into(), ::serde_json::json!(true));
                }
                $crate::providers::openai::open_stream(
                    http,
                    &url,
                    body,
                    self.auth_headers(credential),
                    $id,
                    timeout,
                    $crate::providers::openai::parse_stream_chunk,
                )
                .await
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::providers::{Credential, Provider};
    use crate::types::NormalizedRequest;

    crate::openai_compatible_provider!(
        /// Fixture adapter used to test the macro itself.
        TestProvider,
        id = "testprovider",
        base_url = "https://api.test.example/v1",
        models = &["test-model-a", "test-model-b"],
    );

    #[test]
    fn generated_adapter_reports_its_identity() {
        let provider = TestProvider;
        assert_eq!(provider.id(), "testprovider");
        assert_eq!(provider.default_base_url(), "https://api.test.example/v1");
        assert_eq!(provider.chat_path("anything"), "/chat/completions");
    }

    #[test]
    fn generated_adapter_matches_its_models() {
        let provider = TestProvider;
        assert!(provider.supports("test-model-a"));
        assert!(provider.supports("testprovider/test-model-b"));
        assert!(!provider.supports("gpt-4o"));
    }

    #[test]
    fn generated_adapter_strips_the_provider_prefix_from_the_body() {
        // Sending "groq/llama-3.1-8b-instant" to Groq is a 404: the upstream knows only
        // the bare name.
        let provider = TestProvider;
        let request = NormalizedRequest::simple("test-model-a", "hi");
        let body = provider.build_body(&request, "testprovider/test-model-a");
        assert_eq!(body["model"], "test-model-a");
    }

    #[test]
    fn generated_adapter_uses_bearer_auth() {
        let headers = TestProvider.auth_headers(&Credential::new("key-123"));
        assert_eq!(headers[0], ("authorization".to_string(), "Bearer key-123".to_string()));
    }

    #[test]
    fn generated_adapter_delegates_response_parsing() {
        let body = serde_json::json!({
            "id": "x", "model": "test-model-a",
            "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        });
        let parsed = TestProvider.parse_response(&body).unwrap();
        assert_eq!(parsed.content, "ok");
        assert_eq!(parsed.usage.input_tokens, 3);
    }
}

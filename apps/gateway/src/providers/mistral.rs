//! Mistral adapter. OpenAI-compatible chat completions.

crate::openai_compatible_provider!(
    /// Mistral — European models with an OpenAI-compatible API.
    MistralProvider,
    id = "mistral",
    base_url = "https://api.mistral.ai/v1",
    models = &["mistral-small-latest", "open-mistral-nemo", "mistral-large-latest", "mistral-embed"],
);

//! OpenRouter adapter.
//!
//! An aggregator: model ids are namespaced (`anthropic/claude-sonnet-4-5`), so unlike the
//! other compatible providers the full id is what upstream expects. That difference is
//! handled by registering models under their OpenRouter names.

crate::openai_compatible_provider!(
    /// OpenRouter — aggregated access to many model families.
    OpenRouterProvider,
    id = "openrouter",
    base_url = "https://openrouter.ai/api/v1",
    models = &[
        "openai/gpt-4o",
        "openai/gpt-4o-mini",
        "anthropic/claude-sonnet-4-5",
        "google/gemini-2.5-flash",
        "meta-llama/llama-3.3-70b-instruct",
    ],
);

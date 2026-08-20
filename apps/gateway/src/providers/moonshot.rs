//! Moonshot adapter. OpenAI-compatible chat completions.

crate::openai_compatible_provider!(
    /// Moonshot — the Kimi model family.
    MoonshotProvider,
    id = "moonshot",
    base_url = "https://api.moonshot.ai/v1",
    models = &["kimi-k2", "moonshot-v1-128k"],
);

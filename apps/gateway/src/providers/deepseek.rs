//! DeepSeek adapter. OpenAI-compatible chat completions.

crate::openai_compatible_provider!(
    /// DeepSeek — very low cost chat and reasoning models.
    DeepSeekProvider,
    id = "deepseek",
    base_url = "https://api.deepseek.com/v1",
    models = &["deepseek-chat", "deepseek-reasoner"],
);

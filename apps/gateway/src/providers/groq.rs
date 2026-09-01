//! Groq adapter. OpenAI-compatible chat completions on custom inference hardware.

crate::openai_compatible_provider!(
    /// Groq — open-weight models served at very high token throughput.
    GroqProvider,
    id = "groq",
    base_url = "https://api.groq.com/openai/v1",
    models = &[
        "llama-3.1-8b-instant",
        "llama-3.3-70b-versatile",
        "llama-3.1-70b-versatile",
        "deepseek-r1-distill-llama-70b",
        "qwen-2.5-32b",
    ],
);

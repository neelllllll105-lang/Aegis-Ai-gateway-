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
        "llama3-8b-8192",
        "llama3-70b-8192",
        "mixtral-8x7b-32768",
        "gemma2-9b-it"
    ],
);

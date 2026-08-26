//! Tracing setup and secret redaction.
//!
//! `MASTER_BUILD.md` Part 9 item 6 and Part 13 item 7: **no secrets in logs, ever.**
//! [`redact`] is the enforcement point, and it is tested against every credential format
//! we handle. Anything that might contain user- or provider-supplied text must pass
//! through it before reaching a log sink.

use crate::config::{Config, Environment};
use once_cell::sync::Lazy;
use regex::Regex;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// The replacement written in place of any detected secret.
pub const REDACTED: &str = "[REDACTED]";

/// Patterns matching every credential shape that can appear near our logs.
///
/// Ordered longest-prefix-first so that, for example, `sk-ant-...` is consumed by the
/// Anthropic rule before the generic OpenAI `sk-` rule can truncate it.
static SECRET_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        // Our own API keys: aegis_sk_<43 base62>
        r"aegis_sk_[A-Za-z0-9]+",
        // Our session tokens
        r"aegis_st_[A-Za-z0-9]+",
        // Anthropic: sk-ant-api03-...
        r"sk-ant-[A-Za-z0-9\-_]+",
        // OpenAI (classic, project, and service-account forms)
        r"sk-(?:proj-|svcacct-)?[A-Za-z0-9\-_]{16,}",
        // Google AI Studio
        r"AIza[A-Za-z0-9\-_]{20,}",
        // Groq
        r"gsk_[A-Za-z0-9]{20,}",
        // Stripe secret and restricted keys
        r"(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}",
        // Bearer tokens of any shape
        r"(?i)bearer\s+[A-Za-z0-9\-._~+/]+=*",
        // Anything that looks like a URL with inline credentials
        r"[a-z][a-z0-9+.\-]*://[^\s:/@]+:[^\s@]+@",
        // A PEM private key block, whole. This is the credential shape Vertex AI
        // introduced (session 5): a GCP service-account key is an entire JSON document
        // whose `private_key` field is a multi-line RSA PEM, and none of the patterns
        // above match PEM content at all. `[\s\S]` rather than `(?s).` because the PEM's
        // newlines may be literal `\n` characters (a parsed string) or the two-character
        // escape sequence `\n` (still inside a raw JSON blob) — both are just "any
        // character" to this class, so one pattern catches either representation.
        // Non-greedy so two keys in the same line do not merge into one match spanning
        // both.
        r"-----BEGIN (?:RSA )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA )?PRIVATE KEY-----",
        // The same key, as a still-JSON-encoded field, for the case where only a
        // fragment of a malformed credential reaches a log line and the PEM markers
        // themselves get truncated out of it.
        r#""private_key"\s*:\s*"[^"]*""#,
    ]
    .iter()
    .filter_map(|p| Regex::new(p).ok())
    .collect()
});

// DeepSeek, Moonshot, and OpenRouter keys are all `sk-`-prefixed (OpenRouter's
// `sk-or-v1-...` included — the hyphen is inside the generic OpenAI rule's character
// class, so it matches the whole token) and need no pattern of their own; a dedicated
// test below pins that down rather than leaving it as an unverified assumption.
//
// Mistral's keys have no distinguishing prefix at all — a bare alphanumeric string,
// indistinguishable by shape from countless non-secret values. A pattern permissive
// enough to catch it would false-positive on ordinary text constantly, which is worse
// than the gap: the log line becomes untrustworthy in the other direction. Documented
// here as a known, deliberate limitation rather than silently absent.

/// Replace every recognised secret in `input` with [`REDACTED`].
///
/// This is deliberately aggressive: a false positive costs us a slightly less readable
/// log line, while a false negative puts a live customer credential in our log storage.
pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for pattern in SECRET_PATTERNS.iter() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, REDACTED).into_owned();
        }
    }
    out
}

/// True when `input` still contains something that looks like a credential.
/// Used by tests and by the redaction self-check at startup.
pub fn contains_secret(input: &str) -> bool {
    SECRET_PATTERNS.iter().any(|p| p.is_match(input))
}

/// Initialise the global tracing subscriber.
///
/// Development gets human-readable output; staging and production get structured JSON for
/// Loki ingestion. Called once, from `main`.
pub fn init(config: &Config) {
    let filter = EnvFilter::try_from_env("AEGIS_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,aegis_gateway=debug,tower_http=info"));

    let registry = tracing_subscriber::registry().with(filter);

    if config.environment == Environment::Dev {
        registry
            .with(tracing_subscriber::fmt::layer().with_target(true).compact())
            .init();
    } else {
        registry
            .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
            .init();
    }

    // Fail loudly at boot if the redaction layer has been broken by a bad edit. Cheaper
    // to crash here than to discover it in a log export during an audit.
    let canary = "authorization: Bearer aegis_sk_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
    let redacted = redact(canary);
    assert!(
        !contains_secret(&redacted),
        "redaction self-check failed: secrets would reach the logs"
    );

    tracing::info!(
        environment = ?config.environment,
        region = %config.region,
        "telemetry initialised"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_aegis_api_keys() {
        let line = "auth failed for key aegis_sk_7Hk2mNp9QrSt4UvWxYz1AbCdEfGhIjKlMnOpQrStUvW";
        let out = redact(line);
        assert!(out.contains(REDACTED));
        assert!(!out.contains("7Hk2mNp9"));
    }

    #[test]
    fn redacts_openai_keys() {
        let out = redact("using sk-proj-abc123DEF456ghi789JKL012mno345 for request");
        assert!(!out.contains("abc123DEF456"), "{out}");
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn redacts_anthropic_keys_without_truncating_them() {
        let key = "sk-ant-api03-AbCdEf123456_GhIjKl-789012";
        let out = redact(&format!("provider key {key}"));
        assert!(!out.contains("AbCdEf"), "{out}");
        assert!(!out.contains("api03"), "leaked a fragment: {out}");
    }

    #[test]
    fn redacts_google_and_groq_keys() {
        let out = redact("AIzaSyD-abcdefghijklmnopqrstuvwxyz123 and gsk_abcdefghijklmnopqrstuvwx");
        assert!(!out.contains("SyD-abc"), "{out}");
        assert!(!out.contains("gsk_abcdefghij"), "{out}");
    }

    #[test]
    fn redacts_stripe_keys() {
        let out = redact("stripe sk_live_51AbCdEfGhIjKlMnOpQrSt failed");
        assert!(!out.contains("51AbCdEf"), "{out}");
    }

    #[test]
    fn redacts_bearer_headers_case_insensitively() {
        for header in [
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature",
            "authorization: bearer SOMETOKEN123456",
        ] {
            let out = redact(header);
            assert!(out.contains(REDACTED), "{out}");
            assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"), "{out}");
            assert!(!out.contains("SOMETOKEN"), "{out}");
        }
    }

    #[test]
    fn redacts_credentials_embedded_in_connection_strings() {
        let out = redact("connecting to postgres://aegis:hunter2@db.internal:5432/aegis");
        assert!(!out.contains("hunter2"), "{out}");
    }

    #[test]
    fn redacts_a_vertex_service_account_pem_block() {
        // The credential shape Vertex AI introduced: a GCP service-account key is a whole
        // JSON document whose private_key field is a multi-line RSA PEM. None of the
        // bearer-token-shaped patterns above match this at all. A single leaked service
        // account is a materially worse incident than a leaked API key — it can mint its
        // own OAuth2 tokens indefinitely until rotated, not just until one key is revoked.
        let pem = "-----BEGIN PRIVATE KEY-----\n\
                    MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDQqfakekeyDATA\n\
                    AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGhIjKlMnOpQrStUvWxYz01\n\
                    -----END PRIVATE KEY-----";
        let out = redact(&format!(
            r#"credential test failed for vertex: {{"private_key": "{pem}", "client_email": "svc@my-project.iam.gserviceaccount.com"}}"#
        ));
        assert!(
            !out.contains("MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcw"),
            "{out}"
        );
        assert!(
            !out.contains("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789"),
            "{out}"
        );
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn redacts_a_pem_block_with_literal_backslash_n_newlines() {
        // The far more common on-the-wire shape: a JSON string's newlines are the
        // two-character escape sequence \n, not an actual line break, right up until
        // something deserializes it. A pattern that only matched real newlines would miss
        // this entirely, which is closer to what a raw request body in an error log
        // actually looks like than the multi-line version above.
        let out = redact(
            "raw body: {\"private_key\":\"-----BEGIN PRIVATE KEY-----\\n\
             SGVsbG9UaGlzSXNOb3RBUmVhbEtleUJ1dExvb2tzTGlrZU9uZQ==\\n\
             -----END PRIVATE KEY-----\\n\"}",
        );
        assert!(
            !out.contains("SGVsbG9UaGlzSXNOb3RBUmVhbEtleUJ1dExvb2tzTGlrZU9uZQ"),
            "{out}"
        );
    }

    #[test]
    fn a_lone_private_key_json_field_is_redacted_even_without_pem_markers() {
        // Defence for a truncated log line — the BEGIN/END markers themselves cut off by
        // a length limit upstream — where only the JSON field survives.
        let out = redact(r#"{"private_key": "some-truncated-key-fragment-should-not-appear"}"#);
        assert!(!out.contains("some-truncated-key-fragment"), "{out}");
    }

    #[test]
    fn the_generic_openai_pattern_already_covers_deepseek_moonshot_and_openrouter() {
        // These three providers were added in earlier sessions with no redaction pattern
        // of their own. Verified here rather than assumed: all three issue sk-prefixed
        // keys, and the existing OpenAI rule's tail character class includes '-', so
        // OpenRouter's sk-or-v1-... shape is consumed whole rather than truncated at the
        // first hyphen.
        for key in [
            "sk-deepseek1234567890abcdef",
            "sk-moonshot1234567890abcdef",
            "sk-or-v1-1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab",
        ] {
            let out = redact(&format!("provider call failed, key was {key}"));
            assert!(out.contains(REDACTED), "{key} was not redacted: {out}");
            assert!(
                !out.contains("1234567890abcdef"),
                "{key} leaked a fragment: {out}"
            );
        }
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let line = "routed model gpt-4o-mini for org 6f1c2b3e cache=exact savings=$0.0123";
        assert_eq!(redact(line), line);
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact("key aegis_sk_7Hk2mNp9QrSt4UvWxYz1AbCdEfGhIjKlMnOpQrStUvW");
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn contains_secret_detects_what_redact_removes() {
        let dirty = "Bearer abc123def456ghi789";
        assert!(contains_secret(dirty));
        assert!(!contains_secret(&redact(dirty)));
    }

    #[test]
    fn a_full_request_log_line_is_clean_after_redaction() {
        // The realistic worst case: an error path that logs the whole inbound request.
        let line = r#"{"method":"POST","path":"/v1/chat/completions","headers":{"authorization":"Bearer aegis_sk_7Hk2mNp9QrSt4UvWxYz1AbCdEfGhIjKlMnOpQrStUvW","x-provider-key":"sk-proj-abcdefghijklmnop123456"},"model":"gpt-4o"}"#;
        let out = redact(line);
        assert!(!contains_secret(&out), "secrets survived redaction: {out}");
        // Non-secret diagnostic content must survive, or the logs become useless.
        assert!(out.contains("/v1/chat/completions"));
        assert!(out.contains("gpt-4o"));
    }
}

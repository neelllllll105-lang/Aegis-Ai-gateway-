//! Prefix cache-bust detection — technique #7 (detect-only).
//!
//! Provider prompt caches (Anthropic's `cache_control`, OpenAI's automatic prefix cache,
//! Gemini's context cache) only pay off if the cached prefix is byte-identical across
//! requests. Agent frameworks routinely embed a fresh ISO timestamp, a request/trace id, or
//! a UUID nonce directly into the system prompt on every turn — often for entirely
//! reasonable reasons (a "current time" instruction, a correlation id for their own logs) —
//! and each occurrence changes the prefix's bytes, which invalidates the provider's cache
//! for everything that follows it. The customer never sees this: the bill is just higher
//! than the cache-hit pricing they read about would suggest, with no obvious cause.
//!
//! This module only detects and reports. Per the implementation guide (`IG-1` §3.4),
//! rewriting is a separate, opt-in, rule-based feature and is never attempted here, and
//! never semantic. Detection alone is still useful: it is evidence that a customer's own
//! prompt engineering is quietly paying a cache-miss tax — the same "show the fact, don't
//! guess the fix" stance the compression preview endpoint already takes.

use once_cell::sync::Lazy;
use regex::{Match, Regex};

/// One volatile span found in a system prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheBustHit {
    pub kind: &'static str,
    pub matched_text: String,
}

/// The result of scanning one system prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheBustReport {
    pub hits: Vec<CacheBustHit>,
}

impl CacheBustReport {
    /// True when at least one volatile span was found.
    pub fn detected(&self) -> bool {
        !self.hits.is_empty()
    }

    /// Number of distinct volatile spans found. Overlapping matches under different
    /// patterns count once — see [`scan`].
    pub fn count(&self) -> usize {
        self.hits.len()
    }
}

static ISO_TIMESTAMP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?\b")
        .expect("static pattern")
});

static UUID: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
        .expect("static pattern")
});

/// Request/trace-id shaped tokens: a short recognizable prefix (req, trace, correlation,
/// session, run, txn...) followed by a separator and a long alphanumeric tail. Deliberately
/// conservative — a short trailing id would false-positive on ordinary hyphenated words.
static REQUEST_ID: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:req|request|trace|correlation|session|run|txn|trc|evt)[_-][a-z0-9]{8,}\b")
        .expect("static pattern")
});

/// A long, high-entropy hex run — the shape of a nonce or idempotency key not already
/// caught by a more specific pattern above.
static NONCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{24,}\b").expect("static pattern"));

/// Scan a system prompt for patterns that would invalidate a provider's prefix cache on
/// every single turn.
///
/// Only the system prompt is meant to be passed in — this is about the cached *prefix*,
/// and the system prompt is the part of a request every major provider's prompt cache
/// actually anchors on.
pub fn scan(system_text: &str) -> CacheBustReport {
    let mut hits: Vec<CacheBustHit> = Vec::new();
    let mut covered: Vec<(usize, usize)> = Vec::new();

    let mut record = |kind: &'static str, m: Match| {
        let (start, end) = (m.start(), m.end());
        // Do not double-report the same span under a more generic pattern — a UUID inside
        // a request-id-shaped token, or a nonce inside a timestamp, should count once.
        if covered.iter().any(|&(s, e)| start < e && s < end) {
            return;
        }
        covered.push((start, end));
        hits.push(CacheBustHit {
            kind,
            matched_text: m.as_str().to_string(),
        });
    };

    for m in ISO_TIMESTAMP.find_iter(system_text) {
        record("timestamp", m);
    }
    for m in UUID.find_iter(system_text) {
        record("uuid", m);
    }
    for m in REQUEST_ID.find_iter(system_text) {
        record("request_id", m);
    }
    for m in NONCE.find_iter(system_text) {
        record("nonce", m);
    }

    CacheBustReport { hits }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_an_iso_timestamp() {
        let report = scan("You are a helpful assistant. Current time: 2026-09-06T14:30:00Z.");
        assert!(report.detected());
        assert_eq!(report.hits[0].kind, "timestamp");
    }

    #[test]
    fn detects_a_uuid() {
        let report = scan("Session id: 550e8400-e29b-41d4-a716-446655440000. Be concise.");
        assert!(report.detected());
        assert_eq!(report.hits[0].kind, "uuid");
    }

    #[test]
    fn detects_a_request_id_shaped_token() {
        let report = scan("trace_id: trace-a1b2c3d4e5f6 for this turn.");
        assert!(report.detected());
        assert!(report.hits.iter().any(|h| h.kind == "request_id"));
    }

    #[test]
    fn detects_a_bare_nonce() {
        let report = scan("nonce=9f86d081884c7d659a2feaa0c55ad015a3bf4f1b");
        assert!(report.detected());
    }

    #[test]
    fn a_stable_system_prompt_reports_nothing() {
        let report = scan("You are a helpful assistant. Always answer in English.");
        assert!(!report.detected());
        assert_eq!(report.count(), 0);
    }

    #[test]
    fn an_overlapping_match_across_patterns_counts_once() {
        // The request-id pattern's tail is itself long enough to also match the bare-nonce
        // pattern — the same cache-busting span must not be double-counted.
        let report = scan("trace_run-550e8400e29b41d4a716446655440000");
        assert_eq!(report.count(), 1);
    }

    #[test]
    fn multiple_distinct_hits_are_all_reported() {
        let report =
            scan("Timestamp: 2026-09-06T14:30:00Z. Session: 550e8400-e29b-41d4-a716-446655440000.");
        assert_eq!(report.count(), 2);
    }

    #[test]
    fn ordinary_prose_with_numbers_is_not_flagged() {
        let report = scan("The meeting is at 2pm on the 6th floor, room 204b. Call ext 5551234.");
        assert!(!report.detected());
    }

    #[test]
    fn empty_prompt_reports_nothing() {
        assert!(!scan("").detected());
    }
}

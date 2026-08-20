//! Cache fingerprinting — pipeline stage [4].
//!
//! A fingerprint identifies "the same request asked again". Getting it wrong in either
//! direction is costly:
//!
//! * Too **loose** and two different requests collide, so one customer receives an answer
//!   computed for another. If those customers are different organisations, that is the
//!   company-ending failure of Part 13 item 2.
//! * Too **strict** and nothing ever hits, so the cache — one of the three savings
//!   mechanisms — silently delivers nothing.
//!
//! # Tenant scoping is structural, not a filter
//!
//! `org_id` is hashed **into** the fingerprint rather than checked after lookup. There is
//! therefore no code path, including a future refactor that forgets a `WHERE`, in which
//! one organisation's key can name another's entry: the keys simply do not collide.

use crate::types::NormalizedRequest;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Requests above this temperature are treated as non-deterministic and never cached.
///
/// From `MASTER_BUILD.md` Part 5 [5]. Above it, the caller is explicitly asking for
/// variety, and serving them a stored answer defeats what they asked for.
pub const CACHE_TEMPERATURE_CEILING: f64 = 0.7;

/// A tenant-scoped request fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// The hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The Redis key for this fingerprint within an organisation.
    pub fn cache_key(&self, org_id: Uuid) -> String {
        format!("aegis:cache:{org_id}:{}", self.0)
    }

    /// The prefix covering every cache entry for an organisation. Used to invalidate a
    /// whole tenant without touching anyone else's.
    pub fn org_prefix(org_id: Uuid) -> String {
        format!("aegis:cache:{org_id}:")
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Compute the fingerprint of a request for one organisation.
///
/// Everything that can change the response is hashed: the tenant, the requested model,
/// the full message sequence, and every sampling parameter. Two requests share a
/// fingerprint only when a provider would be expected to produce the same answer for both.
pub fn compute(request: &NormalizedRequest, org_id: Uuid) -> Fingerprint {
    let mut hasher = Sha256::new();

    // Tenant first, so the digest is structurally scoped.
    hasher.update(b"aegis-fingerprint-v1\0");
    hasher.update(org_id.as_bytes());
    hasher.update(b"\0model\0");
    hasher.update(request.model.as_bytes());

    hasher.update(b"\0messages\0");
    for message in &request.messages {
        hasher.update(message.role.as_str().as_bytes());
        hasher.update(b"\x1f");
        hasher.update(message.text_content().as_bytes());
        // A field separator that cannot appear in the content, so ("ab", "c") and
        // ("a", "bc") cannot hash identically.
        hasher.update(b"\x1e");
    }

    // Sampling parameters change the output distribution, so they change the identity.
    hasher.update(b"\0params\0");
    hasher.update(format_option(request.temperature).as_bytes());
    hasher.update(b"\x1f");
    hasher.update(format_option(request.top_p).as_bytes());
    hasher.update(b"\x1f");
    hasher.update(
        request
            .max_tokens
            .map(|v| v.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );

    if let Some(format) = &request.response_format {
        hasher.update(b"\0format\0");
        hasher.update(format.to_string().as_bytes());
    }
    if let Some(stop) = &request.stop {
        hasher.update(b"\0stop\0");
        hasher.update(stop.to_string().as_bytes());
    }

    // Unmodelled parameters (seed, logit_bias, ...) also change the answer. `extra` is a
    // BTreeMap, so iteration order is deterministic.
    if !request.extra.is_empty() {
        hasher.update(b"\0extra\0");
        for (key, value) in &request.extra {
            hasher.update(key.as_bytes());
            hasher.update(b"\x1f");
            hasher.update(value.to_string().as_bytes());
            hasher.update(b"\x1e");
        }
    }

    Fingerprint(hex::encode(hasher.finalize()))
}

/// Format an optional float stably.
///
/// `{:?}` on an `f64` is not a stable representation across values that compare equal, so
/// a fixed-precision rendering is used instead — otherwise the same request could hash
/// two different ways.
fn format_option(value: Option<f64>) -> String {
    value.map(|v| format!("{v:.6}")).unwrap_or_default()
}

/// Why a request is or is not cacheable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cacheability {
    /// Safe to cache.
    Cacheable,
    /// The caller asked for variety.
    NonDeterministic,
    /// Tool calls have side effects; a stored answer would skip them.
    ToolUse,
    /// The organisation has opted out of retention entirely.
    ZeroRetention,
    /// Streaming responses are passed through live.
    Streaming,
}

impl Cacheability {
    /// True when the request may be cached.
    pub fn is_cacheable(self) -> bool {
        matches!(self, Cacheability::Cacheable)
    }

    /// A short reason, for the request log.
    pub fn as_str(self) -> &'static str {
        match self {
            Cacheability::Cacheable => "cacheable",
            Cacheability::NonDeterministic => "non_deterministic",
            Cacheability::ToolUse => "tool_use",
            Cacheability::ZeroRetention => "zero_retention",
            Cacheability::Streaming => "streaming",
        }
    }
}

/// Decide whether a request may be served from, or stored in, the cache.
///
/// Order matters: the organisation-level opt-out is checked first, because a
/// zero-retention tenant must never have its content considered for storage for any
/// reason.
pub fn cacheability(request: &NormalizedRequest, zero_retention: bool) -> Cacheability {
    if zero_retention {
        return Cacheability::ZeroRetention;
    }
    if request.requires_tools() {
        return Cacheability::ToolUse;
    }
    if request.stream {
        return Cacheability::Streaming;
    }
    if request
        .temperature
        .is_some_and(|t| t > CACHE_TEMPERATURE_CEILING)
    {
        return Cacheability::NonDeterministic;
    }
    Cacheability::Cacheable
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    fn org() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn other_org() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }

    #[test]
    fn identical_requests_share_a_fingerprint() {
        let a = NormalizedRequest::simple("gpt-4o", "What is 2+2?");
        let b = NormalizedRequest::simple("gpt-4o", "What is 2+2?");
        assert_eq!(compute(&a, org()), compute(&b, org()));
    }

    #[test]
    fn the_same_request_from_different_orgs_never_collides() {
        // The single most important property in this module.
        let request = NormalizedRequest::simple("gpt-4o", "What is 2+2?");
        let mine = compute(&request, org());
        let theirs = compute(&request, other_org());
        assert_ne!(mine, theirs);
        assert_ne!(mine.cache_key(org()), theirs.cache_key(other_org()));
    }

    #[test]
    fn cache_keys_are_prefixed_by_organisation() {
        let request = NormalizedRequest::simple("gpt-4o", "hi");
        let key = compute(&request, org()).cache_key(org());
        assert!(key.starts_with(&Fingerprint::org_prefix(org())));
        assert!(!key.starts_with(&Fingerprint::org_prefix(other_org())));
    }

    #[test]
    fn different_prompts_differ() {
        let a = NormalizedRequest::simple("gpt-4o", "What is 2+2?");
        let b = NormalizedRequest::simple("gpt-4o", "What is 2+3?");
        assert_ne!(compute(&a, org()), compute(&b, org()));
    }

    #[test]
    fn different_models_differ() {
        let a = NormalizedRequest::simple("gpt-4o", "hi");
        let b = NormalizedRequest::simple("gpt-4o-mini", "hi");
        assert_ne!(compute(&a, org()), compute(&b, org()));
    }

    #[test]
    fn sampling_parameters_change_the_fingerprint() {
        let base = NormalizedRequest::simple("gpt-4o", "hi");

        let warm = NormalizedRequest { temperature: Some(0.5), ..base.clone() };
        assert_ne!(compute(&base, org()), compute(&warm, org()));

        let capped = NormalizedRequest { max_tokens: Some(100), ..base.clone() };
        assert_ne!(compute(&base, org()), compute(&capped, org()));

        let nucleus = NormalizedRequest { top_p: Some(0.9), ..base.clone() };
        assert_ne!(compute(&base, org()), compute(&nucleus, org()));
    }

    #[test]
    fn message_boundaries_cannot_be_confused() {
        // Without separators, ["ab","c"] and ["a","bc"] would hash the same and one
        // conversation could be served another's answer.
        let first = NormalizedRequest {
            messages: vec![
                Message::text(Role::User, "ab"),
                Message::text(Role::User, "c"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let second = NormalizedRequest {
            messages: vec![
                Message::text(Role::User, "a"),
                Message::text(Role::User, "bc"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        assert_ne!(compute(&first, org()), compute(&second, org()));
    }

    #[test]
    fn role_changes_change_the_fingerprint() {
        let as_user = NormalizedRequest {
            messages: vec![Message::text(Role::User, "same text")],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let as_system = NormalizedRequest {
            messages: vec![Message::text(Role::System, "same text")],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        assert_ne!(compute(&as_user, org()), compute(&as_system, org()));
    }

    #[test]
    fn unmodelled_parameters_are_included() {
        // A seed changes the answer; ignoring it would return the wrong cached response.
        let mut with_seed = NormalizedRequest::simple("gpt-4o", "hi");
        with_seed.extra.insert("seed".into(), serde_json::json!(42));
        let mut other_seed = NormalizedRequest::simple("gpt-4o", "hi");
        other_seed.extra.insert("seed".into(), serde_json::json!(43));

        assert_ne!(compute(&with_seed, org()), compute(&other_seed, org()));
    }

    #[test]
    fn fingerprints_are_stable_across_calls() {
        let request = NormalizedRequest::simple("gpt-4o", "stability check");
        let first = compute(&request, org());
        for _ in 0..50 {
            assert_eq!(compute(&request, org()), first);
        }
    }

    #[test]
    fn fingerprint_is_a_full_sha256_digest() {
        let fingerprint = compute(&NormalizedRequest::simple("gpt-4o", "hi"), org());
        assert_eq!(fingerprint.as_str().len(), 64);
        assert!(fingerprint.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_does_not_leak_prompt_content() {
        // Cache keys appear in Redis, in logs, and in metrics labels.
        let request = NormalizedRequest::simple("gpt-4o", "my secret business plan");
        let key = compute(&request, org()).cache_key(org());
        assert!(!key.contains("secret"));
        assert!(!key.contains("business"));
    }

    #[test]
    fn zero_retention_orgs_are_never_cached() {
        // Checked before anything else, and it wins unconditionally.
        let request = NormalizedRequest::simple("gpt-4o", "hi");
        assert_eq!(cacheability(&request, true), Cacheability::ZeroRetention);
        assert!(!cacheability(&request, true).is_cacheable());
    }

    #[test]
    fn tool_requests_are_never_cached() {
        // A cached tool call would skip the side effect the caller wanted.
        let mut request = NormalizedRequest::simple("gpt-4o", "book a flight");
        request.tools = vec![serde_json::json!({"type": "function", "function": {"name": "book"}})];
        assert_eq!(cacheability(&request, false), Cacheability::ToolUse);
    }

    #[test]
    fn high_temperature_requests_are_never_cached() {
        let mut request = NormalizedRequest::simple("gpt-4o", "write me a poem");
        request.temperature = Some(0.9);
        assert_eq!(cacheability(&request, false), Cacheability::NonDeterministic);

        // At and below the ceiling, caching is allowed.
        request.temperature = Some(CACHE_TEMPERATURE_CEILING);
        assert_eq!(cacheability(&request, false), Cacheability::Cacheable);
        request.temperature = Some(0.0);
        assert_eq!(cacheability(&request, false), Cacheability::Cacheable);
    }

    #[test]
    fn streaming_requests_are_not_cached() {
        let request = NormalizedRequest { stream: true, ..NormalizedRequest::simple("gpt-4o", "hi") };
        assert_eq!(cacheability(&request, false), Cacheability::Streaming);
    }

    #[test]
    fn ordinary_requests_are_cacheable() {
        let request = NormalizedRequest::simple("gpt-4o", "What is the capital of France?");
        assert!(cacheability(&request, false).is_cacheable());
    }

    #[test]
    fn no_two_distinct_requests_in_a_large_sample_collide() {
        // A blunt collision check across many shapes and both tenants.
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for i in 0..1_000 {
            for org_id in [org(), other_org()] {
                let request = NormalizedRequest::simple("gpt-4o", &format!("question {i}"));
                assert!(
                    seen.insert(compute(&request, org_id).as_str().to_string()),
                    "collision at i={i}"
                );
            }
        }
        assert_eq!(seen.len(), 2_000);
    }
}

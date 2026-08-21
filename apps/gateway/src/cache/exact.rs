//! Exact-match response cache — pipeline stage [5a].
//!
//! A Redis lookup keyed by the tenant-scoped fingerprint. On a hit the request never
//! reaches a provider, so `actual_cost` is zero and the entire baseline becomes a saving —
//! the largest single win available in the product.
//!
//! Budgeted at 0.1ms: one `GET` against a multiplexed connection, and a miss costs
//! nothing but that round trip.

use crate::cache::fingerprint::{cacheability, compute, Cacheability, Fingerprint};
use crate::error::Result;
use crate::store::KvStore;
use crate::types::{NormalizedRequest, NormalizedResponse};
use std::time::Duration;
use uuid::Uuid;

/// A cached response plus the metadata needed to attribute the saving.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CachedResponse {
    /// The stored response.
    pub response: NormalizedResponse,
    /// The model that produced it, which may differ from the one requested now.
    pub served_model: String,
    /// When it was stored, as a Unix timestamp.
    pub stored_at: i64,
}

impl CachedResponse {
    /// Age in seconds.
    pub fn age_seconds(&self) -> i64 {
        (chrono::Utc::now().timestamp() - self.stored_at).max(0)
    }
}

/// The exact-match cache.
pub struct ExactCache<'a> {
    store: &'a dyn KvStore,
    ttl: Duration,
}

impl<'a> ExactCache<'a> {
    /// Construct with a store and a time to live.
    pub fn new(store: &'a dyn KvStore, ttl: Duration) -> ExactCache<'a> {
        ExactCache { store, ttl }
    }

    /// Look up a request.
    ///
    /// Returns `Ok(None)` both for a miss and for an uncacheable request, so the caller
    /// has one path. A corrupt entry is also a miss: a deserialization failure must cost
    /// one upstream call, never a failed request.
    pub async fn get(
        &self,
        request: &NormalizedRequest,
        org_id: Uuid,
        zero_retention: bool,
    ) -> Result<Option<CachedResponse>> {
        if !cacheability(request, zero_retention).is_cacheable() {
            return Ok(None);
        }

        let key = compute(request, org_id).cache_key(org_id);
        let Some(raw) = self.store.get(&key).await? else {
            return Ok(None);
        };

        match serde_json::from_str::<CachedResponse>(&raw) {
            Ok(cached) => Ok(Some(cached)),
            Err(e) => {
                tracing::warn!(error = %e, "discarding malformed cache entry");
                let _ = self.store.del(&key).await;
                Ok(None)
            }
        }
    }

    /// Store a response.
    ///
    /// Silently does nothing for an uncacheable request, so callers need no branch.
    pub async fn put(
        &self,
        request: &NormalizedRequest,
        org_id: Uuid,
        zero_retention: bool,
        response: &NormalizedResponse,
        served_model: &str,
    ) -> Result<bool> {
        if !cacheability(request, zero_retention).is_cacheable() {
            return Ok(false);
        }

        let entry = CachedResponse {
            response: response.clone(),
            served_model: served_model.to_string(),
            stored_at: chrono::Utc::now().timestamp(),
        };

        let payload = serde_json::to_string(&entry)
            .map_err(|e| crate::error::AegisError::Internal(format!("cache encode: {e}")))?;

        let key = compute(request, org_id).cache_key(org_id);
        self.store.set_ex(&key, &payload, self.ttl).await?;
        Ok(true)
    }

    /// Drop every entry for one organisation.
    ///
    /// Called when settings change in a way that invalidates stored answers, and exposed
    /// so a customer can clear their own cache on demand. Scoped by prefix, so no other
    /// tenant is affected.
    pub async fn invalidate_org(&self, org_id: Uuid) -> Result<u64> {
        self.store
            .del_prefix(&Fingerprint::org_prefix(org_id))
            .await
    }

    /// Why a request was not cached, for the request log.
    pub fn skip_reason(request: &NormalizedRequest, zero_retention: bool) -> Cacheability {
        cacheability(request, zero_retention)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use crate::types::TokenUsage;

    fn org() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn other_org() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }

    fn response(content: &str) -> NormalizedResponse {
        NormalizedResponse {
            id: "resp-1".to_string(),
            model: "openai/gpt-4o-mini".to_string(),
            content: content.to_string(),
            finish_reason: Some("stop".to_string()),
            tool_calls: None,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                estimated: false,
            },
            raw: None,
        }
    }

    #[tokio::test]
    async fn a_stored_response_is_returned_on_the_next_identical_request() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));
        let request = NormalizedRequest::simple("gpt-4o", "What is 2+2?");

        assert!(cache.get(&request, org(), false).await.unwrap().is_none());

        cache
            .put(&request, org(), false, &response("4"), "openai/gpt-4o-mini")
            .await
            .unwrap();

        let hit = cache.get(&request, org(), false).await.unwrap().unwrap();
        assert_eq!(hit.response.content, "4");
        assert_eq!(hit.served_model, "openai/gpt-4o-mini");
    }

    #[tokio::test]
    async fn one_org_can_never_read_another_orgs_cache() {
        // Part 13 item 2. If this test ever fails, stop and treat it as an incident.
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));
        let request = NormalizedRequest::simple("gpt-4o", "our confidential roadmap");

        cache
            .put(
                &request,
                org(),
                false,
                &response("secret answer"),
                "openai/gpt-4o",
            )
            .await
            .unwrap();

        let leaked = cache.get(&request, other_org(), false).await.unwrap();
        assert!(leaked.is_none(), "cross-tenant cache leak");
    }

    #[tokio::test]
    async fn invalidating_one_org_leaves_others_intact() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));
        let request = NormalizedRequest::simple("gpt-4o", "shared question");

        cache
            .put(&request, org(), false, &response("a"), "m")
            .await
            .unwrap();
        cache
            .put(&request, other_org(), false, &response("b"), "m")
            .await
            .unwrap();

        let removed = cache.invalidate_org(org()).await.unwrap();
        assert_eq!(removed, 1);

        assert!(cache.get(&request, org(), false).await.unwrap().is_none());
        assert!(cache
            .get(&request, other_org(), false)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn different_requests_do_not_hit() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));

        let first = NormalizedRequest::simple("gpt-4o", "What is 2+2?");
        cache
            .put(&first, org(), false, &response("4"), "m")
            .await
            .unwrap();

        let second = NormalizedRequest::simple("gpt-4o", "What is 2+3?");
        assert!(cache.get(&second, org(), false).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn entries_expire() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_millis(20));
        let request = NormalizedRequest::simple("gpt-4o", "hi");

        cache
            .put(&request, org(), false, &response("hello"), "m")
            .await
            .unwrap();
        assert!(cache.get(&request, org(), false).await.unwrap().is_some());

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(cache.get(&request, org(), false).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn zero_retention_orgs_neither_read_nor_write() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));
        let request = NormalizedRequest::simple("gpt-4o", "hi");

        assert!(!cache
            .put(&request, org(), true, &response("x"), "m")
            .await
            .unwrap());
        assert!(cache.get(&request, org(), true).await.unwrap().is_none());

        // And nothing was written that a later non-zero-retention read could find.
        assert!(cache.get(&request, org(), false).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tool_and_high_temperature_requests_are_not_stored() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));

        let mut tools = NormalizedRequest::simple("gpt-4o", "book a flight");
        tools.tools = vec![serde_json::json!({"type": "function", "function": {"name": "b"}})];
        assert!(!cache
            .put(&tools, org(), false, &response("booked"), "m")
            .await
            .unwrap());

        let mut creative = NormalizedRequest::simple("gpt-4o", "write a poem");
        creative.temperature = Some(0.95);
        assert!(!cache
            .put(&creative, org(), false, &response("poem"), "m")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn a_corrupt_entry_is_a_miss_not_an_error() {
        // A deserialization failure must cost one upstream call, never a failed request.
        let store = MemoryStore::new();
        let request = NormalizedRequest::simple("gpt-4o", "hi");
        let key = compute(&request, org()).cache_key(org());
        store
            .set_ex(&key, "{not valid json", Duration::from_secs(60))
            .await
            .unwrap();

        let cache = ExactCache::new(&store, Duration::from_secs(60));
        assert!(cache.get(&request, org(), false).await.unwrap().is_none());
        // And the bad entry is cleaned up rather than being re-read forever.
        assert!(store.get(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cached_entries_report_their_age() {
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));
        let request = NormalizedRequest::simple("gpt-4o", "hi");

        cache
            .put(&request, org(), false, &response("x"), "m")
            .await
            .unwrap();
        let hit = cache.get(&request, org(), false).await.unwrap().unwrap();
        assert!(hit.age_seconds() >= 0);
        assert!(hit.age_seconds() < 5);
    }

    #[tokio::test]
    async fn the_full_response_survives_a_round_trip() {
        // Everything a client depends on must come back intact, including the raw body.
        let store = MemoryStore::new();
        let cache = ExactCache::new(&store, Duration::from_secs(60));
        let request = NormalizedRequest::simple("gpt-4o", "hi");

        let mut original = response("full answer");
        original.raw = Some(serde_json::json!({"system_fingerprint": "fp_1"}));
        original.tool_calls = Some(serde_json::json!([{"id": "call_1"}]));

        cache
            .put(&request, org(), false, &original, "openai/gpt-4o")
            .await
            .unwrap();
        let hit = cache.get(&request, org(), false).await.unwrap().unwrap();

        assert_eq!(hit.response, original);
        assert_eq!(hit.response.raw.unwrap()["system_fingerprint"], "fp_1");
    }

    #[tokio::test]
    async fn skip_reason_explains_why_nothing_was_cached() {
        let mut request = NormalizedRequest::simple("gpt-4o", "hi");
        assert_eq!(
            ExactCache::skip_reason(&request, false).as_str(),
            "cacheable"
        );
        request.temperature = Some(0.99);
        assert_eq!(
            ExactCache::skip_reason(&request, false).as_str(),
            "non_deterministic"
        );
        assert_eq!(
            ExactCache::skip_reason(&request, true).as_str(),
            "zero_retention"
        );
    }
}

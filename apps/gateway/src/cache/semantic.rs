//! Semantic response cache — pipeline stage [5b].
//!
//! The exact cache only fires on byte-identical requests. In practice people ask the same
//! question in slightly different words, and an agent loop re-asks with a reworded prompt
//! on every iteration. The semantic cache catches those: embed the request, find a stored
//! request within a cosine similarity threshold, and reuse its answer.
//!
//! # Why the threshold is so high
//!
//! The default is 0.95, which is strict. Below roughly that, embeddings start treating
//! "how do I *enable* X" and "how do I *disable* X" as near-neighbours — semantically
//! opposite questions with very similar vectors. Returning a stored answer for the
//! opposite question is worse than any cache miss, so the threshold is set where false
//! positives become rare rather than where hit rate is maximised.
//!
//! # Tenant isolation
//!
//! Each organisation gets its own Qdrant collection, named from its id. A query cannot
//! reach another tenant's vectors, for the same structural reason the exact cache keys
//! are org-prefixed.

use crate::error::{AegisError, Result};
use crate::types::{NormalizedRequest, NormalizedResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default cosine similarity required for a hit.
pub const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.95;

/// An embedding vector.
pub type Embedding = Vec<f32>;

/// A stored semantic entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticEntry {
    pub id: Uuid,
    /// The response to reuse.
    pub response: NormalizedResponse,
    pub served_model: String,
    pub stored_at: i64,
}

/// A match returned by a similarity search.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticHit {
    pub entry: SemanticEntry,
    pub similarity: f32,
}

/// Cosine similarity between two vectors.
///
/// Returns 0.0 for mismatched lengths or a zero vector rather than `NaN`: a `NaN`
/// propagating into a threshold comparison silently makes every comparison false, which
/// would disable the cache without any error appearing anywhere.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        return 0.0;
    }
    let similarity = dot / (norm_a.sqrt() * norm_b.sqrt());
    // Floating point can push a self-comparison a hair past 1.0.
    similarity.clamp(-1.0, 1.0)
}

/// The text embedded for a request.
///
/// Only the system prompt and the final user message: earlier turns add noise that pulls
/// unrelated conversations together, and the last question is what the answer responds to.
pub fn embedding_text(request: &NormalizedRequest) -> String {
    let system = request.system_text();
    let last_user = request.last_user_message().unwrap_or_default();
    if system.is_empty() {
        last_user
    } else {
        format!("{system}\n\n{last_user}")
    }
}

/// The Qdrant collection backing one organisation.
pub fn collection_name(org_id: Uuid) -> String {
    format!("aegis_cache_{}", org_id.simple())
}

/// Vector storage for the semantic cache.
///
/// A trait so the pipeline can be tested without Qdrant, and so a different vector store
/// can be substituted without touching the cache logic.
#[async_trait::async_trait]
pub trait VectorStore: Send + Sync {
    /// Nearest neighbour within `org_id`'s collection, if any meets `threshold`.
    async fn search(
        &self,
        org_id: Uuid,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Option<SemanticHit>>;

    /// Store an entry.
    async fn upsert(&self, org_id: Uuid, embedding: Embedding, entry: SemanticEntry) -> Result<()>;

    /// Delete an organisation's collection.
    async fn drop_collection(&self, org_id: Uuid) -> Result<()>;

    /// Number of stored vectors for an organisation.
    async fn count(&self, org_id: Uuid) -> Result<usize>;
}

/// In-memory vector store.
///
/// Used in development and tests, and correct for a single instance. Linear scan, so it
/// is only appropriate at small scale — Qdrant handles production.
#[derive(Default)]
pub struct MemoryVectorStore {
    collections: dashmap::DashMap<Uuid, Vec<(Embedding, SemanticEntry)>>,
}

impl MemoryVectorStore {
    /// An empty store.
    pub fn new() -> MemoryVectorStore {
        MemoryVectorStore::default()
    }
}

#[async_trait::async_trait]
impl VectorStore for MemoryVectorStore {
    async fn search(
        &self,
        org_id: Uuid,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Option<SemanticHit>> {
        let Some(collection) = self.collections.get(&org_id) else {
            return Ok(None);
        };

        let best = collection
            .iter()
            .map(|(vector, entry)| (cosine_similarity(vector, embedding), entry))
            .filter(|(similarity, _)| *similarity >= threshold)
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(best.map(|(similarity, entry)| SemanticHit {
            entry: entry.clone(),
            similarity,
        }))
    }

    async fn upsert(&self, org_id: Uuid, embedding: Embedding, entry: SemanticEntry) -> Result<()> {
        self.collections
            .entry(org_id)
            .or_default()
            .push((embedding, entry));
        Ok(())
    }

    async fn drop_collection(&self, org_id: Uuid) -> Result<()> {
        self.collections.remove(&org_id);
        Ok(())
    }

    async fn count(&self, org_id: Uuid) -> Result<usize> {
        Ok(self.collections.get(&org_id).map(|c| c.len()).unwrap_or(0))
    }
}

/// Qdrant-backed vector store.
pub struct QdrantVectorStore {
    base_url: String,
    http: reqwest::Client,
}

impl QdrantVectorStore {
    /// Point at a Qdrant instance.
    pub fn new(base_url: impl Into<String>, http: reqwest::Client) -> QdrantVectorStore {
        QdrantVectorStore {
            base_url: base_url.into(),
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }
}

#[async_trait::async_trait]
impl VectorStore for QdrantVectorStore {
    async fn search(
        &self,
        org_id: Uuid,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Option<SemanticHit>> {
        let collection = collection_name(org_id);
        let body = serde_json::json!({
            "vector": embedding,
            "limit": 1,
            "score_threshold": threshold,
            "with_payload": true,
        });

        let response = self
            .http
            .post(self.url(&format!("/collections/{collection}/points/search")))
            .json(&body)
            .send()
            .await
            .map_err(|e| AegisError::Store(format!("qdrant search: {e}")))?;

        // A missing collection is simply an organisation with no cache yet.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(AegisError::Store(format!(
                "qdrant search returned {}",
                response.status()
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AegisError::Store(format!("qdrant decode: {e}")))?;

        let Some(point) = json.pointer("/result/0") else {
            return Ok(None);
        };
        let similarity = point.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0) as f32;
        let Some(payload) = point.get("payload") else {
            return Ok(None);
        };

        match serde_json::from_value::<SemanticEntry>(payload.clone()) {
            Ok(entry) => Ok(Some(SemanticHit { entry, similarity })),
            Err(e) => {
                tracing::warn!(error = %e, "discarding malformed semantic cache payload");
                Ok(None)
            }
        }
    }

    async fn upsert(&self, org_id: Uuid, embedding: Embedding, entry: SemanticEntry) -> Result<()> {
        let collection = collection_name(org_id);
        // Create on first write; Qdrant treats an existing collection as a conflict,
        // which is fine to ignore.
        let _ = self
            .http
            .put(self.url(&format!("/collections/{collection}")))
            .json(&serde_json::json!({
                "vectors": {"size": embedding.len(), "distance": "Cosine"}
            }))
            .send()
            .await;

        let body = serde_json::json!({
            "points": [{
                "id": entry.id.to_string(),
                "vector": embedding,
                "payload": entry,
            }]
        });

        let response = self
            .http
            .put(self.url(&format!("/collections/{collection}/points")))
            .json(&body)
            .send()
            .await
            .map_err(|e| AegisError::Store(format!("qdrant upsert: {e}")))?;

        if !response.status().is_success() {
            return Err(AegisError::Store(format!(
                "qdrant upsert returned {}",
                response.status()
            )));
        }
        Ok(())
    }

    async fn drop_collection(&self, org_id: Uuid) -> Result<()> {
        let collection = collection_name(org_id);
        self.http
            .delete(self.url(&format!("/collections/{collection}")))
            .send()
            .await
            .map_err(|e| AegisError::Store(format!("qdrant drop: {e}")))?;
        Ok(())
    }

    async fn count(&self, org_id: Uuid) -> Result<usize> {
        let collection = collection_name(org_id);
        let response = self
            .http
            .post(self.url(&format!("/collections/{collection}/points/count")))
            .json(&serde_json::json!({"exact": true}))
            .send()
            .await
            .map_err(|e| AegisError::Store(format!("qdrant count: {e}")))?;

        if !response.status().is_success() {
            return Ok(0);
        }
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AegisError::Store(format!("qdrant decode: {e}")))?;
        Ok(json
            .pointer("/result/count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as usize)
    }
}

/// The semantic cache.
pub struct SemanticCache<'a> {
    store: &'a dyn VectorStore,
    threshold: f32,
}

impl<'a> SemanticCache<'a> {
    /// Construct with a vector store and similarity threshold.
    pub fn new(store: &'a dyn VectorStore, threshold: f32) -> SemanticCache<'a> {
        SemanticCache { store, threshold }
    }

    /// Look up a request by embedding.
    pub async fn get(
        &self,
        embedding: &[f32],
        org_id: Uuid,
        zero_retention: bool,
    ) -> Result<Option<SemanticHit>> {
        if zero_retention {
            return Ok(None);
        }
        self.store.search(org_id, embedding, self.threshold).await
    }

    /// Store a response against its embedding.
    pub async fn put(
        &self,
        embedding: Embedding,
        org_id: Uuid,
        zero_retention: bool,
        response: &NormalizedResponse,
        served_model: &str,
    ) -> Result<bool> {
        if zero_retention {
            return Ok(false);
        }
        let entry = SemanticEntry {
            id: Uuid::new_v4(),
            response: response.clone(),
            served_model: served_model.to_string(),
            stored_at: chrono::Utc::now().timestamp(),
        };
        self.store.upsert(org_id, embedding, entry).await?;
        Ok(true)
    }

    /// Drop an organisation's collection.
    pub async fn invalidate_org(&self, org_id: Uuid) -> Result<()> {
        self.store.drop_collection(org_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role, TokenUsage};

    fn org() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn other_org() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }

    fn response(content: &str) -> NormalizedResponse {
        NormalizedResponse {
            id: "r".into(),
            model: "openai/gpt-4o-mini".into(),
            content: content.into(),
            finish_reason: Some("stop".into()),
            tool_calls: None,
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 5,
                estimated: false,
            },
            raw: None,
        }
    }

    #[test]
    fn identical_vectors_have_similarity_one() {
        let v = vec![0.1, 0.2, 0.3, 0.4];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_have_similarity_zero() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_have_similarity_minus_one() {
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn degenerate_inputs_return_zero_not_nan() {
        // A NaN here would make every threshold comparison false and silently disable the
        // cache with no error anywhere.
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert!(cosine_similarity(&[0.0, 0.0], &[0.0, 0.0]).is_finite());
    }

    #[test]
    fn similarity_is_bounded_even_with_floating_point_error() {
        let v = vec![1e-3f32; 512];
        let similarity = cosine_similarity(&v, &v);
        assert!(
            (-1.0..=1.0).contains(&similarity),
            "out of range: {similarity}"
        );
    }

    #[test]
    fn embedding_text_uses_the_system_prompt_and_last_question() {
        let request = NormalizedRequest {
            messages: vec![
                Message::text(Role::System, "You are terse."),
                Message::text(Role::User, "old question"),
                Message::text(Role::Assistant, "old answer"),
                Message::text(Role::User, "current question"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let text = embedding_text(&request);
        assert!(text.contains("You are terse."));
        assert!(text.contains("current question"));
        // Earlier turns would pull unrelated conversations together.
        assert!(!text.contains("old answer"));
    }

    #[test]
    fn collections_are_named_per_organisation() {
        assert_ne!(collection_name(org()), collection_name(other_org()));
        assert!(collection_name(org()).starts_with("aegis_cache_"));
        // No hyphens: Qdrant collection names are friendlier without them.
        assert!(!collection_name(org()).contains('-'));
    }

    #[tokio::test]
    async fn a_near_identical_request_hits() {
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, DEFAULT_SIMILARITY_THRESHOLD);

        let stored = vec![1.0, 0.0, 0.0];
        cache
            .put(
                stored.clone(),
                org(),
                false,
                &response("Paris"),
                "openai/gpt-4o-mini",
            )
            .await
            .unwrap();

        // Slightly rotated, still well above threshold.
        let query = vec![0.999, 0.045, 0.0];
        let hit = cache.get(&query, org(), false).await.unwrap().unwrap();
        assert_eq!(hit.entry.response.content, "Paris");
        assert!(hit.similarity >= DEFAULT_SIMILARITY_THRESHOLD);
    }

    #[tokio::test]
    async fn a_dissimilar_request_misses() {
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, DEFAULT_SIMILARITY_THRESHOLD);

        cache
            .put(vec![1.0, 0.0, 0.0], org(), false, &response("Paris"), "m")
            .await
            .unwrap();
        assert!(cache
            .get(&[0.0, 1.0, 0.0], org(), false)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn the_threshold_boundary_is_respected() {
        let store = MemoryVectorStore::new();
        cache_put(&store, vec![1.0, 0.0]).await;

        // cos(theta) = 0.94 — just under the bar, so it must not hit.
        let angle = 0.94f32.acos();
        let just_below = vec![angle.cos(), angle.sin()];
        let strict = SemanticCache::new(&store, 0.95);
        assert!(strict
            .get(&just_below, org(), false)
            .await
            .unwrap()
            .is_none());

        // Relaxing the threshold lets the same vector through.
        let lenient = SemanticCache::new(&store, 0.90);
        assert!(lenient
            .get(&just_below, org(), false)
            .await
            .unwrap()
            .is_some());
    }

    async fn cache_put(store: &MemoryVectorStore, embedding: Embedding) {
        SemanticCache::new(store, 0.95)
            .put(embedding, org(), false, &response("stored"), "m")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn one_org_cannot_match_another_orgs_vectors() {
        // The same isolation guarantee as the exact cache, enforced by collection.
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, DEFAULT_SIMILARITY_THRESHOLD);

        cache
            .put(
                vec![1.0, 0.0, 0.0],
                org(),
                false,
                &response("confidential"),
                "m",
            )
            .await
            .unwrap();

        assert!(cache
            .get(&[1.0, 0.0, 0.0], other_org(), false)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn the_best_match_is_returned_when_several_qualify() {
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, 0.5);

        cache
            .put(vec![1.0, 0.0], org(), false, &response("closer"), "m")
            .await
            .unwrap();
        cache
            .put(vec![0.7, 0.7], org(), false, &response("further"), "m")
            .await
            .unwrap();

        let hit = cache.get(&[1.0, 0.0], org(), false).await.unwrap().unwrap();
        assert_eq!(hit.entry.response.content, "closer");
    }

    #[tokio::test]
    async fn zero_retention_orgs_neither_read_nor_write() {
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, DEFAULT_SIMILARITY_THRESHOLD);

        assert!(!cache
            .put(vec![1.0, 0.0], org(), true, &response("x"), "m")
            .await
            .unwrap());
        assert_eq!(store.count(org()).await.unwrap(), 0);
        assert!(cache.get(&[1.0, 0.0], org(), true).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalidation_drops_only_the_named_collection() {
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, 0.5);

        cache
            .put(vec![1.0, 0.0], org(), false, &response("a"), "m")
            .await
            .unwrap();
        cache
            .put(vec![1.0, 0.0], other_org(), false, &response("b"), "m")
            .await
            .unwrap();

        cache.invalidate_org(org()).await.unwrap();
        assert_eq!(store.count(org()).await.unwrap(), 0);
        assert_eq!(store.count(other_org()).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn an_empty_collection_is_a_miss_not_an_error() {
        let store = MemoryVectorStore::new();
        let cache = SemanticCache::new(&store, DEFAULT_SIMILARITY_THRESHOLD);
        assert!(cache
            .get(&[1.0, 0.0], org(), false)
            .await
            .unwrap()
            .is_none());
    }
}

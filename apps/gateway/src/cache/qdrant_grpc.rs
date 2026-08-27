//! Qdrant over gRPC — a second `VectorStore` implementation, alongside the REST-based one
//! in `cache::semantic`.
//!
//! # Why this exists
//!
//! `cache::semantic::QdrantVectorStore` talks REST/JSON over HTTP/1.1. That's correct and
//! already tenant-isolated the same way this module is (one collection per organisation),
//! but every search sends a ~384-float vector as JSON text — several kilobytes of
//! human-readable digits — parsed on both ends, instead of protobuf's compact binary
//! encoding. Once the embedding side of the semantic cache stopped being the dominant cost
//! (see `cache::onnx_embed`), Qdrant's own round trip became a proportionally bigger slice
//! of what's left, which is what makes this worth doing now rather than leaving deferred.
//!
//! # Why the REST implementation is not deleted
//!
//! It still works, it's still tested, and `QDRANT_GRPC_URL` is a separate, optional
//! setting — an operator who hasn't opened the gRPC port yet keeps running on REST with no
//! code change required. `AppState::semantic_store` picks whichever this config implies;
//! see `main.rs`.
//!
//! # Payload shape
//!
//! Rather than hand-map every field of [`SemanticEntry`] to Qdrant's protobuf `Value`
//! type, this stores it as a single JSON-encoded string field — the same trick
//! `cache::durable` uses for its encrypted blob. Simpler, and the two `VectorStore`
//! implementations then agree on payload shape trivially, since JSON-in-a-field is exactly
//! what the REST client already sends as Qdrant's whole payload.

use crate::cache::semantic::{Embedding, SemanticEntry, SemanticHit, VectorStore};
use crate::error::{AegisError, Result};
use async_trait::async_trait;
use qdrant_client::qdrant::{
    CountPointsBuilder, CreateCollectionBuilder, Distance, PointStruct, SearchPointsBuilder,
    UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::{Payload, Qdrant};
use uuid::Uuid;

/// The payload key the whole [`SemanticEntry`] is stored under, JSON-encoded.
const ENTRY_KEY: &str = "entry";

/// Qdrant accessed over its native gRPC port (default 6334) instead of REST (default 6333).
pub struct QdrantGrpcVectorStore {
    client: Qdrant,
}

impl QdrantGrpcVectorStore {
    /// Connect to a Qdrant gRPC endpoint, e.g. `http://localhost:6334`.
    pub fn connect(url: impl AsRef<str>) -> Result<QdrantGrpcVectorStore> {
        let client = Qdrant::from_url(url.as_ref())
            .build()
            .map_err(|e| AegisError::Store(format!("qdrant grpc client: {e}")))?;
        Ok(QdrantGrpcVectorStore { client })
    }

    /// Ensure the collection exists, matching `cache::semantic::QdrantVectorStore`'s
    /// create-on-first-write behaviour. Qdrant treats re-creating an existing collection
    /// as a conflict, which this ignores the same way the REST client does.
    async fn ensure_collection(&self, org_id: Uuid, vector_len: usize) {
        let collection = crate::cache::semantic::collection_name(org_id);
        let _ = self
            .client
            .create_collection(CreateCollectionBuilder::new(collection).vectors_config(
                VectorParamsBuilder::new(vector_len as u64, Distance::Cosine),
            ))
            .await;
    }
}

#[async_trait]
impl VectorStore for QdrantGrpcVectorStore {
    async fn search(
        &self,
        org_id: Uuid,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Option<SemanticHit>> {
        let collection = crate::cache::semantic::collection_name(org_id);

        let response = match self
            .client
            .search_points(
                SearchPointsBuilder::new(collection, embedding.to_vec(), 1)
                    .score_threshold(threshold)
                    .with_payload(true),
            )
            .await
        {
            Ok(response) => response,
            // A collection that doesn't exist yet is simply an organisation with no cache
            // entries — the same "miss, not an error" contract the REST client's 404
            // handling makes, just reached through gRPC's status-code shape instead.
            Err(_) => return Ok(None),
        };

        let Some(point) = response.result.into_iter().next() else {
            return Ok(None);
        };
        let similarity = point.score;

        let Some(raw) = point.payload.get(ENTRY_KEY).and_then(|v| v.as_str()) else {
            return Ok(None);
        };
        match serde_json::from_str::<SemanticEntry>(raw) {
            Ok(entry) => Ok(Some(SemanticHit { entry, similarity })),
            Err(e) => {
                tracing::warn!(error = %e, "discarding malformed semantic cache payload");
                Ok(None)
            }
        }
    }

    async fn upsert(&self, org_id: Uuid, embedding: Embedding, entry: SemanticEntry) -> Result<()> {
        self.ensure_collection(org_id, embedding.len()).await;
        let collection = crate::cache::semantic::collection_name(org_id);

        let raw = serde_json::to_string(&entry)
            .map_err(|e| AegisError::Internal(format!("semantic cache encode: {e}")))?;
        let payload: Payload = serde_json::json!({ ENTRY_KEY: raw })
            .try_into()
            .map_err(|e| AegisError::Internal(format!("qdrant payload encode: {e}")))?;

        let point = PointStruct::new(entry.id.to_string(), embedding, payload);

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection, vec![point]))
            .await
            .map_err(|e| AegisError::Store(format!("qdrant grpc upsert: {e}")))?;
        Ok(())
    }

    async fn drop_collection(&self, org_id: Uuid) -> Result<()> {
        let collection = crate::cache::semantic::collection_name(org_id);
        self.client
            .delete_collection(collection)
            .await
            .map_err(|e| AegisError::Store(format!("qdrant grpc drop: {e}")))?;
        Ok(())
    }

    async fn count(&self, org_id: Uuid) -> Result<usize> {
        let collection = crate::cache::semantic::collection_name(org_id);
        match self
            .client
            .count(CountPointsBuilder::new(collection).exact(true))
            .await
        {
            Ok(response) => Ok(response.result.map(|r| r.count as usize).unwrap_or(0)),
            // A missing collection counts as zero, matching the REST client.
            Err(_) => Ok(0),
        }
    }
}

//! Embedding generation for the semantic cache — pipeline stage [5b]'s input.
//!
//! # Whose bill this rides on
//!
//! Deliberately **never** the customer's own BYOK credential. Two reasons: a customer
//! whose only configured provider is Anthropic (no embeddings endpoint) would otherwise
//! get no semantic caching at all, and no customer should see an unexplained line item on
//! their own OpenAI invoice for a call Aegis made on its own initiative. This always uses
//! Aegis's own pooled key (`providers::pool::SharedKeyPool`) — the same mechanism the free
//! tier's chat traffic already uses — so the cost is Aegis's own operating cost, bundled
//! into the plan price the way Redis and Postgres compute already are, not billed per call.
//!
//! # Why this is a trait
//!
//! A live embedding call is real network I/O with no local test double worth building
//! against (unlike `cache::semantic::VectorStore`, which has a meaningful in-memory
//! implementation). Rather than leave the whole semantic-cache pipeline untestable, the
//! generation step itself is behind [`Embedder`], so a test can supply
//! [`DeterministicEmbedder`] and exercise the real lookup/promote/store logic around it.

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::Config;
use crate::metering::pricing::PricingTable;
use crate::providers::pool::SharedKeyPool;
use crate::providers::{Credential, ProviderRegistry};

/// Generates an embedding for cache-lookup purposes.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed `text`, or return `None` — never an error — when no embedding provider is
    /// configured or the call failed. A missing embedding degrades semantic caching to
    /// "skip it for this request," never to a failed request.
    async fn embed(&self, text: &str) -> Option<Vec<f32>>;
}

/// The real implementation: calls out to whichever provider has the cheapest embedding
/// model in the pricing table. Prefers Aegis's own pooled credential (`SharedKeyPool`),
/// but falls back to a BYOK credential from the database when no pooled key exists —
/// the common case in local development, where the operator's own API key stored in the
/// dashboard is the only credential available.
pub struct ProviderEmbedder {
    pricing: Arc<PricingTable>,
    providers: Arc<ProviderRegistry>,
    shared_pool: Arc<SharedKeyPool>,
    config: Arc<Config>,
    http: reqwest::Client,
    /// Optional database pool for BYOK credential fallback.
    db: Option<sqlx::PgPool>,
    /// Master key for decrypting stored credentials.
    master_key: [u8; 32],
}

impl ProviderEmbedder {
    /// Construct from the same handles `AppState` already holds — cloned once at startup,
    /// not borrowed from it, so this can live in `AppState` as its own field.
    pub fn new(
        pricing: Arc<PricingTable>,
        providers: Arc<ProviderRegistry>,
        shared_pool: Arc<SharedKeyPool>,
        config: Arc<Config>,
        http: reqwest::Client,
        db: Option<sqlx::PgPool>,
    ) -> ProviderEmbedder {
        let master_key = config.master_key;
        ProviderEmbedder {
            pricing,
            providers,
            shared_pool,
            config,
            http,
            db,
            master_key,
        }
    }

    /// Resolve a credential for the given provider: shared pool first, BYOK fallback.
    async fn resolve_credential(&self, provider_id: &str) -> Option<Credential> {
        // Try the shared pool first (production path).
        if let Ok(cred) = self.shared_pool.next_credential(&self.config, provider_id) {
            return Some(cred);
        }

        // Fallback: look for any BYOK credential in the database.
        let pool = self.db.as_ref()?;
        let rows = sqlx::query_as::<_, (Vec<u8>, Option<String>)>(
            "SELECT encrypted_key, base_url FROM provider_credentials \
             WHERE provider = $1 ORDER BY is_default DESC, created_at LIMIT 1",
        )
        .bind(provider_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()?;

        let plaintext = crate::crypto::decrypt(&self.master_key, &rows.0).ok()?;
        let key = String::from_utf8(plaintext).ok()?;
        tracing::debug!(
            provider = provider_id,
            "using BYOK credential for embedding (no shared pool key configured)"
        );
        Some(match rows.1 {
            Some(base_url) => Credential::with_base_url(key, base_url),
            None => Credential::new(key),
        })
    }
}

#[async_trait]
impl Embedder for ProviderEmbedder {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if text.trim().is_empty() {
            return None;
        }

        // Try all embedding models in order of cost, using whichever has an available credential.
        let embedding_models: Vec<_> = self.pricing.all()
            .filter(|m| !m.supports_chat)
            .collect();

        for model_pricing in &embedding_models {
            let model = &model_pricing.model_id;
            let provider = match self.providers.for_model(model) {
                Some(p) => p,
                None => continue,
            };

            let credential = match self.resolve_credential(provider.id()).await {
                Some(c) => c,
                None => continue,
            };

            let base = credential
                .base_url
                .as_deref()
                .unwrap_or_else(|| provider.default_base_url());
            let url = format!("{}/embeddings", base.trim_end_matches('/'));
            let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);

            let mut builder = self.http.post(&url).json(&serde_json::json!({
                "model": bare,
                "input": text,
            }));
            for (name, value) in provider.auth_headers(&credential) {
                builder = builder.header(name, value);
            }

            let response = match builder.send().await {
                Ok(response) => response,
                Err(e) => {
                    tracing::warn!(error = %e, %model, "embedding call failed, trying next model");
                    continue;
                }
            };
            if !response.status().is_success() {
                tracing::warn!(status = %response.status(), %model, "embedding provider returned an error, trying next model");
                continue;
            }

            let json: serde_json::Value = match response.json().await {
                Ok(j) => j,
                Err(_) => continue,
            };
            if let Some(embedding) = extract_embedding(&json) {
                tracing::debug!(%model, dims = embedding.len(), "embedding generated for semantic cache");
                return Some(embedding);
            }
        }

        tracing::debug!("no embedding provider available — semantic cache skipped");
        None
    }
}


/// Pull the first embedding vector out of an OpenAI-shaped `/embeddings` response.
///
/// Split out as a pure function so the parsing logic is unit-testable without a network
/// call — only the HTTP round trip itself is untested, the same honest gap
/// `cache::semantic::QdrantVectorStore` already has for the same reason.
fn extract_embedding(json: &serde_json::Value) -> Option<Vec<f32>> {
    let array = json.pointer("/data/0/embedding")?.as_array()?;
    let embedding: Vec<f32> = array
        .iter()
        .filter_map(|v| v.as_f64())
        .map(|v| v as f32)
        .collect();
    if embedding.is_empty() || embedding.len() != array.len() {
        None
    } else {
        Some(embedding)
    }
}

/// A deterministic embedder for tests.
///
/// Hashes the input text into a small fixed-size vector so identical text always produces
/// an identical vector — enough to exercise the exact-similarity path of the pipeline.
/// Deliberately does not attempt to approximate real semantic similarity between different
/// text (`cache::semantic::cosine_similarity`'s own tests already cover the similarity
/// math); this only needs to prove the wiring, not the embedding quality.
#[derive(Default)]
pub struct DeterministicEmbedder;

#[async_trait]
impl Embedder for DeterministicEmbedder {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if text.trim().is_empty() {
            return None;
        }
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let seed = hasher.finish();

        // 8 dimensions derived from the hash bytes, normalised — enough for
        // `cosine_similarity` to behave sensibly without pulling in a real model.
        let bytes = seed.to_le_bytes();
        let vector: Vec<f32> = bytes.iter().map(|b| (*b as f32) / 255.0 + 0.01).collect();
        Some(vector)
    }
}

/// Never produces an embedding. The default for `AppState::for_tests()`, so every
/// existing test's behaviour is unchanged unless it explicitly opts into
/// [`DeterministicEmbedder`].
#[derive(Default)]
pub struct NullEmbedder;

#[async_trait]
impl Embedder for NullEmbedder {
    async fn embed(&self, _text: &str) -> Option<Vec<f32>> {
        None
    }
}

/// Returns the same fixed vector regardless of input text.
///
/// For proving the pipeline actually reaches the semantic-cache path on two genuinely
/// *different* wordings — a real embedder would put two paraphrases close but not
/// identical, which `cosine_similarity`'s own tests already cover in isolation. This
/// stands in for "two different phrasings that a real embedder judged near-identical,"
/// without depending on a real model's actual output.
#[derive(Clone)]
pub struct ConstantEmbedder(pub Vec<f32>);

#[async_trait]
impl Embedder for ConstantEmbedder {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if text.trim().is_empty() {
            return None;
        }
        Some(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_first_embedding_from_an_openai_shaped_response() {
        let json = serde_json::json!({
            "data": [{"embedding": [0.1, 0.2, 0.3]}]
        });
        assert_eq!(extract_embedding(&json), Some(vec![0.1, 0.2, 0.3]));
    }

    #[test]
    fn a_missing_data_array_produces_none_not_a_panic() {
        assert_eq!(extract_embedding(&serde_json::json!({})), None);
        assert_eq!(extract_embedding(&serde_json::json!({"data": []})), None);
    }

    #[test]
    fn a_non_numeric_entry_is_dropped_and_the_result_is_rejected() {
        // Silent truncation would corrupt every dimension after the bad entry rather than
        // failing loudly — better to discard the whole vector than store a wrong one.
        let json = serde_json::json!({"data": [{"embedding": [0.1, "not a number", 0.3]}]});
        assert_eq!(extract_embedding(&json), None);
    }

    #[tokio::test]
    async fn the_deterministic_embedder_is_actually_deterministic() {
        let embedder = DeterministicEmbedder;
        let a = embedder.embed("how do I reset my password").await.unwrap();
        let b = embedder.embed("how do I reset my password").await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn the_deterministic_embedder_refuses_empty_text() {
        assert!(DeterministicEmbedder.embed("").await.is_none());
        assert!(DeterministicEmbedder.embed("   ").await.is_none());
    }

    #[tokio::test]
    async fn the_null_embedder_never_produces_anything() {
        assert!(NullEmbedder.embed("anything").await.is_none());
    }
}

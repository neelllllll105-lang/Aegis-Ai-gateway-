//! Local ONNX embedding generation — the fast path for the semantic cache.
//!
//! # Why this file exists, and why it's feature-gated
//!
//! `cache::embed::ProviderEmbedder` calls a remote provider's `/embeddings` endpoint —
//! correct, but 50-300ms of network round trip, nowhere near the gateway's own sub-2ms
//! overhead budget (`routes/openai_compat.rs`'s module doc). Running a small embedding
//! model locally, in-process, closes that gap to single-digit milliseconds.
//!
//! This is behind the `local-embeddings` Cargo feature, off by default, because unlike
//! everything else in this crate it depends on a native ONNX Runtime binary and a real
//! model file — neither of which this crate can safely fetch on its own. See
//! `docs/adr/0009-local-onnx-embeddings.md` for the full reasoning and exactly how to
//! provision both. **Nothing in this module has been compiled or run in this session** —
//! there is no model file or ONNX Runtime binary available in this environment, and both
//! must come from the operator, deliberately, not from an automated download this crate
//! triggers on its own. Treat every claim below as a documented design, not a verified one,
//! until it has actually run against a real model.
//!
//! # The model this expects
//!
//! **`BAAI/bge-small-en-v1.5`** (33M params, 384-dim, MIT license) — chosen over
//! `sentence-transformers/all-MiniLM-L6-v2` (22M params) after comparing both against
//! current benchmarks and NVIDIA's open embedding line (Nemotron 3 Embed): NVIDIA's
//! smallest variant is 1.14B parameters and quantized for Blackwell-class GPUs, roughly
//! 50x too large and the wrong architecture family for a sub-5ms CPU cache lookup: two
//! separate models tuned for two different jobs. Within the CPU-sized tier, BGE-small
//! consistently benchmarks above MiniLM at nearly the same latency and size — worth taking
//! for a mechanism whose real danger is a false-positive cache hit, not raw speed.
//!
//! Get it ONNX-exported via `optimum-cli export onnx --model BAAI/bge-small-en-v1.5`, or a
//! pre-exported copy (verify before trusting a third party, same as any model weights —
//! see `docs/runbooks/local-embeddings-setup.md`). Expects a standard BERT-style
//! `input_ids`/`attention_mask`/`token_type_ids` input signature and a `last_hidden_state`
//! output — the raw per-token hidden states, not a pre-pooled sentence embedding.
//!
//! **Pooling: mean, not CLS.** BGE supports both, and they produce *incompatible*
//! embedding spaces — mixing them silently corrupts every similarity comparison. This
//! module always mean-pools (matching this model's own documented default), so don't swap
//! in a CLS-pooled export without changing the code to match.
//!
//! **No query instruction prefix.** BGE's own documentation recommends a
//! `"Represent this sentence for searching relevant passages: "` prefix specifically for
//! *asymmetric* retrieval (a short query against long documents) — our semantic cache
//! compares one prompt against other prompts, a symmetric comparison the same model card
//! says needs no instruction. Prepending one here would just be a self-inflicted
//! fingerprint mismatch against anything cached before the code changed.
//!
//! **The 0.95 threshold is not automatically portable across models.** BGE's own docs
//! note its cosine-similarity distribution for genuinely unrelated text sits noticeably
//! above zero (roughly 0.6+, not the near-zero baseline some embedding spaces produce) —
//! `cache::semantic::DEFAULT_SIMILARITY_THRESHOLD` was picked as "strict enough that false
//! positives are rare," a claim tied to whichever model actually produced the vectors it
//! was reasoned about. Re-validate it against this model specifically (see the runbook's
//! false-positive-rate check) rather than assuming a threshold that made sense for one
//! embedding space still means the same thing in another.

use crate::cache::embed::Embedder;
use async_trait::async_trait;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// A local, in-process embedder backed by ONNX Runtime.
///
/// `ort::session::Session` is `Send` but not safely `Sync` for concurrent `run()` calls in
/// the way this crate's other shared state is (every other `AppState` field is cheap to
/// share across the whole request-handling thread pool) — wrapped in a `Mutex` rather than
/// asserted otherwise. This makes embedding generation serialize under concurrent load,
/// which is the honest tradeoff for a model this small: benchmark whether that serialization
/// is actually a bottleneck (see `benches/semantic_embedding.rs`) before reaching for a
/// session pool instead.
pub struct OnnxEmbedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    max_sequence_length: usize,
}

/// bge-small-en-v1.5's trained context — a BERT-base-style 512-token position embedding
/// limit. Longer inputs are truncated rather than rejected: a cache-lookup embedding
/// degrading gracefully on an unusually long prompt is better than the request failing
/// outright. A different model swapped in later should pass its own real limit to
/// [`OnnxEmbedder::load`] rather than inherit this one silently.
pub const BGE_SMALL_MAX_SEQUENCE_LENGTH: usize = 512;

/// Everything that can go wrong constructing an [`OnnxEmbedder`] — deliberately its own
/// type rather than `crate::error::AegisError`, since this only ever happens at startup
/// (a missing model file is an operator mistake, not a per-request failure mode) and the
/// caller decides whether that's fatal or a fallback to `ProviderEmbedder`.
#[derive(Debug, thiserror::Error)]
pub enum OnnxEmbedderError {
    #[error("loading the ONNX model failed: {0}")]
    Model(#[from] ort::Error),
    #[error("loading the tokenizer failed: {0}")]
    Tokenizer(String),
}

impl OnnxEmbedder {
    /// Load a model and tokenizer from disk.
    ///
    /// `intra_threads` should match the number of physical cores you're willing to give
    /// this one session — not the whole machine's core count, since the gateway's own
    /// async runtime and every other stage of the pipeline are competing for the same CPU.
    ///
    /// `max_sequence_length` must match the model actually being loaded — passed in
    /// explicitly rather than hardcoded, since silently reusing one model's trained
    /// context length for a different model is exactly the kind of quiet mismatch that
    /// degrades embedding quality without ever producing an error. Use
    /// [`BGE_SMALL_MAX_SEQUENCE_LENGTH`] for this module's target model.
    pub fn load(
        model_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
        intra_threads: usize,
        max_sequence_length: usize,
    ) -> Result<OnnxEmbedder, OnnxEmbedderError> {
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(intra_threads)?
            .commit_from_file(model_path)?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| OnnxEmbedderError::Tokenizer(e.to_string()))?;

        Ok(OnnxEmbedder {
            session: Mutex::new(session),
            tokenizer,
            max_sequence_length,
        })
    }

    /// Tokenize, run the model, mean-pool, and L2-normalize — the full pipeline from text
    /// to a comparable embedding vector.
    fn embed_sync(&self, text: &str) -> Option<Vec<f32>> {
        let encoding = self.tokenizer.encode(text, true).ok()?;

        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let mut mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        ids.truncate(self.max_sequence_length);
        mask.truncate(self.max_sequence_length);
        let seq_len = ids.len();
        if seq_len == 0 {
            return None;
        }
        // BERT-style models take a token-type-id input even for a single segment; a
        // single-sentence embedding request is entirely segment 0.
        let type_ids: Vec<i64> = vec![0; seq_len];

        let input_ids = Tensor::from_array(([1, seq_len], ids)).ok()?;
        let attention_mask = Tensor::from_array(([1, seq_len], mask.clone())).ok()?;
        let token_type_ids = Tensor::from_array(([1, seq_len], type_ids)).ok()?;

        let mut session = self.session.lock().ok()?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids,
                "attention_mask" => attention_mask,
                "token_type_ids" => token_type_ids,
            ])
            .ok()?;
        // `outputs` borrows from the session's internal allocator, so the lock has to
        // outlive it — held a little longer than the minimum, released when this
        // function returns rather than explicitly.

        let hidden = outputs.get("last_hidden_state")?;
        let (shape, data) = hidden.try_extract_tensor::<f32>().ok()?;
        // Expected shape: [1, seq_len, hidden_size].
        let hidden_size = *shape.last()? as usize;
        if shape.len() != 3 || shape[1] as usize != seq_len {
            tracing::warn!(
                ?shape,
                "unexpected ONNX output shape, skipping this embedding"
            );
            return None;
        }

        Some(mean_pool_and_normalize(data, seq_len, hidden_size, &mask))
    }
}

/// Mean-pool token embeddings into one sentence embedding, respecting the attention mask
/// (padding tokens must not dilute the average), then L2-normalize.
///
/// Split out as a pure function so the pooling math itself — the part most likely to have
/// an off-by-one or a sign error — is unit-testable with hand-built inputs, no model or
/// tokenizer required.
fn mean_pool_and_normalize(
    token_embeddings: &[f32],
    seq_len: usize,
    hidden_size: usize,
    attention_mask: &[i64],
) -> Vec<f32> {
    let mut summed = vec![0f32; hidden_size];
    let mut valid_tokens = 0f32;

    for position in 0..seq_len {
        if attention_mask.get(position).copied().unwrap_or(0) == 0 {
            continue;
        }
        valid_tokens += 1.0;
        let offset = position * hidden_size;
        for dim in 0..hidden_size {
            summed[dim] += token_embeddings[offset + dim];
        }
    }

    if valid_tokens == 0.0 {
        return vec![0.0; hidden_size];
    }
    for value in &mut summed {
        *value /= valid_tokens;
    }

    let norm: f32 = summed.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in &mut summed {
            *value /= norm;
        }
    }
    summed
}

#[async_trait]
impl Embedder for OnnxEmbedder {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if text.trim().is_empty() {
            return None;
        }
        // ONNX Runtime's `run()` is synchronous CPU work, not I/O — running it inline on
        // the async executor would block that worker thread for the whole inference.
        // `spawn_blocking` hands it to tokio's blocking pool instead, matching how any
        // other CPU-bound call embedded in an async pipeline should be scheduled.
        let text = text.to_string();
        // `OnnxEmbedder` is not `Clone`; the caller holds it behind an `Arc` in
        // `AppState`, and `spawn_blocking` needs a `'static` closure — a raw pointer
        // carried across the boundary would be unsound if the embedder were ever dropped
        // mid-call, which an `Arc` this is always held behind rules out in practice, but
        // making that safe *in the type system* rather than by convention is future work
        // if this graduates past the experimental feature flag. For now: called only via
        // `Arc<dyn Embedder>`, which never drops while a request is in flight.
        tokio::task::block_in_place(|| self.embed_sync(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pooling_ignores_padded_positions() {
        // 2 tokens, hidden_size 2. Position 1 is padding (mask = 0) with a huge value that
        // must not leak into the average.
        let embeddings = vec![
            1.0, 1.0, // position 0, real token
            999.0, 999.0, // position 1, padding — must be excluded
        ];
        let mask = vec![1, 0];
        let pooled = mean_pool_and_normalize(&embeddings, 2, 2, &mask);
        // After normalization: [1.0, 1.0] normalized is [1/sqrt(2), 1/sqrt(2)].
        assert!((pooled[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!((pooled[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn the_result_is_always_unit_length() {
        let embeddings = vec![3.0, 4.0, 0.0, 0.0];
        let mask = vec![1, 1];
        let pooled = mean_pool_and_normalize(&embeddings, 2, 2, &mask);
        let norm: f32 = pooled.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "expected unit length, got {norm}"
        );
    }

    #[test]
    fn an_all_padding_mask_returns_zeros_not_a_panic() {
        let embeddings = vec![5.0, 5.0];
        let mask = vec![0];
        let pooled = mean_pool_and_normalize(&embeddings, 1, 2, &mask);
        assert_eq!(pooled, vec![0.0, 0.0]);
    }
}

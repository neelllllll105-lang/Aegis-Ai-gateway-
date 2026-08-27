//! Latency microbenchmark for the local ONNX embedding path.
//!
//! **Feature-gated (`local-embeddings`) and needs a real model to run at all** — see
//! `docs/adr/0009-local-onnx-embeddings.md`. This has not been run in this session for
//! the same reason `cache::onnx_embed` itself hasn't: no ONNX Runtime binary or model
//! file is available in this environment. This file exists so the benchmark exists and
//! is correct to run the moment those two things are provided — not to claim a number
//! nobody has measured.
//!
//! ```bash
//! cargo bench --bench semantic_embedding --features local-embeddings -- \
//!   --measurement-time 10
//! ```
//!
//! Reports p50/p95/p99-equivalent statistics (criterion's own HTML report includes the
//! full distribution, not just a mean) — a single "it took Nms" number is exactly the
//! kind of theoretical claim this benchmark exists to replace.

#![cfg(feature = "local-embeddings")]

use aegis_gateway::cache::embed::Embedder;
use aegis_gateway::cache::onnx_embed::OnnxEmbedder;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::env;

/// Realistic cache-lookup text, not a toy string — length and structure matter for a
/// tokenizer-bound benchmark. Mirrors `cache::semantic::embedding_text`'s actual shape
/// (a short system prompt plus one user question), not a single word.
fn workload_texts() -> Vec<(&'static str, String)> {
    vec![
        ("short", "How do I reset my password?".to_string()),
        (
            "typical",
            "You are a helpful support assistant for a SaaS billing product.\n\n\
             A customer is asking why their invoice this month is higher than last \
             month's, and whether the usage-based charges are calculated correctly."
                .to_string(),
        ),
        (
            "long",
            "You are a senior support engineer.\n\n".to_string()
                + &"The customer reports intermittent 502 errors on the checkout endpoint \
                    under load, and wants a root-cause analysis plus a mitigation plan. "
                    .repeat(8),
        ),
    ]
}

fn embed_latency(c: &mut Criterion) {
    // Real paths, provided by the operator running this benchmark — never defaulted to
    // something bundled in the repo, for the same reason `cache::onnx_embed`'s own docs
    // give: this crate does not ship a model or a native binary.
    let model_path = env::var("AEGIS_BENCH_ONNX_MODEL").expect(
        "set AEGIS_BENCH_ONNX_MODEL to a bge-small-en-v1.5.onnx path to run this benchmark",
    );
    let tokenizer_path = env::var("AEGIS_BENCH_TOKENIZER")
        .expect("set AEGIS_BENCH_TOKENIZER to a tokenizer.json path to run this benchmark");
    let threads: usize = env::var("AEGIS_BENCH_INTRA_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);

    let embedder = OnnxEmbedder::load(
        &model_path,
        &tokenizer_path,
        threads,
        aegis_gateway::cache::onnx_embed::BGE_SMALL_MAX_SEQUENCE_LENGTH,
    )
    .expect("model and tokenizer must load for this benchmark to mean anything");

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime for the async embed() call");

    let mut group = c.benchmark_group("onnx_embed");
    for (label, text) in workload_texts() {
        group.bench_with_input(BenchmarkId::from_parameter(label), &text, |b, text| {
            b.iter(|| runtime.block_on(embedder.embed(black_box(text))));
        });
    }
    group.finish();
}

criterion_group!(benches, embed_latency);
criterion_main!(benches);

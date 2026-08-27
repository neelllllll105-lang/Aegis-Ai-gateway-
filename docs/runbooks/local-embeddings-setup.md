# Runbook: provisioning local ONNX embeddings

For `cache::onnx_embed::OnnxEmbedder` (`docs/adr/0009-local-onnx-embeddings.md`). This
feature is off by default (`local-embeddings` Cargo feature) precisely because the three
things below cannot be fetched by the crate itself — they're yours to provision,
deliberately, once.

---

## 1. The ONNX Runtime binary

`ort` needs the actual ONNX Runtime shared library at **link and run time**, not just
compile time. Two ways to get it, pick based on where this runs:

**Self-hosted / a machine you control the exact hardware for:**
```bash
cargo build --release --features "local-embeddings ort/download-binaries"
```
`download-binaries` is `ort`'s own feature, not this project's — adding it explicitly, on
the command line, is the one-line informed choice this project's Cargo.toml deliberately
doesn't make for you. It fetches a prebuilt binary from ONNX Runtime's GitHub releases the
first time you build.

**Managed / reproducible builds (recommended for the SaaS deployment path):**
Install a specific, audited ONNX Runtime release yourself and point `ort` at it via
`load-dynamic`:
```bash
export ORT_DYLIB_PATH=/opt/onnxruntime/lib/libonnxruntime.so
cargo build --release --features "local-embeddings ort/load-dynamic"
```
One known binary, pinned by you, not "whatever GitHub served on build day."

## 2. The model

`sentence-transformers/all-MiniLM-L6-v2`, exported to ONNX. The most common path:

```bash
pip install optimum[exporters]
optimum-cli export onnx --model sentence-transformers/all-MiniLM-L6-v2 ./minilm-onnx/
```

This produces `./minilm-onnx/model.onnx` and `./minilm-onnx/tokenizer.json` (and a few
other files `OnnxEmbedder` doesn't need). **Verify the export before trusting it**: this
step downloads model weights from HuggingFace Hub, which is the "untrusted source" this
project's own rules are careful about — check the file hash against a known-good export,
or export it yourself from the model card's stated weights rather than trusting a
third-party mirror.

Confirm the model has the expected shape before wiring it in:
```bash
python -c "
import onnx
m = onnx.load('./minilm-onnx/model.onnx')
print('inputs:', [i.name for i in m.graph.input])
print('outputs:', [o.name for o in m.graph.output])
"
# Expect inputs: input_ids, attention_mask, token_type_ids
# Expect an output named last_hidden_state
```
If the export instead produces a pre-pooled `sentence_embedding` output, `OnnxEmbedder`'s
own mean-pooling code is redundant but harmless *if* `last_hidden_state` is also present —
if only the pooled output exists, this module needs a small change to read that tensor
directly instead. Check before assuming either shape.

## 3. Verify, in this order, before trusting anything downstream

```bash
export AEGIS_BENCH_ONNX_MODEL=/path/to/minilm-onnx/model.onnx
export AEGIS_BENCH_TOKENIZER=/path/to/minilm-onnx/tokenizer.json

# 1. The pure-math unit tests — these have never run anywhere, including this session
#    (see the ADR). This is the first real signal the pooling/normalization logic is
#    actually correct, not just reviewed.
cargo test --features local-embeddings --lib cache::onnx_embed::

# 2. The latency benchmark — p50/p95/p99, not a single number.
cargo bench --bench semantic_embedding --features local-embeddings

# 3. The one that actually matters: false-positive rate at the 0.95 threshold.
# Not yet scripted — build a small harness that embeds N known-distinct prompt pairs
# with this exact model and confirms none land above 0.95 similarity, using
# cache::semantic::cosine_similarity (already unit-tested in isolation). Do this before
# wiring OnnxEmbedder into AppState anywhere real — see the ADR's "When to revisit."
```

## 4. Wiring it in (not yet done anywhere)

Once the above is green, `main.rs` would construct an `OnnxEmbedder` instead of (or as a
fallback chain in front of) `ProviderEmbedder` for `AppState::embedder`. Not attempted in
this session — deliberately sequenced after verification, not before it.

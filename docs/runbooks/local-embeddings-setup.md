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

**`BAAI/bge-small-en-v1.5`** — chosen over the originally-proposed `all-MiniLM-L6-v2` and
over NVIDIA's open embedding line (Nemotron 3 Embed); see the ADR's Context section for
the full comparison. 33M params, 384-dim, MIT license.

```bash
pip install optimum[exporters]
optimum-cli export onnx --model BAAI/bge-small-en-v1.5 ./bge-small-onnx/
```

This produces `./bge-small-onnx/model.onnx` and `./bge-small-onnx/tokenizer.json` (and a
few other files `OnnxEmbedder` doesn't need). A pre-exported copy also exists at
[`onnx-community/bge-small-en-v1.5-ONNX`](https://huggingface.co/onnx-community/bge-small-en-v1.5-ONNX)
if exporting yourself isn't practical. **Verify the export before trusting it either
way**: this step downloads model weights from HuggingFace Hub, which is the "untrusted
source" this project's own rules are careful about — check the file hash against a
known-good export, or export it yourself from the model card's stated weights rather than
trusting a third-party mirror.

Confirm the model has the expected shape before wiring it in:
```bash
python -c "
import onnx
m = onnx.load('./bge-small-onnx/model.onnx')
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

**Two BGE-specific details already handled in code, listed here so nobody "fixes" them
by accident:** `OnnxEmbedder` always mean-pools (not CLS-pools — the two produce
incompatible embedding spaces, and BGE's own default is mean); and it never prepends a
query-instruction prefix (BGE's own docs say that's for asymmetric retrieval, not the
symmetric prompt-to-prompt comparison a cache lookup actually is). See
`cache::onnx_embed`'s own module doc for the full reasoning on both.

## 3. Verify, in this order, before trusting anything downstream

```bash
export AEGIS_BENCH_ONNX_MODEL=/path/to/bge-small-onnx/model.onnx
export AEGIS_BENCH_TOKENIZER=/path/to/bge-small-onnx/tokenizer.json

# 1. The pure-math unit tests — these have never run anywhere, including this session
#    (see the ADR). This is the first real signal the pooling/normalization logic is
#    actually correct, not just reviewed.
cargo test --features local-embeddings --lib cache::onnx_embed::

# 2. The latency benchmark — p50/p95/p99, not a single number.
cargo bench --bench semantic_embedding --features local-embeddings

# 3. The one that actually matters: false-positive rate at the 0.95 threshold.
# Not yet scripted -- build a small harness that embeds N known-distinct prompt pairs
# with this exact model and confirms none land above 0.95 similarity, using
# cache::semantic::cosine_similarity (already unit-tested in isolation). Extra reason
# this specific check matters for BGE: its own docs note unrelated-text similarity in
# its embedding space sits noticeably above zero (roughly 0.6+), not near it -- a
# threshold reasoned about for one embedding space doesn't automatically carry the same
# meaning in another. Do this before wiring OnnxEmbedder into AppState anywhere real --
# see the ADR's "When to revisit."
```

## 4. Wiring it in (not yet done anywhere)

Once the above is green, `main.rs` would construct an `OnnxEmbedder` instead of (or as a
fallback chain in front of) `ProviderEmbedder` for `AppState::embedder`. Not attempted in
this session — deliberately sequenced after verification, not before it.

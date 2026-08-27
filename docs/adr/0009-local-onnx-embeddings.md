# ADR-009: Local ONNX embeddings for the semantic cache, feature-gated and off by default

- **Status:** Proposed — code written and type-checked, **not yet compiled with a real
  model or run**, see Consequences. Promote to Accepted once that verification happens.
- **Date:** 2026-08-27
- **Deviates from `MASTER_BUILD.md`:** Yes — introduces two dependencies (`ort`,
  `tokenizers`) outside Part 2's stack list, and a native ONNX Runtime binary + a model
  file, neither of which is source code this repository can vendor. CLAUDE.md rule 4
  requires this record.

## Context

`cache::embed::ProviderEmbedder` (session 7) calls a remote provider's `/embeddings`
endpoint to generate the vector the semantic cache searches with. That is correct but
slow — 50-300ms of network round trip — nowhere near this gateway's own stated overhead
budget (`routes/openai_compat.rs`'s module doc: auth + rate limit + budget + parse + cache
+ routing + emit ≈ 1.45ms worst case). The founder proposed running the embedding model
locally, in-process, using ONNX Runtime — the standard way to get a small transformer
model (`sentence-transformers/all-MiniLM-L6-v2`) running fast on commodity CPU hardware
without a Python service in front of it.

That request came with a real environment constraint worth stating plainly: `ort`
(ONNX Runtime's Rust bindings) needs a native shared library to actually run inference,
and running the model at all needs its weights — a ~90MB `.onnx` file — plus a tokenizer
vocabulary. **None of those three things can be produced by writing Rust source code.**
This project's own safety rules (and good sense, independent of them) prohibit silently
downloading and executing a native binary from the internet as a side effect of someone
running `cargo build`. So this ADR's decision is as much about *how the dependency is
introduced safely* as it is about the embedding pipeline itself.

**Which model, revisited before shipping.** The founder also asked directly whether an
NVIDIA open embedding model made sense here, and to compare it against the alternatives —
worth researching properly rather than assuming. NVIDIA's current open, commercially-usable
line (Nemotron 3 Embed, released July 2026, OpenMDW-1.1 license) tops the RTEB leaderboard,
but even its smallest variant is 1.14B parameters, decoder-based, and quantized (NVFP4) for
Blackwell-class GPUs — roughly 50x larger than the CPU-sized tier this feature needs, and
architecturally built for a different job: embed a corpus once, offline, as well as
possible, not embed one live request in single-digit milliseconds on a CPU core. Ruled out
on fit, not license. Within the actual CPU tier, `BAAI/bge-small-en-v1.5` (33M params,
384-dim, MIT) was chosen over the originally-proposed `sentence-transformers/all-MiniLM-L6-v2`
(22M params) — multiple current sources rank BGE-small above MiniLM on retrieval quality at
nearly identical latency and size, and for a mechanism whose real risk is a false-positive
cache hit, that tradeoff is worth taking.

## Decision

**`cache::onnx_embed::OnnxEmbedder`**, a second implementation of the `Embedder` trait
(the same interface `ProviderEmbedder` implements — this is exactly the seam that trait
was built for). Behind a new Cargo feature, `local-embeddings`, **off by default**:

```toml
[dependencies]
ort = { version = "=2.0.0-rc.10", default-features = false, features = ["std", "ndarray"], optional = true }
tokenizers = { version = "0.23", default-features = false, features = ["fancy-regex"], optional = true }

[features]
local-embeddings = ["dep:ort", "dep:ndarray", "dep:tokenizers"]
```

`default-features = false` on `ort` deliberately excludes its own `download-binaries`
feature. With it, enabling `local-embeddings` and running `cargo build` would silently
fetch a prebuilt native library from GitHub releases the first time anyone builds with
that flag — exactly the automated-untrusted-binary-download this decision is trying to
avoid making silently. An operator who wants that convenience can add `download-binaries`
to `ort`'s feature list themselves, as an explicit, informed, one-line choice — or point
`ort` at a system-installed ONNX Runtime via its `load-dynamic` feature instead, which is
the safer default for a production deployment (one known, audited binary, not "whatever
GitHub releases served today").

**Pipeline**: tokenize (WordPiece, via `tokenizers`) → ONNX session `run()` →
`last_hidden_state` → mean-pool over real (non-padding) tokens using the attention mask →
L2-normalize. This is the standard `sentence-transformers` recipe for this exact model
family, not a shortcut — a naive mean over *all* positions (including padding) or a raw,
un-normalized vector would both quietly corrupt every cosine-similarity comparison
downstream, which is a correctness bug in a component whose entire job is a strict 0.95
similarity threshold.

**Threading**: `Session` is wrapped in a `Mutex` inside `OnnxEmbedder`, and `embed()` runs
it via `tokio::task::block_in_place` — inference is synchronous CPU work, not I/O, and
running it inline on an async worker thread would stall that thread for the whole
inference. The `Mutex` means concurrent embedding calls serialize; see Consequences for
when that stops being acceptable.

## Consequences

**This has not been verified end to end, and that gap is real, not a formality — and it
goes one step further than "untested," confirmed by actually trying.** Every other piece
of Rust written in this project this session compiled *and linked and ran*, in this
environment, before being called done. This one only got the first of those three:

- `cargo check --features local-embeddings` **succeeds** — the code type-checks against
  the real `ort` v2.0.0-rc.10 and `tokenizers` v0.23 APIs, not a guess at their surface.
  One real bug this caught: the first draft dropped the `ort` session's `MutexGuard`
  before reading its output tensor, which doesn't compile because the output borrows from
  the session's allocator — fixed by holding the guard for the whole function instead.
- `cargo test --features local-embeddings` (or `cargo build`) **fails to link**, confirmed
  directly, not assumed: `ort-sys`'s build script emits a deliberately self-explanatory
  placeholder linker input — `add_ort_library_path_or_enable_feature_download-binaries_see_ort_docs.lib`
  — when no ONNX Runtime binary is configured, and the linker fails loudly on it rather
  than doing anything silent. This means **even the pure mean-pooling/normalization unit
  tests, which never call into `ort` at runtime, cannot run in this environment either** —
  Rust links whole test binaries, not individual functions, so a crate that merely
  *depends* on `ort` needs the native library resolvable at link time regardless of which
  specific test is selected. That's a more precise (and more honest) claim than "the math
  is tested, only the model loading isn't" — right now, with this feature flag on, nothing
  in this crate runs at all in this environment, full stop.
- What this **does** prove, and what it doesn't: the confirmed link failure is evidence the
  `local-embeddings` feature genuinely requires an explicit, deliberate binary-provisioning
  step to do anything — which is exactly the safety property this ADR's design was going
  for — but it also means the mean-pooling math's correctness (unit-tested with hand-built
  vectors, reviewed by hand) has not been exercised by an actual test run, only read.
- Confirmed unaffected: `cargo build`/`cargo test` **with no feature flags** — the default
  path everyone actually uses — re-ran clean after all of the above, 774 lib / 814
  full-suite passing, identical to before this ADR existed. The isolation the feature flag
  is supposed to provide is real, not just claimed.
- Re-confirmed after switching the target model from MiniLM to BGE-small-en-v1.5 (which
  also made `max_sequence_length` a constructor parameter instead of a hardcoded constant,
  since silently reusing one model's trained context length for a different model is
  exactly the kind of quiet mismatch worth designing out): `cargo check`/`cargo clippy
  --lib`/`cargo clippy --benches`, all with `--features local-embeddings`, still pass
  clean, and the default build still shows the same 774/814.

What remains genuinely open, and needs a real ONNX Runtime binary plus a real
`bge-small-en-v1.5.onnx` and tokenizer file to close: whether the pure-math unit tests
actually pass when they can run, and — the part that actually matters — whether real
embeddings from this model behave the way `cache/semantic.rs`'s 0.95 threshold assumes.

**Quantization was deliberately not attempted here — and the same caution applies to the
model choice itself, not just quantization.** The founder's original brief proposed INT8
quantization for extra speed; this ADR ships FP32 first, on purpose. A quantized model
shifts the embedding space, which can change which pairs of prompts land above or below
the 0.95 similarity threshold. But that's really one instance of a broader fact worth
stating plainly: **any change to which model produces the vectors — quantizing it,
swapping it for a different architecture, even switching pooling strategy — can move that
threshold's real meaning**, because 0.95 was reasoned about in the context of whatever
model actually produced the scores it was calibrated against. BGE-small's own
documentation makes this concrete: unrelated-text similarity in its embedding space sits
noticeably above zero, not near it. For a mechanism whose entire safety argument is "false
positives must be rare," any of these changes needs a real false-positive-rate benchmark
before it ships, not an assumption that a threshold tuned in one context still means the
same thing in another. FP32 BGE-small first establishes the correctness baseline;
quantization is a follow-up decision, not bundled into this one.

**The `Mutex`-serialized session is a real, accepted limitation for now.** Under load,
every concurrent semantic-cache-eligible request queues behind the same lock for its
inference. Whether that's a problem depends entirely on real numbers this environment
can't produce — `benches/semantic_embedding.rs` (added alongside this ADR) measures single-
call latency, which does not tell you throughput under concurrency. If benchmarking against
a real model shows the mutex is the bottleneck, the fix is a small pool of `Session`
instances (`ort` sessions are independently constructible from the same model file) rather
than sharing one — not attempted here because it's premature without a number proving it's
needed.

**Deployment story differs by path.** Self-hosted customers build and run on their own
known hardware — `download-binaries` or a vendored system ONNX Runtime is their call, made
once, in an environment they control. The managed SaaS path (Hetzner, session-6-era
runbook) would need the model file and ONNX Runtime baked into the Docker image at build
time — a CI change, not an application-code change, and not attempted in this session.

## When to revisit

Once a real model file and `ort` binary are available anywhere (a developer's machine, a
CI runner with `download-binaries` explicitly enabled for a verification job, or the
managed SaaS build pipeline): run `cargo test --features local-embeddings`, run
`benches/semantic_embedding.rs`, and — the one that actually matters — check the
false-positive rate at the 0.95 threshold against real embeddings from this exact model
before promoting this ADR's status to Accepted. If the `Mutex`-serialized session shows up
as a real bottleneck under `tests/overhead_under_load.rs`-style concurrency, revisit the
session-pool design then, with a number in hand rather than guessing ahead of it.

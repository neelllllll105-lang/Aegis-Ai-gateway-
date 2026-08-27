# ADR-010: Qdrant over gRPC, preferred when configured, REST kept as a fallback

- **Status:** Accepted — compiled, linked, and the full test suite passes with this
  dependency active by default (unlike ADR-009's `ort`, this needed no native binary and
  no model file, so this one got the verification that one couldn't). **Not yet exercised
  against a live Qdrant server** — no Docker in this environment — see Consequences.
- **Date:** 2026-08-27
- **Deviates from `MASTER_BUILD.md`:** Yes — introduces `qdrant-client` (and its `tonic`/
  `prost` gRPC stack) outside Part 2's stack list. CLAUDE.md rule 4 requires this record.

## Context

`cache::semantic::QdrantVectorStore` talks Qdrant's REST API over HTTP/1.1 via `reqwest` —
correct, tenant-isolated (one collection per organisation), and already tested through the
`VectorStore` trait's `MemoryVectorStore` coverage. When the semantic-cache latency work
first came up (session 7, the ONNX embedding discussion), moving Qdrant to gRPC was
deliberately scoped out — the founder chose "embedding fix only, for now," correctly,
since the remote embedding call at 50-300ms dwarfed Qdrant's REST round trip. That
priority call was right at the time.

It stops being obviously right once `cache::onnx_embed::OnnxEmbedder` (ADR-009) is
actually in place: with the embedding step down to single-digit milliseconds, Qdrant's own
round trip — a JSON-encoded ~384-float vector, several kilobytes of digit text, parsed on
both ends — becomes a proportionally larger share of what's left in the semantic-cache
path. The founder asked directly whether to pick this back up now; this ADR is that.

## Decision

**`cache::qdrant_grpc::QdrantGrpcVectorStore`**, a second implementation of the
`VectorStore` trait (the same interface `QdrantVectorStore` already implements — the trait
existed specifically so a second backend is a new `impl`, not a rewrite). Unlike `ort`,
`qdrant-client` is pure Rust with no native binary dependency, so it needed none of ADR-009's
feature-gating — it's a normal, always-compiled dependency:

```toml
qdrant-client = { version = "1", default-features = false, features = ["serde", "uuid"] }
```

`AppState::semantic_store` now picks in this order: gRPC (`QDRANT_GRPC_URL`) if
configured, REST (`QDRANT_URL`) if not, in-process (`MemoryVectorStore`) if neither is set
— an operator who hasn't opened Qdrant's gRPC port (default 6334, distinct from REST's
6333) keeps running on REST with zero code change. `QDRANT_GRPC_URL` is a deliberately
**separate** setting from `QDRANT_URL` rather than derived by substituting the port —
Qdrant Cloud and some self-hosted setups front REST and gRPC on different hosts entirely,
and a derived-and-wrong URL fails in a way that's much harder to notice than an unset one
correctly falling back to REST.

**Payload shape**: rather than hand-mapping every field of `SemanticEntry` to Qdrant's
protobuf `Value` type, the gRPC store JSON-encodes the whole entry into a single payload
string field — the same trick `cache::durable`'s encrypted blob already uses. Keeps the
two `VectorStore` implementations trivially payload-compatible and avoids a much larger
surface of hand-written protobuf conversion code that would need its own correctness
review.

The REST implementation is **not deleted**. It still works, it's still what a deployment
without gRPC access runs on, and there was no reason to remove tested code that a real
fallback path depends on.

## Consequences

**Fully verified at the type/link/test level — a stronger claim than ADR-009 could make,
and worth naming why.** `qdrant-client` has no native binary to provision, so
`cargo check`, `cargo clippy --all-targets -- -D warnings`, and the full `cargo test` all
ran for real here: every API call in `qdrant_grpc.rs`
(`Qdrant::from_url().build()`, `SearchPointsBuilder`, `UpsertPointsBuilder`,
`CreateCollectionBuilder`, `CountPointsBuilder`, `PointStruct::new`, `Payload::try_from`)
matched the real `qdrant-client` v1.19.0 API on the first attempt, and the default build's
774 lib / 814 full-suite tests all pass with this dependency compiled in. That is real
evidence the code compiles and doesn't disturb anything else — it is **not** evidence it
works against an actual Qdrant server, which this environment has no way to run (same
Docker gap noted throughout `MEMORY.md`). `QdrantGrpcVectorStore` has the same testing
posture the REST-based `QdrantVectorStore` has always had: zero direct unit tests (neither
is meaningfully testable without a live backend), covered indirectly through the
`VectorStore` trait's behavioural contract and `MemoryVectorStore`'s tests in
`cache/semantic.rs`. This is parity with the existing implementation, not a regression
from it.

**A real, caught-before-shipping `cargo audit` finding.** Enabling `local-embeddings`
(ADR-009) pulls `tokenizers`, which depends on the now-unmaintained `paste` crate
(RUSTSEC-2024-0436) — a lockfile artifact of the same shape session 6 found and closed for
`sqlx-mysql`/`rsa`: `paste` resolves into `Cargo.lock` but has zero edges into the actual
build graph with `local-embeddings` off (confirmed: `cargo tree -i paste` with default
features finds nothing; `--all-features` shows the single path
`paste -> tokenizers -> aegis-gateway`). Would have broken CI's `cargo audit --deny
warnings` gate the moment this branch merged. Closed with a scoped, evidenced ignore in
`.cargo/audit.toml`, matching the existing entry's format exactly. Re-ran
`cargo audit --deny warnings` from the repository root (matching CI's working directory)
after the fix: clean.

**Not covered by this ADR.** TLS/auth for a Qdrant Cloud gRPC endpoint (API-key headers,
TLS verification) — `Qdrant::from_url` handles the common self-hosted, no-auth case; a
managed Qdrant Cloud deployment would need the client builder's auth/TLS options wired
through config, not attempted here since this environment has no Qdrant instance, managed
or otherwise, to validate against.

## When to revisit

Once a real Qdrant instance is reachable (self-hosted via `infra/docker-compose.yml`, once
Docker works on this machine, or any other real deployment): confirm the gRPC path
actually connects, run a real search/upsert/count/drop cycle, and — the part that would
actually prove the latency claim this ADR is implicitly making — measure gRPC vs REST
round-trip time for the same search, not assume protobuf-over-HTTP/2 is faster without a
number. If Qdrant Cloud becomes a target deployment, revisit the auth/TLS gap noted above
before pointing `QDRANT_GRPC_URL` at it.

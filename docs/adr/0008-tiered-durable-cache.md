# ADR-008: A durable, encrypted cache tier between Redis and "gone after 24 hours"

- **Status:** Accepted
- **Date:** 2026-08-27
- **Deviates from `MASTER_BUILD.md`:** Yes — Part 5 specs exactly two cache tiers, exact
  (Redis) and semantic (Qdrant, `>= 0.95` similarity). This adds a third. CLAUDE.md rule 4
  requires this record. It also completes the semantic tier itself, which was built
  (`cache/semantic.rs`) but never wired into the live pipeline — see the session 3/5 audits
  in `MEMORY.md`.

## Context

The founder asked, directly: caching content to cut cost, or not storing content at all
for security — is there a way to get both? The honest answer was already implicit in the
existing design but never built: the hot Redis tier (`cache/exact.rs`) is intentionally
short (24h default TTL) because most prompts are asked once, and there is no reason to
keep a one-off prompt's plaintext around. But a **genuinely repeated** query — the same
onboarding question, the same boilerplate request — asked again after that window is a
full provider call for something Aegis has already answered. That gap was real and,
before this ADR, unaddressed.

Separately, the semantic cache (`cache/semantic.rs`) — embed the request, match within
0.95 cosine similarity, catch a *reworded* repeat — was fully implemented, fully tested in
isolation, and never called from the live pipeline. Two audits (session 3, session 5) both
flagged this as the one "built but not wired" item that was a genuine product decision
(it adds an embedding-API call and its latency to every cache-miss request) rather than an
oversight to just fix. The founder's explicit ask this session — "we want it to be as
smart as possible" — is that product decision, made.

## Decision

**Three tiers, in lookup order, each only reached if the one before it missed:**

1. **Hot (Redis, unchanged).** Every cache-eligible response, plaintext, 24h default TTL.
2. **Durable (new — Postgres, `cache_entries` table, migration 0004).** Only a fingerprint
   that has *already proven it repeats* — a genuine hot-tier hit — gets promoted here,
   encrypted with the same per-tenant HKDF key (`crypto::derive_tenant_key`) BYOK
   credentials already use. Expiry is sliding, 30 days by default
   (`AEGIS_DURABLE_CACHE_TTL_DAYS`), refreshed on every further hit, so a query that keeps
   repeating keeps its place and one that stops repeating ages out with no separate logic.
   A hit here re-populates the hot tier so the *next* repeat is instant again.
3. **Semantic (Qdrant, now wired).** Only reached once both exact tiers miss. The
   embedding is generated once per request and reused for both the lookup and, on a true
   miss, the write — never two embedding calls for one request. A semantic hit also writes
   the *current* wording into the hot tier under its own exact fingerprint, so a repeat of
   this specific phrasing skips the embedding call entirely next time.

**Every tier honors `zero_retention` identically** — an org that opted out of storage
never has anything written to any of the three tiers, full stop, regardless of how many
times a question repeats.

**Smart caching (durable promotion + semantic) is a Pro/Enterprise feature**, gated on
`auth.plan != "free"`. This matches `MASTER_BUILD.md`'s own plan table, which already
lists semantic caching under Pro. Free-tier traffic runs on Aegis's own pooled provider
keys with no revenue case for the extra storage and embedding cost.

**The embedding call always rides on Aegis's own pooled credential**
(`providers::pool::SharedKeyPool`), never a customer's BYOK key — see `cache/embed.rs`'s
own module documentation for the full reasoning (a customer whose only provider doesn't do
embeddings still gets semantic caching; no customer sees a surprise line item on their own
provider bill).

## Consequences

**Cost.** One new table, one new scheduled purge job (reusing the existing 04:00 UTC
session-purge window in `workers::scheduler`), and — the real new operating cost — an
embedding-API call on every semantic-cache-eligible miss for Pro/Enterprise orgs. Small in
absolute terms (a cheap embedding model, picked automatically via
`PricingTable::cheapest_embedding_model`) but real, and unlike every other cost in this
system, it is not billed to the customer — it is bundled into the plan price the way Redis
and Postgres compute already are.

**A new crypto surface, but not a new crypto primitive.** `cache_entries.encrypted_response`
uses the exact same AES-256-GCM-via-per-tenant-HKDF-key shape `provider_credentials`
already uses. No new key management story to explain to a security reviewer.

**Testability tradeoff, deliberately accepted.** `cache::embed::Embedder` is a trait
specifically so the semantic pipeline is testable without a live HTTP call — the real
`ProviderEmbedder` implementation itself has no direct test, the same honest gap
`cache::semantic::QdrantVectorStore` already had before this session, for the same reason
(no HTTP-mocking crate in this workspace, and adding one for a single call site was judged
not worth the new dependency). The JSON-parsing logic inside it (`extract_embedding`) is
split out and unit-tested directly.

**Not covered by this ADR.** Cross-replica consistency of the in-memory `MemoryVectorStore`
fallback (Qdrant-backed in production, single-instance-only otherwise — pre-existing,
inherited from `cache/semantic.rs`, not introduced here). Admin visibility into which
queries are actually hot in the durable tier — `hit_count` is tracked and available, but no
`/api/admin/*` endpoint surfaces it yet.

## When to revisit

If the embedding cost becomes material at scale, the natural next step is caching the
embedding *generation* itself for identical exact-fingerprint misses across an org (not
done here — the exact cache already prevents a second call for identical text; only
different-wording misses pay the embedding cost, which is the feature working as intended,
not a gap). If `hit_count` visibility becomes a support or sales need, surface it on
`GET /api/admin/pricing` or a new endpoint rather than adding another one-off query.

# Aegis — Project Context for AI Assistants

> Paste this whole document into a new LLM chat to give it working context on the Aegis
> project. Written 2026-09-02, against commit `50213fd`. It will go stale as the project
> moves — if the receiving LLM has repo access, point it at `MASTER_BUILD.md`, `MEMORY.md`,
> and `docs/PHASES.md` for the living, authoritative versions of everything summarized here.

---

## 1. What Aegis is

**Aegis** is a commercial, closed-source, multi-tenant SaaS platform: an AI cost-optimization
gateway that sits between applications and LLM providers. A customer changes one base URL —
their app, IDE, or SDK now points at Aegis instead of OpenAI/Anthropic/Google directly — and
Aegis:

1. **Routes** each request to the cheapest model that preserves quality
2. **Caches** responses (exact + semantic + durable) so repeats cost $0
3. **Compresses** context to cut token counts before the provider ever sees the prompt
4. **Governs** spend with budgets, rate limits, and per-key/org policies
5. **Meters** every request — tokens, cost, latency, savings — with financial-grade accuracy
6. **Proves** savings with per-request, auditable attribution (`baseline_cost` vs `actual_cost`)
7. **Fails over** automatically across providers during an outage

**Business model:** Free ($0, 10k req/mo on shared cheap models) → Pro ($29/mo + 20% of
verified savings) → Team ($299/mo + 15%) → Enterprise ($2k+/mo + 10%, SSO/SCIM/self-hosted).
Revenue is subscription *plus a share of savings we can prove*, computed as:

```
gross_savings = baseline_cost - actual_cost   (baseline = cost on the model the user asked for)
our_fee        = gross_savings * savings_share_rate   (only when gross_savings > 0)
customer_net   = gross_savings - our_fee
```

**Explicitly not:** a model provider, an observability-only tool, open source, or a wrapper
around OpenRouter (direct provider integrations; BYOK settles directly with the provider).

**Non-negotiable principles** (violating any of these is treated as a bug, not a tradeoff):
performance (<1ms P99 gateway overhead, no sync Postgres I/O in the hot path), meter
everything, security-first (keys encrypted, tenant isolation via `org_id` on every query, API
keys stored only as SHA-256 hashes), zero content retention by default, horizontal
scalability, cost discipline, closed source, and "conservative when in doubt" (route to the
requested model, keep the data, log the decision).

---

## 2. Architecture — three levels

```
Level 1  apps/web        Next.js 15 App Router. Dashboard + marketing + docs.
                          Talks to the gateway ONLY over its own public HTTP API —
                          no direct DB/Redis access, same as any external integration.
                          |
                          | HTTPS, REST + SSE, session cookie or API key
                          v
Level 2  apps/gateway     Rust + Axum + Tokio. THE PRODUCT. Single binary, stateless,
                          horizontally scalable. Runs a ~12-stage request pipeline
                          in-process per call (see §4). 6 background workers.
                          |
                          | Postgres: durable writes, org-scoped SQL
                          | Redis: counters, exact cache, sessions, atomic reservations
                          | Qdrant: semantic cache vector search
                          v
Level 3  Data layer       PostgreSQL 16 (system of record), Redis 7 (hot path, sub-ms),
                          Qdrant (semantic cache vectors)
```

The control plane has no special access path — it is a client of the gateway's own API, which
matters for security review (the dashboard cannot bypass any check a third-party integration
would hit).

---

## 3. Tech stack (do not introduce anything outside this without an ADR)

| Component | Choice |
|---|---|
| Gateway/core | Rust + Axum + Tokio |
| HTTP client | reqwest + hyper |
| DB access | SQLx (Postgres), prepared statements only, no ORM |
| Cache/rate limit | Redis 7, atomic Lua scripts |
| Vector cache | Qdrant |
| Frontend | Next.js 15 App Router + TypeScript + Tailwind |
| Database | PostgreSQL 16, JSONB for metadata, monthly partitioning on `usage_records` |
| Auth | Custom session tokens + hashed API keys (argon2id passwords, SHA-256 keys) |
| Payments | Stripe (built, never exercised against real Stripe) |
| Observability | OpenTelemetry + Prometheus + Grafana, self-hosted |
| Deploy target | Coolify on Hetzner, Cloudflare free tier (never actually deployed) |

Forbidden: MongoDB, Kafka, Kubernetes (until >10k RPS), microservices (until a module
independently needs to scale), any GPL/AGPL code.

---

## 4. The request pipeline (the actual product)

Every call to `/v1/chat/completions` or `/v1/messages` runs through this, in-process, budgeted
at <1ms of Aegis's own overhead (measured P99 1.27ms on an unoptimized debug build under
64-way concurrency — real numbers, not a target):

```
[1]  AUTH            SHA-256 key hash, LRU -> Redis -> Postgres
[2]  RATE LIMIT       Redis Lua sliding window, per-key + per-org, proven atomic
[3]  BUDGET           Atomic reserve-then-true-up across up to 5 scopes at once
                       (org/team/key/region — see §7), 402 on hard exceed
[4]  PARSE/NORMALIZE   Provider-specific JSON -> internal NormalizedRequest
[5]  CACHE LOOKUP      [5a] exact (Redis, fingerprint hashes org_id in) ->
                       [5b] semantic (Qdrant, org-namespaced, similarity >= 0.80,
                            Pro/Enterprise only) -> [5c] durable (Postgres,
                            encrypted, 30-day sliding TTL, promotes a proven-repeat
                            fingerprint past Redis's shorter TTL)
[6a] CLASSIFY          12-feature heuristic/linear scorer -> simple/medium/complex
                       (see §5)
[6b] ROUTE             Pick the model (see §5)
[6c] COMPRESS          Six lossless techniques on the prompt (see §6)
[7]  PROVIDER CALL     Retry x2, cross-provider fallback chain, circuit breaker,
                       streaming SSE with instant cache-hit replay
[8]  RESPONSE NORMALIZE  Provider's wire shape -> OpenAI- or Anthropic-compatible,
                       depending on which endpoint the caller used (independent of
                       which provider actually served it)
[9]  COST & SAVINGS     Micro-cents, baseline vs actual, computed once and frozen
                       onto the usage record
[10] USAGE EMIT         Redis XADD, non-blocking
[11] RESPOND            X-Aegis-* headers (cost, savings, cache, routing reason...)
[12] BACKGROUND WORKER   Batches Redis stream -> Postgres, idempotent
```

Money is **always** `i64` micro-cents (1 cent = 10,000 micro-cents). Floating point never
touches a stored monetary value — enforced by property tests, not just convention.

---

## 5. Smart routing — the core IP

**Classifier** (`engine/classifier.rs`): 12 lexical/structural features extracted in one
lowercase pass (length, turns, code presence, code intent, reasoning-verb density, multi-step
chaining, trivial-task vocabulary, short-question detection, tool use, system-prompt length,
task domain). Two scorers share the same feature vector — V1 hand-tuned heuristic weights, V2
a linear model with committed weights fit offline on a 100-example fixture set. Both hit ~98%
on that set (a small, somewhat circular benchmark — the fixture phrasing overlaps the keyword
lists, so real-world accuracy is less proven than the number suggests). **Tool use forces the
top band (0.75) regardless of anything else.**

Score bands: `<0.35` simple, `0.35–0.7` medium, `>0.7` complex.

**Router** (`engine/router.rs`): band maps to a tier ceiling —

```
complex -> requested model, UNCONDITIONAL, returns before any downgrade logic runs
medium  -> mid tier
simple  -> cheap tier
```

Budget pressure can tighten medium/simple further (steers toward cheaper models before a hard
402 refusal) but **cannot touch the complex branch** — it already returned. This is the single
most important guarantee in the codebase: quality is never traded for savings, enforced by
control flow, not by score thresholds.

Candidates are filtered by real capability (tools, vision, context window ≥ prompt +
`max_tokens`, chat-capable — embedding models are structurally excluded from ever winning a
chat request) and ranked by **effective price**, not sticker price:

```
effective_price = blended_price_per_mtok
                 x health_penalty      // <90% recent success rate -> up to 8x
                 x latency_penalty     // slower than the candidate median -> up to +20%, capped
```

A task-domain preference re-ranks within a 25% price band (code → DeepSeek/Groq/Mistral;
reasoning → Google/Vertex/Anthropic; language → Mistral/Groq/Google). A UCB1 bandit then
reorders — never widens — the permitted set based on recorded outcomes.

**Escape hatch:** `X-Aegis-Routing-Hint: passthrough` outranks everything, including org
policy. Always available, by design (Part 13 of the blueprint: "the passthrough escape hatch
must never be compromised").

**Known gap:** `RoutingHint::Cheap` is parsed from the header but the router never branches on
it — sending `X-Aegis-Routing-Hint: cheap` today is identical to `auto`. A proper 5-mode
ladder (`passthrough` / `quality` / `balanced` / `economy` / `auto`) with per-key/project/org
defaults is designed but not yet built.

**Failover:** graded circuit breaker per provider (closed → open after 5 consecutive failures
→ half-open probe after 30s), plus a real cross-provider fallback chain computed live from the
pricing table. The chain advances on 404/401/429/5xx; only a genuine 400 stops it (identical
error everywhere, so retrying elsewhere is pointless).

**Where this sits vs. the market** (researched September 2026): almost every production
gateway (LiteLLM, OpenRouter, Portkey, Requesty) routes on operational signals only and never
reads the prompt. Aegis is in the smaller "predictive classifier" family alongside RouteLLM
and Martian. The strongest published technique nothing here does yet is **cascade routing**
(call cheap, verify, escalate only on failure — RouteLLM reports 95% of GPT-4 quality at 14%
strong-model calls); the two genuinely novel ideas identified but not built are **prompt-cache
affinity** (keep a conversation on the provider that already has its prefix cached, since
cached tokens are 75–90% cheaper) and **regret detection** (reuse the semantic-cache
embeddings to spot when a downgraded answer is immediately followed by a near-identical
re-ask, and feed that back to the bandit as negative reward — the only way to make "does cheap
routing hurt quality?" falsifiable rather than asserted).

---

## 6. Context compression

`engine/compressor.rs`, stage [6c], six techniques, all lossless-in-effect (never paraphrases
or summarizes — the thing that actually damages quality):

1. **Exact-duplicate system message removal** — agent frameworks resend the whole system
   prompt every turn; only byte-identical duplicates are dropped.
2. **Whitespace collapsing** — outside fenced code blocks only; indentation inside a fence is
   never touched (it can be the program).
3. **History truncation** — past 40 messages, keeps system prompt + most recent 20, inserts an
   explicit marker so the model knows history is partial rather than assuming it's complete.
4. **JSON minification** *(new)* — pretty-printed tool-result JSON re-serialized compactly.
   Provably lossless: the parsed value is identical, only formatting bytes are gone.
5. **Duplicate large-block referencing** *(new)* — a document or file re-sent verbatim later in
   the conversation gets its body replaced with a one-line pointer to the earlier copy, not
   deleted, so turn structure survives. Byte-identical only, above 400 chars.
6. **Stale tool-result trimming** *(new)* — tool output older than 6 messages and large enough
   keeps its head and tail, loses the middle, with a marker stating how much was removed. The
   most recent tool results are never touched.

Measured on a realistic agentic conversation (repeated system prompt, duplicated file reads,
pretty-printed tool output): **2,711 → 485 tokens, 82.1% saved**, all four applicable
techniques firing together.

**Demo/verification endpoint:** `POST /api/compression/preview` — runs the real compressor on
a supplied prompt and returns before/after tokens, a per-technique breakdown, and the saving
priced at the named model's real input rate, **without calling a provider, spending anything,
or writing a usage record.** Built specifically because "we compress your context" is an
assertion and a before/after with money attached is evidence.

**Caveat:** token counts are estimated at `chars/4`, not a real BPE tokenizer — the percentage
saved is trustworthy, the absolute count is approximate.

---

## 7. Governance, budgets, and multi-tenancy

**Current model:** Organization → Teams → Members, with API keys scoped to org and optionally
to a team. Roles: `owner / admin / member / viewer` (org-level only — `team_memberships` has
no role column yet, so there is no "project lead" concept in the schema).

**Budgets:** four scopes today — org, team, API key, region — against a single money limit
(`limit_mc`), enforced by an atomic Redis reserve-then-true-up primitive (proven against real
Redis, not just an in-memory store). No token-denominated limit exists yet, and **no
per-person scope exists** — `AuthContext::from_key` hardcodes `user_id: None` for all API-key
traffic (which is all gateway traffic), and `usage_records` has no `user_id` column at all.
"How much did this specific employee spend" is currently unanswerable at any layer.

**A larger org/project/budget redesign is drafted but not built**, covering: per-person
attribution, a real "Projects" layer (built on the existing `teams` table) with a project
lead role, project + per-member API keys each independently budgeted, token-denominated
budgets alongside money ones, and three dashboard views (org authority / project lead /
member) each scoped to what that role actually needs to see.

**Policy engine** (`engine/policy.rs`): JSON rules, `[{"when": {...}, "then": {...}}]`, matched
in order, evaluated after the passthrough hint and before the classifier. `when` supports
`complexity`, `model_requested` (wildcard), `team`, `requires_tools`, `min_input_tokens`.
`then` supports `model_tier`, `max_model_tier`, `pin_model`, `deny`, `passthrough`. A rule that
doesn't parse to this exact shape is rejected with a 400 at save time (found and fixed this
session — the dashboard's own built-in presets were using an invented schema and had never
successfully saved).

---

## 8. Providers

10 registered adapters, each implementing a shared `Provider` trait (`id`, `supported_models`,
`chat`, `chat_stream`, `estimate_cost`): **openai, anthropic, google, custom (any
OpenAI-compatible base URL), openrouter, moonshot, deepseek, mistral, groq, vertex**. Golden-file
translation tests in both directions per adapter.

Two public wire surfaces, both reaching every provider (the router dispatches on the `model`
field independent of which endpoint received the call):

- `POST /v1/chat/completions` — OpenAI-shaped
- `POST /v1/messages` — Anthropic-shaped, accepts `x-api-key` or `Authorization: Bearer`

**No native Google/Gemini SDK passthrough exists.** Gemini models are called by naming them
(`google/gemini-2.5-flash`) through either of the two surfaces above.

A separate, explicitly non-authoritative `openrouter_pricing_reference` table stores real
OpenRouter pricing data (420 models) purely as a human cross-check — never read by the router
or by billing.

---

## 9. What's genuinely verified — read this before trusting any claim

This project grades every feature by evidence, not intention, because several things were
believed correct for multiple sessions before a live run or a real user report surfaced a
genuine defect (a fabricated password hash that never actually worked; a dashboard tier filter
comparing against values the API never sends; policy presets that had never once saved
successfully). The grading:

- **Live** — confirmed running against real Postgres/Redis/Qdrant, by hand, this project.
- **Tested** — passing automated test suite; not re-run against live infra recently.
- **Partial** — built and working, with a specific named gap (e.g., SSO/SCIM work end-to-end
  against static fixtures, never against a real Okta/Entra tenant).
- **Planned** — blueprint item, not yet built or not met.

**Never done, at any point in this project's history:** a real completion against any live
provider account (no API key has ever been supplied), the Stripe checkout round trip, the k6
load test at real network scale (1k RPS — an in-process substitute exists: 64-way concurrency,
2,560 requests, P99 1.27ms), a restore-from-backup drill (no backup has ever been taken),
deployment anywhere outside a local machine.

**As of this document, Docker Desktop is down** on the development machine — crashed on a
corrupted startup socket (`sailor-ingest.sock`) that survives every removal method tried
(direct delete, PowerShell, `cmd`, `fsutil reparsepoint delete`), with no process holding a
lock on it. Standard fix is a machine restart, not yet done. Until then nothing can be
verified against live Postgres/Redis/Qdrant, and the gateway cannot start with a database
configured.

---

## 10. Test suite / quality gates

As of commit `50213fd`: **870 tests passing** (819 lib + 51 across integration binaries: auth,
tenant isolation, durable cache, overhead-under-load, redis-concurrency, route-surface). Gates
that must stay clean: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm
run lint` (ESLint + a design-token guard script), `npm run build`.

68 of 69 tasks across all 8 build phases are checked off in `docs/PHASES.md` — the one
remaining is the load test at real network scale.

---

## 11. Where to look for more

- **`MASTER_BUILD.md`** — the immutable original blueprint (business model, principles, full
  schema, full API spec, deployment topology). Deviations require an ADR.
- **`MEMORY.md`** — the living project state, updated every session, with a full session-by-
  session log of what was found and fixed and why. **The single most useful file for picking
  up where things left off.**
- **`docs/PHASES.md`** — per-phase task lists with an evidence column (what's actually proven,
  not just built).
- **`docs/adr/`** — every deliberate deviation from the blueprint, with rationale (e.g. the
  third durable cache tier beyond the original two-tier spec, the Vertex AI JWT-signing
  approach).
- **`CLAUDE.md`** — the behavioral contract this project's AI sessions operate under (read
  MEMORY.md first, every session ends with MEMORY.md updated, money is always integers, every
  DB query is org-scoped, no exceptions).

---

## 12. House style, if you're generating code or docs for this project

- Conventional commits (`feat(scope): message [P#.#]`), referencing a phase task where one
  applies.
- Rust: `cargo fmt` + `clippy -D warnings` clean, no `unwrap()`/`expect()` outside tests and
  startup, every public fn documented, all I/O async.
- TypeScript: strict mode, no `any` in committed code, server components by default.
- Every module ships tests in the same commit. Untested code is unfinished code.
- Comments explain *why*, not *what* — this codebase's comments routinely cite the specific
  bug or audit finding that produced a piece of logic, which is worth matching.
- The dashboard uses a specific "Deskwork" design system (light warm-cream canvas `#EAE3D3`,
  paper surfaces a shade lighter, hard offset shadows, ink borders `#211C14`, red accent
  `#A8341E`, three type voices — Spectral serif for documents, Space Grotesk for UI chrome,
  JetBrains Mono for exact values) — defined in `apps/web/app/globals.css`. Match it rather
  than introducing new visual language.

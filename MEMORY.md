# MEMORY.md — Living Project State

> **If you are an AI agent or a new engineer picking this project up: start here.**
> This file is the handoff protocol. It tells you where the project is, what works, what
> does not, what was decided and why, and exactly what to do next.
>
> **Last updated:** 2026-08-20 — Session 1
> **Updated by:** Claude Opus 5 (Claude Code)
> **Update this file before ending any session.** See `CLAUDE.md`.

---

## 30-Second Orientation

**Aegis** is a multi-tenant AI cost-optimization gateway. Apps point at us instead of
OpenAI/Anthropic/Google by changing one base URL. We route each request to the cheapest
model that preserves quality, cache aggressively, meter every token, and prove the savings
per request. We charge a subscription plus a share of verified savings.

- **Core product:** `apps/gateway` (Rust + Axum). This is where the value is.
- **Control plane:** `apps/web` (Next.js 15). Dashboard + marketing.
- **Blueprint:** `MASTER_BUILD.md` — never contradict it without an ADR.
- **Money rule:** micro-cents integers everywhere. 1 cent = 10,000 micro-cents.
- **Tenancy rule:** every query scoped by `org_id`.

---

## Phase Status

| Phase | Goal | Status |
|-------|------|--------|
| 0 | Foundation: repo, CI, dev stack, schema, config | 🟢 complete |
| 1 | Auth, keys, orgs | 🟡 gateway side done; web + wiring in progress |
| 2 | Core gateway (proxy + metering) | 🟢 complete |
| 3 | Optimization engine | 🟢 complete |
| 4 | Dashboards, billing, launch prep | ⚪ not started |
| 5 | Launch + provider expansion | 🟡 providers + classifier v2 done |
| 6 | Enterprise readiness | ⚪ not started |
| 7 | Scale + moat | 🟡 bandit done |

Legend: ⚪ not started · 🟡 in progress · 🟢 complete · 🔴 blocked

Machine-readable equivalent: `.aegis/state.json`.

---

## Current Focus

Completing the HTTP surface (management API, admin, health), background workers, and
`main.rs`, then the Next.js dashboard.

---

## What Actually Works Right Now

Verified by `cargo test` — **443 unit tests passing**, zero failures.

- **Money math.** Micro-cent integers, savings attribution, plan fee splits. Property
  tests over a wide grid prove the fee never exceeds the saving and the parts always
  reconstitute the whole.
- **Crypto.** AES-256-GCM credential encryption with per-call nonces, HKDF per-tenant key
  derivation, argon2id passwords, uniform base62 key generation.
- **Secret redaction.** Nine credential formats scrubbed from logs, tested against a
  realistic full-request log line.
- **Store.** Redis and in-memory backends behind one trait. Sliding-window rate limiting
  is atomic — a concurrency test proves 50 racing callers against a limit of 10 admit
  exactly 10.
- **Providers.** Nine adapters. OpenAI/Anthropic/Google translation is golden-file tested
  in both directions, including Anthropic's system-prompt hoisting and required
  `max_tokens`, and Gemini's `contents`/`parts` shape. The SSE decoder is proven to lose
  nothing under byte-at-a-time delivery.
- **Classifier.** V1 heuristics and V2 linear model, both 98% on the 100-case labelled
  fixture set. No complex request is ever classified simple (a stricter, separately
  tested bar).
- **Router.** Passthrough hint > policy > complexity, conservative on every ambiguity.
  Tested: complex requests are never downgraded, tool requests are never downgraded,
  routing never promotes to a pricier model, circuit-open providers are skipped.
- **Caches.** Exact (Redis) and semantic (Qdrant/in-memory). Tenant scoping is structural
  — `org_id` is hashed into the fingerprint, so cross-tenant keys cannot collide.
- **Pipeline.** The full twelve-stage path runs end to end against a mock provider: a
  simple request routes cheaper and saves money, a repeat is served from cache at zero
  cost, a zero-retention org is never cached, transient failures retry, 4xx does not.
- **Circuit breakers, compressor, UCB1 bandit** — all tested, including a replay showing
  the bandit beats static routing.

### Not yet verified against real infrastructure

Everything above runs without PostgreSQL, Redis, or Qdrant. The code paths for all three
exist and are used by the same trait-based interfaces, but **no test has run against a
live database** in this session (Docker was unavailable — see Blockers). The integration
tests in `apps/gateway/tests/` are written to run when `AEGIS_TEST_DATABASE_URL` is set,
and CI provides one.

---

## Blockers

| Blocker | Impact | Workaround |
|---------|--------|------------|
| Docker Desktop not running on the build machine | Cannot run Postgres/Redis/Qdrant locally; DB-backed integration tests unverified locally | Gateway compiles and unit-tests with no external service. Integration tests gated behind `AEGIS_TEST_DATABASE_URL`; CI runs them against service containers. |

---

## Known Limitations (be honest about these)

1. **Seed pricing is unverified.** `metering/pricing.rs` carries dated prices for 28
   models, but they were written from memory, not fetched from provider price sheets.
   **This is a launch blocker for billing.** Run `docs/runbooks/pricing-update.md` and
   confirm every row before charging anyone. The schema and code are already built to
   treat the database as authoritative over the seed data.
2. **Classifier V2 does not beat V1.** Both sit at 98% on the fixture set — 100
   hand-written cases cannot separate them. The test asserts "does not regress", not
   "beats", because tuning fixtures until V2 won would measure nothing. V2's real value
   is that it is retrainable from production outcomes without a code change.
3. **Token estimation is approximate.** Four characters per token, used for routing and
   budget projection only. Billing always uses provider-reported counts, and any
   estimated figure is flagged `tokens_estimated` on the usage record.
4. **No load test has been executed.** The k6 script is committed but has not been run
   against a deployed instance, so the sub-1ms P99 claim is a design target backed by
   per-stage measurement, not a measured production figure.

---

## Next Steps (do these in order)

1. Finish the management API routes and the admin console endpoints.
2. Background workers: usage writer, budget alerts, reconciliation.
3. `main.rs` wiring and the Docker/Compose dev stack.
4. Next.js dashboard and marketing site.
5. Verify against real Postgres/Redis once Docker is available; run the integration suite.
6. **Before any real billing:** verify the pricing table (limitation 1 above).

---

## Key Decisions (and why)

Full records in `docs/adr/`. Highlights:

- **ADR-004: runtime-checked SQL, not `query!` macros.** The compile-time macros need a
  live database at build time, which would stop any contributor compiling or testing
  without first standing up PostgreSQL — directly at odds with the requirement that
  anyone can pick this up. Column mapping is covered by integration tests instead.
- **Store behind a trait.** Redis in production, in-memory in development, identical
  semantics. This is what lets the entire pipeline be tested with no infrastructure.
  `Config::validate` refuses to start production on the in-memory backend, because
  per-instance limits stop being limits with two replicas.
- **Scaled integers, not decimals.** `gateway_overhead_us` (microseconds) and
  `complexity_score_milli` (thousandths) rather than `NUMERIC`, avoiding a decimal
  dependency and matching the no-floats house rule.
- **Sign-constrained classifier training.** An unconstrained fit scored 99% on fixtures
  with semantically backwards weights (a longer prompt implying a *simpler* request). The
  constraints cost a point of fixture accuracy and buy a model that generalises.

---

## Gotchas / Traps

- **Do not seed the real pricing table in pipeline tests.** The router will correctly find
  that a real provider is cheaper and the test will make live API calls to OpenAI. Test
  pricing tables must contain only mock models. (This actually happened; see the comment
  in `routes/openai_compat.rs::tests::test_state`.)
- **The Bash tool on this machine wraps commands in `bash -c '...'`,** so a single quote
  anywhere in the command breaks it. Use the Write tool for file content.
- **`serde(flatten)` is required** on `NormalizedRequest::extra` — without it, unknown
  provider parameters are silently dropped rather than passed through.
- **Store `f32` as `f64` for anything that lands in JSON.** `temperature: 0.2` as an
  `f32` serializes as `0.20000000298023224`.

---

## Session Log

Append a dated entry per working session. Newest first.

### 2026-08-20 — Session 1 — Claude Opus 5

Bootstrapped the repository and built Phases 0–3 of the gateway plus parts of 5 and 7.
Wrote `MASTER_BUILD.md`, `CLAUDE.md`, this file, `docs/PHASES.md`. Implemented money,
crypto, telemetry, metrics, store, types, nine provider adapters, the classifier (with a
real train/dump pipeline), router, policies, compressor, circuit breakers, both caches,
the bandit, the full schema, the repository layer, auth/rate-limit/budget middleware, and
the complete request pipeline. 443 unit tests passing.

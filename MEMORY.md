# MEMORY.md — Living Project State

> **If you are an AI agent or a new engineer picking this project up: start here.**
> This file is the handoff protocol. It tells you where the project is, what genuinely
> works, what does not, what was decided and why, and exactly what to do next.
>
> **Last updated:** 2026-08-21 — Session 1
> **Updated by:** Claude Opus 5 (Claude Code)
> **Update this file before ending any session.** See `CLAUDE.md`.

---

## 30-Second Orientation

**Aegis** is a multi-tenant AI cost-optimization gateway. Apps point at us instead of
OpenAI/Anthropic/Google by changing one base URL. We route each request to the cheapest
model that preserves quality, cache aggressively, meter every token, and prove the savings
per request. We charge a subscription plus a share of verified savings.

- **Core product:** `apps/gateway` (Rust + Axum). The value is here.
- **Control plane:** `apps/web` (Next.js 15). Dashboard + marketing.
- **Blueprint:** `MASTER_BUILD.md` — never contradict it without an ADR.
- **Money rule:** micro-cent integers everywhere. 1 cent = 10,000 micro-cents.
- **Tenancy rule:** every query scoped by `org_id`.

First commands:

```bash
bash scripts/status.sh          # what the repo actually contains right now
cargo test --lib                # 622 tests, ~3s, no database needed
bash scripts/verify-phase.sh 3  # automated acceptance checks for a phase
```

---

## Phase Status

| Phase | Goal | Status |
|-------|------|--------|
| 0 | Foundation: repo, CI, dev stack, schema, config | 🟢 complete |
| 1 | Auth, keys, orgs | 🟢 complete |
| 2 | Core gateway (proxy + metering) | 🟢 complete |
| 3 | Optimization engine | 🟢 complete |
| 4 | Dashboards, billing, launch prep | 🟡 built, unverified against live infra |
| 5 | Launch + provider expansion | 🟡 providers + classifier v2 done |
| 6 | Enterprise readiness | 🟡 logic done, HTTP routes not wired |
| 7 | Scale + moat | 🟡 bandit + SDKs done |

Legend: ⚪ not started · 🟡 in progress · 🟢 complete · 🔴 blocked

Detail with per-criterion evidence: `docs/PHASES.md`. Machine-readable: `.aegis/state.json`.

---

## Current Focus

The gateway and dashboard are feature-complete for Phases 0–3 and largely so for 4–7.
**Nothing has been verified against live PostgreSQL, Redis, or a real provider account.**
That is the next block of work, and it is what stands between "built" and "working".

---

## What Actually Works — Verified

`cargo test --lib` → **622 passing, 0 failing**. `clippy -D warnings` clean. `cargo fmt`
clean. Dashboard: `tsc` and `eslint` clean, 15 routes build.

Executed and confirmed by hand this session:

- **The gateway runs and serves.** Started it, hit `/health` (200, per-dependency status),
  `/ready`, `/status`, `/metrics`. Security headers present on every response.
- **Auth rejects correctly.** Unauthenticated → 401 with an actionable message; malformed
  key → rejected before any store lookup; unknown key → 401.
- **No secrets in logs.** Grepped a live gateway log for credential patterns: zero hits.
- **The dashboard renders and calculates.** Landing, pricing, docs, and the dashboard
  error state all verified in a real browser. The savings calculator was exercised: at
  $2,000/month on a "mostly short requests" workload it produces $1,560 avoided, $312
  share, $29 subscription, $1,219 kept — which is arithmetically correct.
- **Both SDKs work.** Python asserted against realistic Aegis response headers;
  TypeScript typechecks.

Covered by tests (not hand-executed):

- **Money.** Property tests over a wide grid prove the fee never exceeds the saving and
  the parts always reconstitute the whole. A million 3-micro-cent charges sum exactly.
- **Crypto.** AES-256-GCM with per-call nonces, HKDF per-tenant keys, argon2id,
  uniform base62 key generation, constant-time comparison.
- **Store.** 50 racing callers against a limit of 10 admit exactly 10.
- **Providers.** Nine adapters, golden-file tested in both directions. The SSE decoder
  loses nothing under byte-at-a-time delivery.
- **Classifier.** 98% on 100 labelled fixtures. No complex request is ever classified
  simple — a separately tested, stricter bar.
- **Router.** Complex requests never downgraded; tool requests never downgraded; routing
  never promotes to a pricier model; circuit-open providers skipped.
- **Caches.** Tenant scoping is structural — `org_id` is hashed into the fingerprint.
- **Pipeline.** Full twelve-stage path end to end against a mock provider.

### NOT verified

- Anything against **live PostgreSQL, Redis, or Qdrant** (Docker was unavailable).
- Anything against a **real provider account** (no API keys).
- The **Stripe** round trip.
- The **load test** — the k6 script is committed but has never been run, so the sub-1ms
  P99 claim is a design target backed by per-stage measurement, not a measured figure.
- **SCIM and SSO HTTP routes** — the logic is tested, the routes are not wired.

---

## Blockers

| Blocker | Impact | Workaround |
|---|---|---|
| Docker Desktop not running on the build machine | No verification against live Postgres/Redis/Qdrant | Gateway compiles and unit-tests with no external service. Integration tests are gated on `AEGIS_TEST_DATABASE_URL` and run in CI. |
| No provider API keys available | Cannot confirm a real end-to-end completion | The mock provider exercises the whole pipeline. Needs one real key to close. |

---

## ⚠️ Launch Blockers

**Do not bill a customer until these are closed.**

1. **Seed pricing is unverified.** `metering/pricing.rs` and `scripts/seed.sql` carry
   dated prices for 28 models, but they were transcribed from memory during the build,
   not fetched from provider price sheets. Every row is marked `UNVERIFIED` in its
   `source` field. Every savings figure and invoice line depends on them.
   → Follow `docs/runbooks/pricing-update.md`. Check with:
   ```bash
   psql "$DATABASE_URL" -c "SELECT COUNT(*) FROM model_pricing WHERE source LIKE '%UNVERIFIED%' AND effective_to IS NULL;"
   ```
2. **Load test never executed.** The landing page states sub-1ms P99 overhead. Run
   `infra/loadtest/k6-gateway.js` against staging and record the result before publishing
   that claim.
3. **Restore drill never executed.** Part 13 item 6: an untested backup is not a backup.

---

## Known Limitations (be honest about these)

1. **Classifier V2 does not beat V1.** Both sit at 98% on the fixture set — 100
   hand-written cases cannot separate them. The test asserts "does not regress", not
   "beats", because tuning fixtures until V2 won would measure nothing. V2 earns its keep
   by being retrainable from production outcomes. See `docs/adr/0006-classifier-versioning.md`.
2. **Token estimation is approximate.** Four characters per token, used only for routing
   and budget projection. Billing always uses provider-reported counts, and any estimated
   figure is flagged `tokens_estimated` on the usage record.
3. **Streaming is OpenAI-only.** `/v1/messages` returns a clear error for `stream: true`.
   Anthropic's named-event SSE format is a different shape and was not built.
4. **Some dashboard pages are missing:** models comparison, org/teams/policies management.
   The API endpoints exist; the pages do not.
5. **SCIM and SSO are logic-only.** Thoroughly tested, but no HTTP routes and never tested
   against a real Okta or Entra tenant.
6. **Not started:** referral program, changelog, compliance pack, read replicas, spend
   anomaly detection, model auto-discovery, investor pack. See `docs/PHASES.md` for the
   annotated list.

---

## Next Steps (in order)

1. **Start Docker and verify against real infrastructure.**
   ```bash
   docker compose -f infra/docker-compose.yml up -d
   export AEGIS_TEST_DATABASE_URL=postgres://aegis:aegis_dev_password@localhost:5432/aegis
   cargo test --tests          # the 14 integration tests will now actually run
   ```
2. **Seed and run the gateway with persistence**, then sign up through the dashboard and
   mint a key end to end.
3. **Add one real provider key** and confirm a live completion, checking the savings
   figure by hand against the provider's own billing.
4. **Verify the pricing table** (launch blocker 1).
5. **Deploy to staging and run the load test** (launch blocker 2).
6. Wire the SCIM/SSO HTTP routes, then the remaining dashboard pages.

---

## Key Decisions (and why)

Full records in `docs/adr/`. The ones that will surprise you:

- **ADR-004: runtime-checked SQL, not `query!` macros.** The macros need a live database
  at *compile* time, which would stop anyone compiling or testing without first standing
  up PostgreSQL — directly against the requirement that anyone can pick this up. Column
  mapping is covered by integration tests instead.
- **ADR-005: the store is behind a trait.** Redis in production, in-memory in dev, same
  semantics. This is what makes the whole pipeline testable with no infrastructure.
  `Config::validate` refuses to start production on the in-memory backend.
- **ADR-006: no ONNX for the classifier.** Twelve features and a linear model do not
  justify a runtime dependency and a model file. Weights are `const` arrays fitted by
  `scripts/train_classifier.py` from features dumped by the real extractor.
- **Sign-constrained training.** An unconstrained fit scored 99% with semantically
  backwards weights (a longer prompt implying a *simpler* request). The constraints cost
  a point of fixture accuracy and buy a model that generalises.
- **Scaled integers, not decimals.** `gateway_overhead_us` and `complexity_score_milli`
  rather than `NUMERIC` — no decimal dependency, and consistent with the no-floats rule.

---

## Gotchas / Traps

Each of these cost real time during the build.

- **Do not put the real pricing table in pipeline tests.** The router correctly finds that
  a real provider is cheaper and the test makes live API calls to OpenAI. This actually
  happened. Test pricing tables must contain only mock models.
- **`serde(flatten)` on `NormalizedRequest::extra` is load-bearing.** Without it, unknown
  provider parameters are silently dropped rather than passed through.
- **JSON floats must be `f64`, not `f32`.** `temperature: 0.2` as an `f32` serialises as
  `0.20000000298023224`.
- **`cargo fmt` collapses `\` string continuations** and bakes the indentation into the
  literal. Use `concat!` for multi-line messages.
- **`command -v python3` is not enough on Windows.** The App Execution Alias shim is on
  PATH and exits with an install prompt. The scripts test by running it.
- **The Bash tool wraps commands in `bash -c '...'`,** so a single quote anywhere in the
  command breaks it. Use the Write tool for file content.
- **Shared test helpers need `#![allow(dead_code)]`.** Cargo compiles `tests/common/mod.rs`
  separately into every test binary, and each uses a different subset.
- **Stop the gateway before rebuilding on Windows** — the running binary is locked.

---

## Session Log

Newest first.

### 2026-08-21 — Session 1 — Claude Opus 5

Built the project from an empty directory through all eight phases.

Wrote `MASTER_BUILD.md`, `CLAUDE.md`, this file, `docs/PHASES.md`, `docs/HANDOFF.md`, six
ADRs, four runbooks, and the handoff tooling (`scripts/status.sh`,
`check-memory-freshness.sh`, `verify-phase.sh`, `update-memory.sh`, `.aegis/state.json`).

Gateway: money, crypto, telemetry with tested redaction, metrics, store abstraction, nine
provider adapters, classifier with a real train/dump pipeline, router, policies,
compressor, circuit breakers, both caches, UCB1 bandit, full schema with monthly
partitioning, repository layer, auth/rate-limit/budget middleware, the complete twelve-stage
pipeline, management and admin APIs, three workers, invoicing, Stripe webhook verification,
licensing, SSO, SCIM, TOTP, residency.

Dashboard: landing with savings calculator, pricing, docs, auth, and seven dashboard pages.

Also: 14 integration tests, k6 load test, both SDKs, CI with a GPL licence gate and a
MEMORY.md staleness gate, seed data, Grafana dashboard.

**Found by running it rather than testing it:** a well-formed unknown API key returned
`internal_error` instead of `unauthorized` when no database is configured. Fixed.

622 unit tests passing, clippy and rustfmt clean, dashboard builds clean.

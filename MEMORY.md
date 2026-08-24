# MEMORY.md — Living Project State

> **If you are an AI agent or a new engineer picking this project up: start here.**
> This file is the handoff protocol. It tells you where the project is, what genuinely
> works, what does not, what was decided and why, and exactly what to do next.
>
> **Last updated:** 2026-08-24
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
cargo test --lib                # 682 tests, ~3s, no database needed
bash scripts/verify-phase.sh 3  # automated acceptance checks for a phase
```

---

## Phase Status

| Phase | Goal | Status |
|-------|------|--------|
| 0 | Foundation: repo, CI, dev stack, schema, config | 🟢 complete |
| 1 | Auth, keys, orgs | 🟢 complete |
| 2 | Core gateway (proxy + metering) | 🟢 complete |
| 3 | Optimization engine | 🟡 exact cache/router/classifier/compressor complete; semantic cache never wired (found session 3) |
| 4 | Dashboards, billing, launch prep | 🟡 built; load test only partially executed (see below) |
| 5 | Launch + provider expansion | 🟢 complete |
| 6 | Enterprise readiness | 🟡 SSO/SCIM/residency wired; TOTP has no repo layer or login check (found session 3) |
| 7 | Scale + moat | 🟡 bandit records outcomes live but is never read from for routing (found session 3) |

Legend: ⚪ not started · 🟡 in progress · 🟢 complete · 🔴 blocked

> `scripts/check-memory-freshness.sh` will warn that phase 5 shows complete while phase 3
> does not, since phases are meant to be strictly ordered. **This is known and
> intentional, not an oversight:** phase 3 was genuinely complete when phase 5 was built —
> it was only downgraded in session 3, retroactively, after an audit found the semantic
> cache had been marked done without ever being wired into the pipeline. Nothing in phase
> 5 depends on semantic caching. Phase 4's remaining item (P4.8, the k6 load test) is
> similarly pure infrastructure execution with no dependency on anything after it. The
> warning is correct to flag both; this note is the confirmation it asks for.

**65 of 69 tasks across all eight phases are checked off in `docs/PHASES.md`.** (Was 68/69
after session 2; a session 3 audit unchecked three that had been marked done in error —
semantic cache, TOTP, and bandit-informed routing — because each is implemented and
tested but never actually invoked from live code. See Known Limitations item 0 for the
full list, which also includes budget threshold alerts, a task line ledger still counts
as done because its UI and enforcement halves are genuinely complete.) One remaining item
(P4.8, the load test) is pure infrastructure execution. The other three are real
feature-completion work, not documentation corrections — "built" no longer means
"finished" for those three until they are actually wired in.

Detail with per-criterion evidence: `docs/PHASES.md`. Machine-readable: `.aegis/state.json`.

---

## Current Focus

All eight phases are code-complete: every handler, every route, every worker described in
`MASTER_BUILD.md` exists, compiles, and is tested. Session 2 closed the last eleven open
phase-task lines (dashboard pages, Anthropic streaming, SCIM/SSO route wiring, the
scheduler, read replica, regional budgets, referral program, compliance pack, investor
pack, self-hosted deployment, changelog) and added a route-surface test plus an in-process
concurrency measurement of gateway overhead.

**Session 3 ran a "is everything we built actually connected" audit**, prompted by the
user's concern that features assembled from different sources might not genuinely work
together. Found a real, consistent pattern: several sophisticated features were built and
thoroughly tested *in isolation* but never wired into the live request path. See "Built
but not wired" under Known Limitations below — this is now the most important thing for
the next session to read, because it changes what several existing documents (this file
included, in earlier revisions) claimed was working. One item (data residency enforcement)
was small and safety-critical enough to fix immediately; it is now genuinely live and
covered by wiring-proof tests. The rest are real feature-completion work, not quick
fixes, and are listed with honest effort estimates rather than attempted under time
pressure.

**What remains is entirely "run it against something real," not "build it":**

1. Docker Desktop — the user reported installing it; this session found only installer
   fragments in `C:\Program Files\Docker\Docker\tmp-delete\`, no `docker.exe`, no running
   daemon. The install did not complete or was rolled back. **This needs to be redone.**
2. A real provider API key (the user selected Google Gemini) has not yet been supplied.
3. Once both land: run the 14+ integration tests against live Postgres, seed data, send
   one real completion, run the k6 load test against a deployed instance, and run the
   backup/restore drill.

Nothing in this list requires further code. It requires infrastructure the build machine
does not have.

---

## What Actually Works — Verified

`cargo test --lib` → **682 passing, 0 failing**. Full `cargo test` (lib + 4 integration
binaries, one of which drives real concurrency) → **702 passing, 0 failing**. `clippy -D
warnings` clean. `cargo fmt --check` clean. Dashboard: `tsc --noEmit` and `eslint .` clean,
all 21 routes build, `next build` produces standalone output.

Executed and confirmed by hand this session (session 3):

- **Data residency enforcement is now genuinely live**, not just implemented and
  self-tested. `enterprise::residency::enforce()` existed and was correct in isolation but
  was never called from `/v1/chat/completions`, `/v1/embeddings`, or `/v1/messages` — an
  organisation pinned to a region was never actually protected from being served by an
  instance elsewhere, despite `docs/compliance/security-whitepaper.md` describing this as
  an active control. Wired into all three entry points, immediately after auth. Four new
  tests prove the *wiring* (not just the logic, which already had its own tests): a
  mismatched region is refused with zero provider calls made; a matching region is served;
  same independently for embeddings and messages.
- **The landing page's green was found and replaced.** The interactive routing simulator
  (`components/routing-simulator.tsx`) and both auth pages used a saturated green
  (`#15803D` / `#059669`) that read as a foreign hue against the cream/camel palette
  everywhere else. Replaced with a new `--color-positive` token (`#6B4423`, a deep coffee
  brown), applied consistently for every "good" indicator — cache hit, low complexity,
  savings percentage — across marketing and dashboard alike. Verified live: the CSS
  variable resolves to the correct hex on the running page after interacting with the
  simulator.
- **A hardcoded metrics-path bug was caught before it shipped.** Wiring residency into the
  embeddings handler meant reusing `record_rejection()`, which had `/v1/chat/completions`
  hardcoded as the Prometheus path label regardless of which endpoint actually rejected
  the request. Fixed to take the path explicitly before it could mislabel a real metric.

Executed and confirmed by hand in session 2:

- **The full dashboard renders correctly**, verified in a real browser (Claude's Browser
  pane) against a throwaway fixture API standing in for the gateway (scratchpad only, never
  committed). All five new pages — models, policies, budgets, team, billing — checked for
  content, console errors, and layout. Confirmed no horizontal overflow at 375px width and
  that wide tables scroll inside their own container.
- **A real, reproducible defect was found and fixed by looking at the rendered page, not
  the code**: nine `var(--color-*)` tokens referenced by the dashboard did not exist in
  `globals.css` — a palette rename during the marketing redesign never propagated. CSS
  custom properties fail silently (inherit rather than error), so `tsc`, `eslint`, and
  `next build` all stayed green while the pages would have rendered with transparent
  backgrounds and inherited text in production. Fixed, and `npm run lint` now runs
  `scripts/check-design-tokens.mjs`, which was verified to actually catch a re-injected
  dangling token before being trusted.
- **Every advertised route is reachable.** `tests/route_surface.rs` probes all 58 routes
  with bare requests and asserts none 404 or 405 — including the SCIM/SSO/governance
  routes added this session, which existed as handlers with no `.route()` line until this
  work. A companion test proves the probe itself is meaningful (an unwired path really
  does 404) and that every tenant-data route refuses an anonymous caller.
- **Gateway overhead under real concurrency, measured, not assumed.**
  `tests/overhead_under_load.rs` drives the actual pipeline (auth → rate limit → budget →
  parse → cache → route → mock-provider call → meter) from 64 concurrent tasks, 2,560
  requests total. Result on this machine, unoptimized debug build: mean 0.24ms, P50
  0.19ms, P95 0.45ms, **P99 1.27ms**. This is not the k6-against-a-deployed-instance test
  `MASTER_BUILD.md` calls for (no network, no real DB/Redis, no 1k RPS) — that is still
  outstanding — but it is real numbers from the real request path under real concurrency,
  which is materially more than existed before.

Executed in earlier sessions and still true:

- **The gateway runs and serves.** `/health` (200, per-dependency status), `/ready`,
  `/status`, `/metrics` all confirmed. Security headers present on every response.
- **Auth rejects correctly.** Unauthenticated → 401; malformed key → rejected before any
  store lookup; unknown key → 401 (this was a real bug, fixed — see Session Log history).
- **No secrets in logs.** Grepped a live gateway log for credential patterns: zero hits.
- **The savings calculator is arithmetically correct**, checked by hand against its inputs.
- **Both SDKs work.** Python asserted against realistic response headers; TypeScript
  typechecks.
- **Pricing is verified**, not transcribed. 33 of 38 priced models were checked in this
  build's earlier session against live provider pricing pages and carry a checked date;
  the remaining 5 (mistral ×2, groq ×2, moonshot/kimi-k2) are explicitly marked
  `UNVERIFIED` in the `source` field rather than silently presented as checked. Check
  count:
  ```bash
  grep -c 'UNVERIFIED.to_string()' apps/gateway/src/metering/pricing.rs   # → 5
  ```

Covered by tests (not hand-executed against live infra):

- **Money.** Property tests over a wide grid prove the fee never exceeds the saving and
  the parts always reconstitute the whole. A million 3-micro-cent charges sum exactly.
- **Crypto.** AES-256-GCM with per-call nonces, HKDF per-tenant keys, argon2id, uniform
  base62 key generation, constant-time comparison.
- **Store.** 50 racing callers against a limit of 10 admit exactly 10.
- **Scheduler.** A 64-way concurrent claim race for the same job/period produces exactly
  one winner — the property the whole distributed-jobs design depends on.
- **Providers.** Nine adapters, golden-file tested in both directions. The SSE decoder
  loses nothing under byte-at-a-time delivery. Anthropic's named-event streaming sequence
  is round-tripped through our own decoder.
- **Classifier.** 98% on 100 labelled fixtures. No complex request is ever classified
  simple — a separately tested, stricter bar.
- **Router.** Complex requests never downgraded; tool requests never downgraded; routing
  never promotes to a pricier model; circuit-open providers skipped.
- **Caches.** Tenant scoping is structural — `org_id` is hashed into the fingerprint.
- **Regional budgets.** Isolated per org, case-insensitive region matching, and adding the
  scope does not start rejecting traffic for anyone who never configured one.
- **Analytics/primary pool separation.** A source-reading test fails if a reporting
  handler reads from the primary pool, or a mutating handler reads from the replica.
- **Pipeline.** Full twelve-stage path end to end against a mock provider.

### NOT verified

- Anything against **live PostgreSQL, Redis, or Qdrant** (Docker unavailable — see
  Blockers; the install appears to have failed partway).
- Anything against a **real provider account** (no API key supplied yet).
- The **Stripe** round trip.
- The **k6 load test against a deployed instance** — `infra/loadtest/k6-gateway.js` is
  committed but has never been run. In-process concurrency evidence now exists (above);
  the 1k-RPS-over-a-network claim does not.
- **The restore drill** — script and runbook exist (`scripts/backup-restore-drill.sh`,
  `docs/runbooks/restore.md`); never executed, because no backup has ever been taken.
- **SSO and SCIM against a real identity provider.** Both are now fully wired and
  reachable at their HTTP routes (they were not, before this session), and their shapes
  are tested against both Okta's and Entra's deprovisioning payload formats — but neither
  has exchanged a single real assertion or provisioning call with an actual IdP.

---

## Blockers

| Blocker | Impact | Workaround |
|---|---|---|
| Docker Desktop not functional on the build machine | No verification against live Postgres/Redis/Qdrant | Gateway compiles and unit-tests with no external service. **Action needed:** the install in `C:\Program Files\Docker\Docker\` only contains leftover installer files (`tmp-delete\Docker Desktop Installer.exe...`), not a working install — reinstall Docker Desktop from scratch, confirm `docker version` succeeds, then run `docker compose -f infra/docker-compose.yml up -d`. |
| No provider API key supplied | Cannot confirm a real end-to-end completion or check a real invoice | The mock provider exercises the whole pipeline, including a 64-concurrency load pass. Needs one real key (user selected Google Gemini) to close. |

---

## ⚠️ Launch Blockers

**Do not bill a customer until these are closed.**

1. **5 pricing rows still unverified.** `mistral/mistral-large-latest`,
   `mistral/mistral-small-latest`, `groq/llama-3.3-70b-versatile`,
   `groq/llama-3.1-8b-instant`, `moonshot/kimi-k2` — all other 33 priced models were
   checked against live provider pages in an earlier session. Follow
   `docs/runbooks/pricing-update.md` for these five, then confirm:
   ```bash
   grep -c 'UNVERIFIED.to_string()' apps/gateway/src/metering/pricing.rs   # must read 0
   ```
2. **Load test not executed against a deployed instance.** In-process concurrency evidence
   now exists (`tests/overhead_under_load.rs`: P99 1.27ms on a debug build, no network, no
   real DB/Redis) — real, but not the same claim as `infra/loadtest/k6-gateway.js` against
   staging at 1k RPS. Run the k6 script once a staging deployment exists, before repeating
   the sub-1ms claim to a customer under real network + database conditions.
3. **Restore drill never executed.** No backup has ever been taken, so none has been
   restored. `scripts/backup-restore-drill.sh` is written; it has not been run once.

---

## Known Limitations (be honest about these)

0. **Built but not wired — found by an explicit audit in session 3, not by accident.**
   Each of these is fully implemented and has its own passing unit tests, but nothing in
   the live request path calls it. This is a materially different (and worse) situation
   than "not started": the tests give false confidence that the feature works end to end.

   | Feature | What exists | What's missing |
   |---|---|---|
   | **Semantic cache** | `cache/semantic.rs`, 624 lines, 15 tests, an HTTP-based Qdrant client and an in-memory test double behind a `VectorStore` trait, tenant-isolated by collection. `CacheOutcome::Semantic` is a real enum variant. | Nothing in the pipeline generates an embedding for an inbound request or calls `SemanticCache::lookup()`. `CacheOutcome::Semantic` is never constructed outside tests. The landing page's routing simulator *demonstrates* a semantic hit — that demo is scripted fixture data, not a real gateway response. **Wiring this changes the pipeline's cost/latency profile** (an embedding call before every cache-miss request), which is a real product decision, not a pure bug fix — flagged rather than silently done. |
   | **Outcome-trained bandit** | `engine/bandit.rs`, UCB1, a 3,000-step replay proving it beats static routing in isolation. `state.bandit.record(...)` **is** called live after every request. | The router never reads the bandit back to *make* a routing decision — it is a write-only data collector right now. The "outcome-trained routing intelligence" claim in `MEMORY.md` and the founder walkthrough artifact describes the intended behaviour, not the current one. |
   | **Budget threshold alerts** (Slack/webhook/email at 50/80/100%) | `workers/budget_alerts.rs`: `crossed_threshold()`, `render()`, `deliver()`, all tested. | None of it is called from the live budget-check path or from any worker. (The **weekly digest** is a different feature in the same file and *is* correctly wired via `workers/scheduler.rs` — do not confuse the two.) Needs a "last threshold alerted" watermark and a decision on whether detection happens inline (adds I/O risk to the 0.1ms-budgeted hot path) or via a periodic job (simpler, small delay). |
   | **TOTP two-factor auth** | `enterprise/totp.rs`, RFC 6238, correct and tested. `users.totp_secret_encrypted` exists in the schema. `users.totp_enabled` is read on every user fetch. | No repo function reads or writes `totp_secret_encrypted` at all. No enrollment endpoint (generate secret, show provisioning URI/QR, confirm a code). No verification step in `POST /api/auth/login`. This is the largest of the four — needs new endpoints, per-user encryption key derivation (existing crypto derives per-*tenant*, not per-*user*), and dashboard UI. |

   **Action needed:** decide priority and scope with the user before building further —
   these are feature-completion work of real size, not one-line fixes like the residency
   wiring above was.

1. **Classifier V2 does not beat V1.** Both sit at 98% on the fixture set — 100
   hand-written cases cannot separate them. The test asserts "does not regress," not
   "beats," because tuning fixtures until V2 won would measure nothing. V2 earns its keep
   by being retrainable from production outcomes. See
   `docs/adr/0006-classifier-versioning.md`.
2. **Token estimation is approximate.** Four characters per token, used only for routing
   and budget projection. Billing always uses provider-reported counts, and any estimated
   figure is flagged `tokens_estimated` on the usage record.
3. **SSO/SCIM are wired but IdP-unproven.** Every route is reachable (confirmed by
   `tests/route_surface.rs`) and shapes are tested against both Okta's and Entra's
   deprovisioning payloads, but no real assertion or SCIM call has ever been exchanged
   with an actual identity provider.
4. **No third-party security attestation.** No SOC 2, no ISO 27001, no penetration test.
   `docs/compliance/soc2-readiness.md` is an honest, itemised gap analysis with a costed
   remediation order — read it before telling a prospect anything about compliance
   posture.
5. **The DPA template has not been reviewed by counsel.** It is technically accurate about
   what the system does; the legal framing needs a lawyer before it goes to a customer.
6. **Pre-revenue, pre-incorporation.** No company entity, no cap table, no customers.
   `docs/investor/data-room-index.md` states this plainly rather than implying otherwise.

---

## Next Steps (in order)

1. **Fix the Docker install, then verify against real infrastructure.**
   ```bash
   docker version   # must succeed before anything below is worth attempting
   docker compose -f infra/docker-compose.yml up -d
   export AEGIS_TEST_DATABASE_URL=postgres://aegis:aegis_dev_password@localhost:5432/aegis
   cargo test --tests          # integration tests will now actually run, not skip
   ```
2. **Seed and run the gateway with persistence**, sign up through the dashboard, mint a
   key end to end, exercise the new pages (`/models`, `/policies`, `/budgets`, `/team`,
   `/billing`) against the real API instead of the fixture server this session used.
3. **Add the Gemini provider key** and confirm one live completion, checking the savings
   figure by hand against Google's own billing.
4. **Close the 5 remaining unverified pricing rows** (launch blocker 1).
5. **Deploy to staging, run the k6 load test** (launch blocker 2), then **run the restore
   drill** (launch blocker 3).
6. Only after 1–5: consider the compliance items in `docs/compliance/soc2-readiness.md`
   that require a live deployment (capacity monitoring, recovery testing).

---

## Key Decisions (and why)

Full records in `docs/adr/`. The ones that will surprise you:

- **ADR-004: runtime-checked SQL, not `query!` macros.** The macros need a live database
  at *compile* time, which would stop anyone compiling or testing without first standing
  up PostgreSQL — directly against the requirement that anyone can pick this up. Column
  mapping is covered by integration tests instead.
- **ADR-005: the store is behind a trait.** Redis in production, in-memory in dev, same
  semantics. This is what makes the whole pipeline testable with no infrastructure, and
  what let `tests/overhead_under_load.rs` measure real concurrent behaviour with zero
  external services.
- **ADR-006: no ONNX for the classifier.** Twelve features and a linear model do not
  justify a runtime dependency and a model file. Weights are `const` arrays fitted by
  `scripts/train_classifier.py` from features dumped by the real extractor.
- **Scheduled jobs claim a slot rather than trusting a timer.** A naive `interval` on N
  replicas sends N copies of every weekly digest. `workers/scheduler.rs` has every replica
  attempt to atomically claim a named slot for a named period via `incr_by` (not
  get-then-set, which races) before doing anything with an external side effect. A 64-way
  concurrency test asserts exactly one winner. Store failure declines the claim rather
  than assuming it — skipping a digest once is recoverable, sending four copies is not.
- **The nightly pricing job reports drift and never applies it.** A model price is what an
  invoice is computed from. Auto-applying a scraped price means a customer's bill changes
  because a marketing page changed. A human approves every price change.
- **Regional budgets are a fourth scope, not a variant of the org budget.** An org running
  in several regions has one spend figure but several exposures — the org total says
  nothing about one region burning its allowance by the 9th. Counters are keyed by org
  *and* region; region alone would aggregate every tenant in a region into one number,
  which is a cross-tenant leak with extra steps.
- **The read replica is an accessor, not a parameter threaded everywhere.**
  `AppState::analytics_db()` returns the replica when configured and the primary
  otherwise, so removing the replica from the environment changes performance and nothing
  else. A source-reading test keeps reporting handlers on it and mutating handlers off it,
  because the difference is invisible in every environment without a replica configured —
  which is all of them, until production.
- **Sign-constrained training.** An unconstrained fit scored 99% with semantically
  backwards weights (a longer prompt implying a *simpler* request). The constraints cost a
  point of fixture accuracy and buy a model that generalises.
- **Scaled integers, not decimals.** `gateway_overhead_us` and `complexity_score_milli`
  rather than `NUMERIC` — no decimal dependency, and consistent with the no-floats rule.

---

## Gotchas / Traps

Each of these cost real time during the build.

- **CSS custom properties fail silently.** `var(--color-does-not-exist)` is not a type
  error, not a lint error, not a build error — it just resolves to nothing and the element
  inherits. This is how a palette rename shipped nine dangling dashboard tokens undetected
  through `tsc`, `eslint`, and `next build`. There is now a real guard:
  `scripts/check-design-tokens.mjs`, wired into `npm run lint`, which was verified to
  actually fail on an injected dangling token before being trusted.
- **A Next.js route group layout that needs `export const metadata` cannot be a client
  component.** `(dashboard)/layout.tsx` was `"use client"` for the sidebar/session logic,
  which meant it silently could not export metadata — every authenticated page inherited
  the marketing title and its `index, follow` robots directive. Fix: split the client UI
  into `shell.tsx`, make `layout.tsx` a plain server component that renders it and owns
  `metadata`.
- **Moving page files on disk without clearing `.next/` leaves a stale route manifest.**
  After restructuring the dashboard layout into segment-level layouts, the dev server kept
  serving 404s for pages that existed and compiled — `rm -rf .next` before restarting
  fixed it. If a route 404s right after a file move despite compiling with no errors in
  the server log, this is why.
- **A source-reading Rust test that scans "from this handler's signature to the next
  `pub async fn`" will read past the last handler into its own `#[cfg(test)] mod` and
  pick up every identifier the test itself mentions.** Cut the haystack at `\n#[cfg(test)]`
  before scanning, or the test can fail (or pass) for reasons that have nothing to do with
  the code under test. Hit this writing the analytics-pool guard test in
  `routes/management.rs`.
- **Do not put the real pricing table in pipeline tests.** The router correctly finds that
  a real provider is cheaper and the test makes live API calls to OpenAI. This actually
  happened. Test and load-test pricing tables must contain only mock models.
- **`serde(flatten)` on `NormalizedRequest::extra` is load-bearing.** Without it, unknown
  provider parameters are silently dropped rather than passed through.
- **JSON floats must be `f64`, not `f32`.** `temperature: 0.2` as an `f32` serialises as
  `0.20000000298023224`.
- **`cargo fmt` collapses `\` string continuations** and bakes the indentation into the
  literal. Use `concat!` for multi-line messages.
- **`command -v python3` is not enough on Windows.** The App Execution Alias shim is on
  PATH and exits with an install prompt. The scripts test by running it.
- **The Bash tool wraps commands in `bash -c '...'`,** so a single quote anywhere in the
  command breaks it. Use the Write tool for file content, or a Python script file executed
  afterward.
- **Shared test helpers need `#![allow(dead_code)]`.** Cargo compiles `tests/common/mod.rs`
  separately into every test binary, and each uses a different subset.
- **Stop the gateway before rebuilding on Windows** — the running binary is locked, and
  `cargo build`/`cargo test` fails with "Access is denied" until the process exits.
- **`docker` on PATH does not mean Docker is installed.** Checked this session:
  `C:\Program Files\Docker\Docker\` existed but contained only
  `tmp-delete\Docker Desktop Installer.exe...` — remnants of an interrupted install, no
  `docker.exe`, no daemon. Always confirm with `docker version`, not a directory listing.

---

## Session Log

Newest first.

### 2026-08-21 — Session 3 — Claude Opus 5

User asked for four things in one message: setup/run requirements, a beta-testing plan,
a feature audit ("make sure everything taken from different open-source things actually
works"), and a landing-page color fix (a green they disliked).

**Color fix:** found the green — `components/routing-simulator.tsx` (the homepage's
interactive demo) plus both auth pages' link hover states, `#15803D`/`#059669`. Replaced
with a new `--color-positive: #6B4423` token (deep coffee brown), applied consistently
across marketing and dashboard. Verified live in a browser.

**Feature audit — the important part.** Systematically checked every module under
`engine/`, `enterprise/`, and `workers/` for whether its public API is actually referenced
from a live route or another wired module, versus only from its own tests. Found four
built-and-tested-but-never-connected features: semantic caching, the outcome bandit
(records but is never read from), budget threshold alerts, and TOTP 2FA (also missing its
repo layer and enrollment endpoint entirely). Full detail in Known Limitations item 0
above. Fixed the one that was both small and security-relevant: data residency
enforcement was implemented and tested in isolation but never called from any live
request path, despite the compliance whitepaper describing it as active. Wired it into
all three authenticated entry points with four new wiring-proof tests. Caught and fixed a
latent metrics-mislabeling bug along the way (`record_rejection` had a hardcoded path).

The other three findings are real feature-completion work (new endpoints, a cost/latency
tradeoff decision for semantic caching, a per-user crypto path for TOTP) and were
deliberately left for a scoping conversation rather than rushed.

682 lib tests passing (was 678), clippy clean, web typecheck/lint clean.

### 2026-08-21 — Session 2 — Claude Opus 5

Closed every remaining open task in `docs/PHASES.md` — 11 lines went from unchecked to
checked, leaving 68/69 done. The one remaining item (P4.8, k6-against-a-deployed-instance)
is blocked purely on infrastructure this machine does not have; an in-process concurrency
equivalent was added instead and documented as a distinct, narrower claim.

**Gateway:** Applied the previously-drafted Anthropic streaming patch (`/v1/messages` now
emits the real named-event SSE sequence). Added governance/chargeback/referral endpoints
and their repo functions. Moved router assembly from `main.rs` into the library
specifically so `tests/route_surface.rs` could reach it — this test exists because the new
SCIM/SSO/governance routes were handlers with no `.route()` line, which is a completely
silent failure mode (compiles, unit-tests pass, 404s in production) that nothing had been
checking for. Built the distributed-claim scheduler for the weekly digest and nightly
pricing-drift check. Added read-replica support with a source-reading test that keeps
reporting and mutating handlers on the correct pool. Added regional budgets as a fourth
budget scope. Added `tests/overhead_under_load.rs` (64-way concurrency, 2,560 requests,
real numbers: P99 1.27ms on a debug build).

**Dashboard:** Built the five missing pages (models, policies, budgets, team, billing) and
the `/api/models` endpoint they needed. Found and fixed a real, previously-undetected
defect while verifying in a browser: nine design tokens referenced by the dashboard did
not exist in `globals.css` after a palette rename, and CSS's silent-failure behaviour let
it pass every automated check. Added a real guard for it
(`scripts/check-design-tokens.mjs`), verified the guard actually catches the failure mode
before trusting it. Also found and fixed authenticated pages being indexable with the
wrong title, caused by a client-component layout being unable to export metadata.

**Infra & docs:** Self-hosted Docker Compose stack + web Dockerfile (standalone Next.js
output). Compliance pack (security whitepaper, subprocessors, DPA template, data-flow
doc, honest SOC 2 gap analysis with a costed remediation order). Investor pack (metrics
definitions naming the exact code that computes each figure, and a data-room index that
states plainly what does not exist yet — no incorporation, no customers, no attestations).
Changelog with "breaking change" defined to include routing/pricing semantics, not just
schema changes.

**Attempted and abandoned:** tried to start the gateway against a live Docker stack to do
real end-to-end verification. Found Docker Desktop's install directory contains only
leftover installer files, not a working installation — documented as a blocker with the
exact path, so the next session (or the user) does not have to rediscover this.

698 tests passing (678 lib + 20 across four integration binaries), clippy and rustfmt
clean, dashboard builds clean, 68/69 phase tasks checked off.

### 2026-08-21 — Session 1 — Claude Opus 5

Built the project from an empty directory through all eight phases.

Wrote `MASTER_BUILD.md`, `CLAUDE.md`, this file, `docs/PHASES.md`, `docs/HANDOFF.md`, six
ADRs, four runbooks, and the handoff tooling (`scripts/status.sh`,
`check-memory-freshness.sh`, `verify-phase.sh`, `update-memory.sh`, `.aegis/state.json`).

Gateway: money, crypto, telemetry with tested redaction, metrics, store abstraction, nine
provider adapters, classifier with a real train/dump pipeline, router, policies,
compressor, circuit breakers, both caches, UCB1 bandit, full schema with monthly
partitioning, repository layer, auth/rate-limit/budget middleware, the complete
twelve-stage pipeline, management and admin APIs, three workers, invoicing, Stripe webhook
verification, licensing, SSO, SCIM, TOTP, residency.

Dashboard: landing with savings calculator, pricing, docs, auth, and seven dashboard pages.

Also: 14 integration tests, k6 load test, both SDKs, CI with a GPL licence gate and a
MEMORY.md staleness gate, seed data, Grafana dashboard.

Also verified pricing against live provider pages (33 of 38 models checked; 5 explicitly
flagged UNVERIFIED rather than silently presented as checked) and wrote
`engine/governance.rs` and `routes/enterprise.rs` — the latter not yet wired into the
router at end of session, which session 2 above closed.

**Found by running it rather than testing it:** a well-formed unknown API key returned
`internal_error` instead of `unauthorized` when no database is configured. Fixed.

622 unit tests passing, clippy and rustfmt clean, dashboard builds clean.

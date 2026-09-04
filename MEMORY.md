# MEMORY.md — Living Project State

> **If you are an AI agent or a new engineer picking this project up: start here.**
> This file is the handoff protocol. It tells you where the project is, what genuinely
> works, what does not, what was decided and why, and exactly what to do next.
>
> **Last updated:** 2026-09-04
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
cargo test --lib                # 774 passing, 0 failing, 0 ignored, ~5s, no database needed
bash scripts/verify-phase.sh 3  # automated acceptance checks for a phase
```

**Read this next, before anything else:** [Aegis Enterprise Readiness Audit](https://claude.ai/code/artifact/aa6e48d0-50e5-445f-a8c0-70417960003e)
— a full 16-section audit (pricing, security, reliability, scale, routing, testing,
enterprise-deployment fitness, a 0-10 scorecard, and a P0-P3 findings list) done session 5
by tracing real execution paths and running adversarial tests, not by reading
documentation, **updated in place session 6** after a remediation pass closed every P0/P1
finding and 9 of 10 P2s — the scorecard and every finding status now reflect what's
actually fixed, with struck-through prior values kept visible rather than silently edited
away. This file's Known Limitations section below summarizes the findings that still
change project state; the artifact has the full evidence for each, updated to match. Also
current: the [Aegis Field Manual](https://claude.ai/code/artifact/fc18e8d1-cb1d-40d6-88d5-393be51bf276),
the founder-level walkthrough of the whole system, updated the same session; and the
[Aegis Control Room](https://claude.ai/code/artifact/3e1c0060-2d00-4995-bbf6-3e113dcf6e87)
(session 6), the operator's answer to how it's deployed, the org-role vs.
platform-admin control split, plan/billing mechanics, and how self-hosted licensing works;
and [Aegis Journeys](https://claude.ai/code/artifact/196f024b-ffc7-4955-bfc9-e5cb7c175446)
(new session 9), the individual/organization/admin workflow walkthrough, unusual among
these artifacts for marking each claim by whether it was live-verified in a real browser
against a real running gateway+dashboard or only checked against source.

---

## Phase Status

| Phase | Goal | Status |
|-------|------|--------|
| 0 | Foundation: repo, CI, dev stack, schema, config | 🟢 complete |
| 1 | Auth, keys, orgs | 🟢 complete; TOTP now fully enforced end to end (session 6) |
| 2 | Core gateway (proxy + metering) | 🟡 budget check is now atomic under concurrency — reserve-then-true-up, proven against `MemoryStore` and real Redis (fixed session 6, was found session 5). Only remaining gap: 9 pricing rows still unverified — a data task, not a code gap |
| 3 | Optimization engine | 🟢 complete; fallback chain populated with real cross-provider alternates, streaming has full retry/fallback/health-tracking, routing is health- and bandit-informed (session 6). Semantic cache is now wired in too (session 7), plus a new third cache tier not in the original spec — see ADR-008 |
| 4 | Dashboards, billing, launch prep | 🟡 built; budget threshold alerts now actually deliver (fixed session 6); k6 load test against a deployed instance still not executed (infra-blocked, see below) |
| 5 | Launch + provider expansion | 🟢 complete; Vertex AI added session 5 as a tenth provider (`docs/adr/0007-vertex-ai-jwt-signing.md`), not yet verified against a real GCP project |
| 6 | Enterprise readiness | 🟡 SSO can now functionally complete (OIDC only — SAML explicitly out of scope), TOTP fully enforced, SCIM tokens self-service, admin audit log correctly org-scoped, customer-facing audit export added (all fixed session 6). Remaining: never verified against a real Okta/Entra tenant |
| 7 | Scale + moat | 🟢 bandit now informs live routing decisions, not just records them (fixed session 6) — the one open item is a Postgres connection-pool scaling wall past ~10 replicas, an infra-sizing task not application code |

Legend: ⚪ not started · 🟡 in progress · 🟢 complete · 🔴 blocked

> `scripts/check-memory-freshness.sh` will warn that phases 3 and 5 show complete while
> phase 2 does not, since phases are meant to be strictly ordered. **This is known and
> intentional, not an oversight:** phases 2, 3, and 5 were all genuinely complete when
> phase 5 was built — phases 2 and 3 were only downgraded retroactively, after audits found
> gaps that existed all along but had gone unnoticed (semantic cache never wired, session
> 3; the budget-check race and the fallback chain's hardcoded-empty alternates, session 5),
> and both have since been re-closed with real fixes (session 6 for phase 2's budget race
> and phase 3's fallback chain; session 7 for phase 3's semantic cache, the last of the
> three). Nothing in phase 5 depends on any of these — Vertex AI (added session 5, also
> phase 5) uses the same pipeline stages as every other provider and inherits none of the
> gaps that mattered. Phase 4's remaining item (P4.8, the k6 load test) is similarly pure
> infrastructure execution with no dependency on anything after it. Phase 2's own remaining
> item (9 unverified pricing rows) is data verification, not a code dependency, so it does
> not block anything downstream either. The warning is correct to flag the ordering; this
> note is the confirmation it asks for.

**68 of 69 tasks across all eight phases are checked off in `docs/PHASES.md`.** (Was 68/69
after session 2; a session 3 audit unchecked three that had been marked done in error —
semantic cache, TOTP, and bandit-informed routing. A session 5 enterprise-readiness audit
unchecked two more — the fallback chain and SSO — for the same reason: implemented and
tested in isolation, but not delivering the capability the task line names once you trace
where it's actually called from. **Session 6 fixed and re-checked four of those five**
(fallback chain, TOTP, SSO for OIDC, bandit-informed routing) with real code and tests, not
by editing the checkbox. **Session 7 closed the last one**: semantic caching, once the
founder made the product call it had been waiting on — see ADR-008 for the design, which
went beyond the original task line and added a third cache tier along with it.) One item
remains unchecked: **P4.8** (the k6 load test — pure infrastructure execution, blocked on
a deployed instance this machine does not have).

Detail with per-criterion evidence: `docs/PHASES.md`. Machine-readable: `.aegis/state.json`.

---

## Current Focus

All eight phases are code-complete: every handler, every route, every worker described in
`MASTER_BUILD.md` exists, compiles, and is tested. Session 5 was a full brutally-honest
enterprise-readiness audit that produced a 16-section report, a 0-10 scorecard (**4/10 at
the time**), and a P0-P3 findings list — three P0s fixed live during the audit, the rest
left as an open, evidenced backlog.

**Session 6's mandate, quoted because it set the scope for everything below:** *"improve
all the metrics and get all the metrics to minimum of seven to eight, if not higher. Work
on everything that you said is not implemented and should make the product stand out the
most. and work on all of its lacunas... and complete it fully."* Followed by "work on all
the things that remaining in development" after the first PR attempt hit a broken `gh`
auth token.

**What session 6 actually did — every open P0 and P1 finding from the session 5 audit, plus
most of P2, closed with real code and real tests, not by editing a checkbox:**

1. **Fixed the budget-check race (P0-01, the single highest-priority finding)** —
   `middleware/budget.rs::check_and_reserve` atomically reserves a *projected* cost against
   every applicable counter (key/team/region/org) before admitting a request, and rolls
   back on refusal; `Reservation::commit()` hands the actual cost to `usage::emit` as a
   pure delta so the correction happens exactly once (an earlier version of this fix
   double-corrected — caught by its own test, see Gotchas). The exact 20-concurrent-
   request/$1.00-limit/$0.05-headroom scenario that used to overshoot 8/8 runs now admits
   at most 1, proven by a rewritten test — and, new, proven against **real Redis**, not
   just `MemoryStore` (`tests/redis_concurrency.rs`).
2. **Populated the fallback chain and fixed streaming resilience (P0-04, P1-01)** —
   `alternates_for()` computes real cross-provider candidates from the pricing table at the
   one call site that used to hardcode `&[]`; streaming now shares the same retry/fallback
   path and health-recording as non-streaming. Added an outer request deadline
   (`AEGIS_REQUEST_DEADLINE_SECS`, default 180s) so nothing can hang unbounded regardless.
3. **Fixed cached-token pricing and Gemini's long-context tier (P0-05)** — `TokenUsage`
   gained `cached_input_tokens`/`cache_write_tokens`; each provider's `parse_usage()`
   normalizes that provider's own cache-token convention (Anthropic additive, OpenAI/Google
   subtractive); `ModelPricing` gained a `LongContextTier` so Gemini 2.5 Pro's 200K-token
   rate is modeled instead of flattened.
4. **Found and fixed a severe pre-existing bug not in the original audit**: `Requirements`
   had no `chat` capability flag, so an embedding model (cheapest price, needs no tools or
   vision) could win a simple chat-routing decision on price alone. Fixed with
   `chat: bool` defaulting to `true`, proven by
   `a_chat_request_is_never_routed_to_an_embedding_model`.
5. **Rebuilt provider health from a binary circuit-breaker signal into a graded
   `HealthScore`** (rolling success rate + EWMA latency) that now actually feeds routing
   selection (`price_penalty()`, `is_degraded()`) — and wired the outcome bandit back into
   `select_at_tier` so it can finally influence which model gets picked, not just log what
   happened after the fact (this closes the long-standing "bandit is write-only" gap from
   session 3).
6. **Made SSO able to functionally complete, for OIDC** (P1-03) — discovery, JWKS
   fetch/verify, and the previously-unregistered `/api/auth/sso/callback` route are all in
   place now. SAML gets an explicit "not yet supported" error rather than a rushed
   implementation — a deliberate scope decision, not an oversight.
7. **Enforced TOTP two-factor end to end** (the largest of session 3's four "built but not
   wired" findings) — repo layer, enroll/confirm/disable endpoints, a new per-*user* HKDF
   key namespace (disjoint from the existing per-tenant one), and a real login-time check.
8. **Sent a real `Idempotency-Key` to every provider on every retry** (P1-04) — generated
   once per fallback-chain attempt, reused only across that attempt's own internal retries.
9. **Built a minimum-viable alerting pipeline from nothing** (P1-05) —
   `infra/prometheus/alerts.yml`, 8 rules across 4 groups.
10. **Proved every atomicity claim against real Redis, not just `MemoryStore`** (P1-02) —
    `tests/redis_concurrency.rs`, 4 tests, the rate limiter/budget-reservation/scheduler-
    claim guarantees this project has always claimed but never actually exercised against
    the real backend.
11. **Closed the smaller P2 gaps**: Vertex service-account keys now redacted in logs;
    expired-session purging now actually runs (a new scheduled window following the
    existing digest/pricing-drift pattern); SCIM tokens are self-service-mintable via a new
    `/api/scim-tokens` surface instead of only by direct DB write; three real `cargo audit`
    findings resolved (removed an `rsa` dev-dependency that was itself triggering an
    advisory, upgraded `lru` past a genuine unsoundness issue, added one scoped documented
    ignore for a phantom lockfile entry with zero build-graph edges); the platform-admin
    audit log was scoped to the calling admin's *own* org — useless for investigating a
    customer — now takes an `org_id` parameter; and a customer-facing
    `GET /api/audit-log.jsonl` export was added where none existed at all, despite the
    compliance whitepaper describing one.

**Deliberately left alone, with reasons**: SAML (explicit scope decision, see #6); the
semantic cache (still an embedding-latency product tradeoff, not a bug); API-key
read-scope narrowing — investigated this session and found `MASTER_BUILD.md` itself
specifies the management API as "session or API-key auth, org-scoped," so the current
behavior is the documented design, not a defect; live-provider/live-infrastructure
verification (blocked by the same absent Docker/no-provider-keys environment constraint
that has blocked it every session).

**What remains, in order**: verify against live infrastructure once Docker works, get real
provider keys for an actual end-to-end completion, run the k6 load test and the restore
drill, close the 9 remaining `UNVERIFIED` pricing rows, and — the only genuinely new
decision this session surfaced — scope the semantic-cache wiring tradeoff with the user
before building it.

---

## What Actually Works — Verified

`cargo test --lib` → **774 passing, 0 failing, 0 ignored** (the session-5 budget-race
`#[ignore]`d proof no longer exists as a documented-bypass — it was replaced by a passing
proof-of-fix, `concurrent_requests_cannot_overshoot_a_hard_budget`, see session 6 below).
Full `cargo test` (lib + integration binaries, including a new `tests/durable_cache.rs`) →
**814 passing, 0 failing, 0 ignored**. `clippy --all-targets -D warnings` clean. `cargo fmt
--check` clean. Dashboard: `eslint . && check-design-tokens.mjs` clean, `next build`
produces 23 static routes with no errors.

Executed and confirmed by hand session 7:

- **A differently-worded repeat of a cached question is now actually served from cache,
  not re-billed.** `a_differently_worded_repeat_hits_the_semantic_cache` proves the full
  chain: miss → provider call → second, unrelated-looking wording → semantic hit, zero
  cost, zero provider calls → the *same* wording asked a third time hits the exact tier
  directly (proving the promote-on-semantic-hit optimization actually fires). A companion
  test with a real per-text embedder (not a fixed test vector) proves two genuinely
  unrelated questions never collide.
- **Free-tier and zero-retention orgs are structurally excluded**, not just told not to
  use it — `free_tier_never_gets_semantic_caching` and
  `a_zero_retention_org_never_gets_a_semantic_hit_even_with_an_embedder_configured` both
  configure an embedder that *would* produce a hit and prove the plan/retention gate stops
  it before the embedder is ever consulted.
- **The durable tier's tenant isolation, encryption round trip, sliding expiry, and purge
  job were each proven against a real Postgres schema**, not just the encryption math in
  isolation — `tests/durable_cache.rs`, 6 tests, gated on `AEGIS_TEST_DATABASE_URL`,
  confirmed to compile and skip correctly here (Docker still unavailable on this machine).

Executed and confirmed by hand this session (session 6):

- **The budget-race proof now proves the fix, not the bug.** The old
  `#[ignore]`d `concurrent_requests_can_overshoot_a_hard_budget` (8/8 reproductions) is
  gone; `concurrent_requests_cannot_overshoot_a_hard_budget` runs the identical 20-
  concurrent-request/$1.00-limit/$0.05-headroom scenario, unignored, in the normal suite,
  and asserts at most 1 admission. A companion test,
  `concurrent_requests_fill_a_budget_exactly_to_the_line`, proves the fix didn't
  overcorrect into under-admitting. Both pass. **Also proven against real Redis**, not just
  `MemoryStore`: `tests/redis_concurrency.rs::budget_reservation_is_atomic_against_real_redis`
  and `a_refused_reservation_leaves_the_real_counter_untouched` — gated on
  `AEGIS_TEST_REDIS_URL`, confirmed to compile and skip gracefully locally (Docker still
  unavailable this session), designed specifically to close the session-5 finding that no
  test in the repo read the Redis env var CI provisions.
- **The severe embedding-routing bug is closed and regression-tested.** Before this
  session, `Requirements::default()` had no `chat` field, so a request with only a
  `messages` array (the overwhelming majority of traffic) imposed no chat requirement at
  all — an embedding model, needing no tools/vision/large context, would legitimately win
  on price. `a_chat_request_is_never_routed_to_an_embedding_model` and a companion
  degraded-health test both pass.
- **A degraded provider genuinely loses routing weight now, calibrated against real seed
  prices, not synthetic ones.** `a_degraded_provider_loses_to_a_healthy_one_when_the_penalty_outweighs_the_price_gap`
  targets `groq/llama-3.1-8b-instant` ($0.0575/Mtok) against
  `openai/gpt-5-nano` ($0.1375/Mtok, ~2.4x) with enough recorded failures (16/20, 20%
  success rate) to produce a price penalty that actually exceeds the real gap — and a
  complementary test proves a *mild* degradation (90% success) does not flip the choice,
  so the mechanism isn't just always picking the healthier option regardless of price.
- **SSO's OIDC flow was run end to end at the unit/handler level, not just the assertion
  math.** `select_decoding_key` is tested against realistic JWKS shapes (multiple keys,
  `kid` matching, an unmatched `kid`); `verify_id_token` is tested against static PEM
  fixtures for two independent keypairs (audience/issuer/expiry/signature-mismatch all
  independently break verification). SAML is proven to return a clear, distinct error
  rather than attempting anything.
- **TOTP's full lifecycle was proven, not just the RFC 6238 math.**
  `totp_protects_login_end_to_end` drives enroll → confirm → a login attempt with no code
  (rejected with `totp_required`) → a login attempt with the correct code (succeeds) against
  a real handler and a real database-shaped test path. A companion test proves confirming
  without enrolling first is rejected, and that an API key (not a session) cannot manage
  TOTP at all.
- **The idempotency key genuinely reaches the provider call, and is reused correctly
  across retries but not across fallback attempts.** `mock.rs`'s `RecordedCall` now
  captures the key it was sent; `the_idempotency_key_reaches_the_provider_call` and
  `a_retried_request_reuses_the_same_idempotency_key` both pass against the real call path,
  not just the type signature.
- **The platform-admin/customer audit-log split was proven with three separate roles, not
  just a happy path.** `a_platform_admin_can_inspect_a_different_organisations_audit_log`
  (staff, own org membership irrelevant, reads another org via `?org_id=`),
  `an_ordinary_member_cannot_reach_the_admin_audit_endpoint` (a non-admin session gets the
  deliberate 404-not-403), and `a_customer_can_export_their_own_audit_log` (an ordinary
  reader gets their own org's JSONL) all pass against a real handler.
- **`cargo fmt --check`, `clippy --all-targets -- -D warnings`, and the full `cargo test`
  suite all re-run clean after every fix above**, on this machine, this session.

Executed and confirmed by hand session 5:

- **Vertex AI is a registered, tested provider** — `providers::tests::builtin_registry_has_every_shipped_provider`
  now asserts 10 providers including `"vertex"`. JWT claim construction/expiry, malformed-
  key rejection, credential-shape validation (rejects an AI-Studio-shaped key with a
  message naming what's actually expected), region default/override, and byte-identical
  request bodies vs. the existing `google.rs` adapter are all covered by real, run tests.
  What is not verified: an actual OAuth2 token exchange or `generateContent` call against
  a real GCP project — this environment has never had one, for any provider.
- **The SSRF fix closes the exact exploit chain, proven against real code and a real
  database, not just unit-level.** `tests/auth_and_billing.rs::a_freshly_signed_up_org_cannot_register_a_provider_pointed_at_cloud_metadata`
  drives the real `management::create_provider` handler with a real session and a real
  Postgres-backed org, asserts a 400 and that nothing was persisted (`repo::list_credentials`
  stays empty). A companion test (`a_legitimate_custom_endpoint_is_still_accepted`) proves
  the fix didn't also break the legitimate case. Both skip gracefully without
  `AEGIS_TEST_DATABASE_URL` and were confirmed to at least compile and skip correctly
  locally — the assertions themselves have not yet run against a live database this
  session (Docker still unavailable), consistent with every other DB-gated test's status.
- **The budget-bypass race is real, not theorised** — reproduced 8 of 8 runs, numbers in
  `apps/gateway/src/middleware/budget.rs`'s `concurrent_requests_can_overshoot_a_hard_budget`
  test comment. First attempt under the default single-threaded test runtime did *not*
  reproduce it, which was itself a finding (a race-condition test needs genuine OS-thread
  parallelism, not cooperative single-thread scheduling, or it gives false confidence) —
  fixed by adding `flavor = "multi_thread", worker_threads = 8` and a `tokio::sync::Barrier`
  forcing simultaneous execution.
- **The metering-completeness fix was independently corroborated**, not just self-reported
  — a parallel focused investigation into observability found the exact same discarded-
  `Result` bug at the exact same call sites before either the direct trace or the
  investigation knew of the other's finding.
- **`cargo fmt --check`, `clippy --all-targets -- -D warnings`, `cargo test`, `npm run
  lint`, and `npm run build` all re-run clean after every fix above**, on this machine,
  this session — not carried forward from an earlier session's report.

Executed and confirmed by hand in session 3:

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
- **Pricing is verified**, not transcribed, for the majority of the table. The remaining
  9 unverified rows are explicitly marked `UNVERIFIED` in the `source` field rather than
  silently presented as checked: mistral ×2, groq ×2, moonshot/kimi-k2 (never re-verified),
  `google/gemini-1.5-flash`/`-pro` (session 4 — added as *retired* entries so a legacy
  request is still priced rather than metered at $0, not because anyone confirmed the
  number), and `vertex/gemini-3.6-flash`/`gemini-3.1-pro-preview` (session 5 — preview
  models whose price could not be independently confirmed at build time). Check count:
  ```bash
  grep -c 'UNVERIFIED.to_string()' apps/gateway/src/metering/pricing.rs   # → 9
  ```
  **Found session 5, fixed session 6:** the *verified* rows used to price only input and
  output tokens. Every provider adapter's `parse_usage()` now extracts a cached-token
  figure normalized to that provider's own convention (Anthropic additive, OpenAI/Google
  subtractive with a `saturating_sub`/`.min()` clamp), `TokenUsage` carries
  `cached_input_tokens`/`cache_write_tokens` as first-class fields, and `ModelPricing`
  carries real `CachePricing` rates per model. Gemini 2.5 Pro's 200K-token tier
  ($2.50/$15.00 vs. the flat $1.25/$10.00 it used to be priced at) is now modeled via a new
  `LongContextTier`. ~14 new pricing tests cover both. Full original finding: the audit
  artifact linked above, §1 and §14 (finding P0-05, now closed).

Covered by tests (not hand-executed against live infra):

- **Money.** Property tests over a wide grid prove the fee never exceeds the saving and
  the parts always reconstitute the whole. A million 3-micro-cent charges sum exactly.
- **Crypto.** AES-256-GCM with per-call nonces, HKDF per-tenant keys, argon2id, uniform
  base62 key generation, constant-time comparison.
- **Store.** 50 racing callers against a limit of 10 admit exactly 10 — the **rate
  limiter** (one atomic Redis Lua script). **Budget checking used to be a different, not
  atomic, code path** — session 5 proved it could admit 2-5 of 20 concurrent requests past
  a hard limit with only $0.05 of headroom, 8 of 8 runs. **Fixed session 6**: budget
  checking now goes through the same reserve-then-true-up shape as the rate limiter
  (`check_and_reserve`), and the equivalent 20-concurrent-request scenario now admits at
  most 1 — proven against both `MemoryStore` and real Redis (see Session Log).
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
- **SSO and SCIM against a real identity provider.** SCIM is fully wired and reachable at
  its HTTP routes and its shapes are tested against both Okta's and Entra's deprovisioning
  payload formats, but has never exchanged a real provisioning call with an actual IdP.
  **SSO can now functionally complete, for OIDC** (fixed session 6 — discovery, JWKS
  fetch/verify, and the callback route all exist and are registered) but has never been run
  against a real identity provider end to end, only against static fixtures and
  self-generated test keypairs. SAML remains explicitly unsupported.
- **Any atomic-under-concurrency claim, against real Redis, on this machine specifically.**
  Found session 5: CI provisions a Redis service container and sets an env var for it, but
  no test in the repo read it. **Session 6 wrote the tests** (`tests/redis_concurrency.rs`
  — rate limiter, budget reservation, refused-reservation rollback, scheduler claim, all
  against real Redis) and confirmed they compile and skip gracefully when the env var is
  absent, which it still is on this machine (Docker unavailable). **They have not yet
  actually run against a live Redis instance on this machine** — CI is the only place they
  will execute for real until that changes. Postgres is different: `tests/tenant_isolation.rs`
  and `tests/auth_and_billing.rs` do run against a real database service container in CI,
  confirmed by reading the CI YAML directly.

---

## Blockers

| Blocker | Impact | Workaround |
|---|---|---|
| Docker Desktop not functional on the build machine | No verification against live Postgres/Redis/Qdrant | Gateway compiles and unit-tests with no external service. **Action needed:** the install in `C:\Program Files\Docker\Docker\` only contains leftover installer files (`tmp-delete\Docker Desktop Installer.exe...`), not a working install — reinstall Docker Desktop from scratch, confirm `docker version` succeeds, then run `docker compose -f infra/docker-compose.yml up -d`. |
| No provider API key supplied | Cannot confirm a real end-to-end completion or check a real invoice | The mock provider exercises the whole pipeline, including a 64-concurrency load pass. Needs one real key (user selected Google Gemini) to close. |
| No GCP service account for Vertex AI (new, session 5) | Vertex's OAuth2 token exchange and `generateContent` call are untested against real Google infrastructure | 14 tests cover everything that doesn't require a live GCP project (JWT construction, credential parsing, request-body shape). Needs a real service-account JSON key, scoped to a project with the Vertex AI API enabled, to close. |
| `gh auth` token is invalid (new, session 6) | `gh pr create` fails outright — `gh auth status` reports "The token in default is invalid." Every session-6 fix went directly to `origin/main` as a result, consistent with the pattern already established in prior sessions and not objected to by the user. | User runs `gh auth login -h github.com`; a stale `fix/enterprise-audit-remediation` branch from the failed PR attempt was left pointing at an ancestor of `main` (zero unique commits) — safe to delete once `gh` or local git push access works, currently blocked by the same permission classifier that also requires explicit confirmation for branch deletion. |

---

## ⚠️ Launch Blockers

**Do not bill a customer until these are closed.**

1. **9 pricing rows still unverified.** `mistral/mistral-large-latest`,
   `mistral/mistral-small-latest`, `groq/llama-3.3-70b-versatile`,
   `groq/llama-3.1-8b-instant`, `moonshot/kimi-k2`, `google/gemini-1.5-flash`,
   `google/gemini-1.5-pro` (retired entries, session 4), `vertex/gemini-3.6-flash`,
   `vertex/gemini-3.1-pro-preview` (session 5). This is now a pure data-verification task —
   the cached-token/long-context pricing-model gaps that used to accompany this item were
   fixed session 6 (see below). Follow `docs/runbooks/pricing-update.md`, then confirm:
   ```bash
   grep -c 'UNVERIFIED.to_string()' apps/gateway/src/metering/pricing.rs   # must read 0
   ```
2. **Load test not executed against a deployed instance.** In-process concurrency evidence
   exists (`tests/overhead_under_load.rs`, debug build, no network, no real DB/Redis) —
   real, but not the same claim as `infra/loadtest/k6-gateway.js` against staging at 1k
   RPS. Run the k6 script once a staging deployment exists, before repeating any sub-2ms
   claim to a customer under real network + database conditions.
3. **Restore drill never executed.** No backup has ever been taken, so none has been
   restored. `scripts/backup-restore-drill.sh` is written; it has not been run once.
4. **Nothing this session has run against live infrastructure.** Every fix below is proven
   by a real, passing test written in the same session as the fix — a materially stronger
   claim than "implemented," but still not the same claim as "exercised in production" or
   "reviewed by anyone outside this agent." Docker remains unavailable on this machine, so
   the Redis- and Postgres-gated tests that exist to close this gap (`tests/redis_concurrency.rs`,
   `tests/auth_and_billing.rs`, `tests/tenant_isolation.rs`) have been confirmed to compile
   and skip correctly, not confirmed to pass for real, outside of CI.

**Closed this session (session 6) — kept here briefly for the record, no longer blocking:**

- **Budget enforcement bypass under concurrent requests (P0-01)** — was the single
  highest-priority open finding in the audit. `budget::check_and_reserve` now reserves
  atomically before admitting; the exact reproduction scenario (20 concurrent requests,
  $1.00 hard limit, $0.05 headroom) now admits at most 1, proven against both `MemoryStore`
  and real Redis.
- **The fallback chain had zero effective redundancy for the highest-value traffic
  (P0-04), and streaming had no retry/fallback/health-tracking at all (P1-01)** — both
  fixed; see Current Focus above for what changed. An outer request deadline now bounds the
  worst case regardless.
- **Cached-token pricing didn't exist for any provider, and Gemini 2.5 Pro's long-context
  tier wasn't modeled (P0-05)** — both fixed; see "What Actually Works" above.

Fixed and verified closed, session 5 (kept here briefly for the record — no longer
blocking): an SSRF chain from a zero-privilege signup to the gateway's own cloud
infrastructure, and a silent metering-completeness gap that made the one dashboard panel
built to detect billing loss structurally incapable of firing. Both have real regression
tests. Full detail in the Session Log below and the audit artifact.

---

## Known Limitations (be honest about these)

0. **Built but not wired — found by an explicit audit in session 3, most of it closed by
   sessions 6-7.** Each row below was fully implemented with its own passing unit tests,
   but nothing in the live request path called it — a materially worse situation than "not
   started," since the tests gave false confidence the feature worked end to end. **All
   seven of the original findings are now fixed**, real code and tests, no exceptions
   remaining in this table.

   | Feature | Status |
   |---|---|
   | **Semantic cache** | **Fixed session 7.** Wired into `execute_with_headroom` behind a new `cache::embed::Embedder` trait, gated to Pro/Enterprise plans, embedding generated once per request and reused for both lookup and (on a miss) storage. A semantic hit also writes the current wording into the hot exact-match tier, so its own repeat skips the embedding call next time. Went further than the original task: a new third cache tier (`cache::durable`, Postgres, per-tenant-encrypted, 30-day sliding TTL) now promotes any fingerprint the hot tier has proven repeats, so it survives past the hot tier's 24h window too. Full design: `docs/adr/0008-tiered-durable-cache.md`. |
   | **Outcome-trained bandit** | **Fixed session 6.** The router now reads the bandit back via `RoutingInputs::bandit`, folded into `select_at_tier`'s candidate scoring alongside graded provider health and price. Previously write-only. |
   | **Budget threshold alerts** | **Fixed session 6.** `workers/budget_alerts::run` sweeps every org on an interval and delivers exactly one alert per threshold crossing per period (a "last alerted" watermark), spawned from `main.rs`. |
   | **TOTP two-factor auth** | **Fixed session 6.** Repo layer, enroll/confirm/disable endpoints, a new per-user HKDF key namespace, and a real login-time check all added — proven end to end by `totp_protects_login_end_to_end`. |
   | **Fallback chain** | **Fixed session 6.** `alternates_for()` populates real cross-provider candidates from the pricing table at the call site that used to hardcode `&[]`. |
   | **Streaming resilience** | **Fixed session 6.** `open_stream_with_fallback` gives streaming the same retry/fallback path and health-recording non-streaming already had. |
   | **`workers::reconciliation::run` and the budget-alert worker never spawned** | **Fixed session 6.** Both spawned from `main.rs`, alongside a new `workers::health_probe::run` publishing dependency-up gauges. |

   Every item that used to be in this table is now closed.

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
   deprovisioning payloads. SSO can now functionally complete for OIDC (session 6 — the
   previously-missing callback route and JWKS verification are both in place), but no real
   assertion or SCIM call has ever been exchanged with an actual identity provider. SAML is
   explicitly unsupported — the callback returns a clear error rather than attempting it.
4. **No third-party security attestation.** No SOC 2, no ISO 27001, no penetration test.
   `docs/compliance/soc2-readiness.md` is an honest, itemised gap analysis with a costed
   remediation order — read it before telling a prospect anything about compliance
   posture.
5. **The DPA template has not been reviewed by counsel.** It is technically accurate about
   what the system does; the legal framing needs a lawyer before it goes to a customer.
6. **Pre-revenue, pre-incorporation.** No company entity, no cap table, no customers.
   `docs/investor/data-room-index.md` states this plainly rather than implying otherwise.
7. **Operational blind spots found session 5 — three of five fixed session 6.**
   - ~~No alerting pipeline exists at all.~~ **Fixed session 6**: `infra/prometheus/alerts.yml`,
     8 rules across 4 groups (still needs a real Alertmanager/PagerDuty/Opsgenie
     integration in front of it to actually page anyone — the rule file alone is necessary,
     not sufficient).
   - ~~No idempotency key is sent to any upstream provider on retry.~~ **Fixed session 6**:
     every provider now receives a real `Idempotency-Key`, generated once per fallback-
     chain attempt and reused across that attempt's own retries only.
   - **API keys can read broad org-wide management/billing data — investigated session 6,
     found to be documented design, not a defect.** `MASTER_BUILD.md` itself specifies the
     management API as "session or API-key auth, org-scoped" — any valid gateway API key
     can call every `require_reader` endpoint within its own org (never cross-tenant).
     Narrowing this to a per-key permission model would be a deliberate product change
     requiring an ADR, not a bug fix. Left as-is.
   - **No distributed tracing anywhere**, and `x-aegis-request-id` is minted mid-handler
     (after auth/rate-limit already ran) and then essentially never appears in a log line
     — a support engineer holding a customer's own request ID cannot grep for it. **Not
     attempted session 6** — real infra work (a tracing backend), not a code-only fix.
   - **Postgres connection-pool math caps the fleet at roughly 8-10 replicas** with zero
     headroom (N replicas × 20 connections ≤ the reference Postgres instance's connection
     limit) — independent of Redis or CPU. **Not attempted session 6** — infrastructure
     sizing (PgBouncer or a larger instance class), not application code. See the audit
     artifact §6 for the full breakdown by user count.

   None of the remaining two block a first customer. Both would surface in a real
   enterprise security or SRE review. Full evidence for each: the audit artifact linked at
   the top of this file.

---

## Next Steps (in order)

Session 6 closed essentially every code-level P0/P1/P2 finding from the session 5 audit.
What's left is almost entirely "run it against something real" — the same category of gap
that has persisted since session 3, now the dominant one:

1. **Provision a real ONNX Runtime binary + `BAAI/bge-small-en-v1.5` model** and run
   `docs/runbooks/local-embeddings-setup.md` end to end — `cache::onnx_embed::OnnxEmbedder`
   type-checks and passes clippy but has never actually linked, run, or been benchmarked
   in any environment yet (see the session 7 log entry and
   `docs/adr/0009-local-onnx-embeddings.md`). Do the false-positive-rate check against the
   0.95 similarity threshold before wiring it into `AppState` anywhere real — that's the
   one number that actually matters, not just speed.
2. **Fix the Docker install, then verify against real infrastructure** — the single
   highest-leverage remaining infra-verification step, since it unblocks re-running this
   session's entire Redis/Postgres-gated test surface for real rather than confirming it
   merely compiles and skips, and also unblocks the first real test of
   `cache::qdrant_grpc::QdrantGrpcVectorStore` against a live Qdrant instance — the gRPC
   Qdrant path is fully compiled/linked/test-suite-verified (see `docs/adr/0010-qdrant-grpc-client.md`)
   but has never actually connected to a real server.
   ```bash
   docker version   # must succeed before anything below is worth attempting
   docker compose -f infra/docker-compose.yml up -d
   export AEGIS_TEST_DATABASE_URL=postgres://aegis:aegis_dev_password@localhost:5432/aegis
   export AEGIS_TEST_REDIS_URL=redis://localhost:6379
   cargo test --tests          # integration tests, including tests/redis_concurrency.rs and tests/durable_cache.rs, will now actually run
   ```
3. **Add the Gemini provider key** (and a Vertex AI service-account key) and confirm one
   live completion against each, checking the savings figure by hand against the
   provider's own billing — and, if step 1 hasn't landed yet, confirm one real remote
   embedding call for the semantic cache too.
4. **Close the 9 remaining unverified pricing rows** — `docs/runbooks/pricing-update.md`,
   pure data verification now that the cached-token/long-context pricing-model work is
   done.
5. **Deploy to staging, run the k6 load test**, then **run the restore drill**.
6. **Test SSO and SCIM against a real Okta or Entra tenant** — the code path is complete
   for OIDC; only real-IdP verification remains.
7. **Fix `gh auth`**, retry PR creation or push directly, and consider cleaning up the
   stale `fix/enterprise-audit-remediation` branch (zero unique commits vs. `main`).
8. Only after 1–7: the two remaining operational blind spots from the session 5 audit that
   need real infrastructure work, not application code — distributed tracing, and a
   connection-pooling proxy (or larger instance class) ahead of the ~8-10-replica Postgres
   scaling wall — plus the compliance items in `docs/compliance/soc2-readiness.md` that
   require a live deployment. Also worth a look once there's real traffic: surface
   `cache_entries.hit_count` somewhere in the admin console — it's tracked and available,
   nothing reads it yet.
9. **Visually verify the authenticated dashboard pages** (Settings, Keys, Providers,
   Models, Policies, Team, Billing, Savings, Usage) against the new Deskwork theme once a
   real session exists (needs step 2, Docker/Postgres) — Session 10 re-themed every one of
   them via the shared `components/ui.tsx` primitives and a hex-literal sweep, verified by
   `tsc`/`eslint`/the token guard and by direct `getComputedStyle` checks, but never
   actually loaded any of them in a browser with a live session. `docs/design.md` has the
   full token/component reference if something looks off.

---

## Key Decisions (and why)

Full records in `docs/adr/`. The ones that will surprise you:

- **Budget reservations are a pure hand-off, not a second I/O point (session 6).** The
  first version of the atomic-budget fix had both `Reservation::settle()` and
  `usage::emit()` independently correct the counter from projected to actual cost — a
  double-correction bug caught by its own test before it shipped. `commit()` now does no
  I/O at all; it just marks the reservation resolved and returns the amount, so exactly one
  place (`emit`) ever moves the counter. A reservation that is dropped without `commit()`
  or an explicit rollback logs a warning via `Drop` rather than failing silently.
- **SAML is out of scope, deliberately, not silently (session 6).** The SSO callback
  returns a clear "not yet supported" error for a SAML-configured connection instead of a
  rushed or partially-correct implementation. OIDC is what actually works end to end.
- **API-key management-API access stays org-scoped, not narrowed to inference-only
  (session 6).** `MASTER_BUILD.md`'s own API spec describes the management API as
  "session or API-key auth, org-scoped" — this is the documented design, confirmed by
  re-reading the source of truth rather than assumed. Narrowing it to a per-key permission
  model (read-only inference keys vs. full-access keys) is a real product feature, not a
  bug fix, and would need its own ADR if pursued.
- **`Requirements` defaults to `chat: true` (session 6).** Before this, a bare chat request
  imposed no chat-capability requirement at all, so an embedding model — needing no tools,
  vision, or large context — could legitimately win on price alone. Defaulting to `true`
  rather than requiring every call site to opt in matches how every other capability flag
  in `Requirements` already worked, and closes the gap without touching any call site.

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

- **Text color has to be chosen relative to what's actually behind it, not by habit.**
  Repainting `apps/web` to a dark-desk/warm-paper theme (Session 10) meant most page
  background went from light to dark — every section header that has no card wrapper
  (sits directly on the page `<body>`) was still using the "text on paper" color tokens
  and went nearly invisible, dark-text-on-dark-background. `getComputedStyle` on the
  actual rendered element caught it definitively where screenshots alone were ambiguous
  (see the next entry) — when a redesign changes what a bare section sits on, grep for
  every text color used outside a `bg-[var(--color-surface*)]` container and check it by
  hand, don't assume the token name still describes the contrast it used to.
- **The browser-preview screenshot tool can return a stale/blank frame at a non-zero
  scroll position while reporting success** — repeatable, not a one-off timeout retry:
  `computer{action:"screenshot"}` after `window.scrollTo(0, 900)` returned an identical
  solid-color frame on three consecutive attempts (including after an explicit wait),
  while the same call at `scrollTo(0,0)` worked immediately. `getComputedStyle` on the
  actual DOM elements (color, position) is ground truth when this happens — don't trust a
  screenshot's absence of content as proof of a rendering bug without cross-checking
  computed styles first. Resizing the viewport taller (`resize_window` to e.g. 900×1600)
  so the content of interest fits without scrolling worked as a reliable workaround.
- **A near-identical "blank screenshot" in Session 11 had a completely different, more
  mundane cause: a missing `tabId` on `computer{action:"scroll"/"screenshot"}`.** With two
  tabs open (gateway on one, the dashboard on another), several calls silently landed on
  the wrong tab — scrolling and screenshotting the short `/health` JSON response, past its
  few lines into genuinely empty page, not the web app at all. `tabs_select` alone does not
  make subsequent `computer` calls default to that tab; pass `tabId` explicitly on every
  `scroll`/`screenshot` call once more than one tab is open, every time, not just after
  navigating. `getBoundingClientRect()` via `javascript_tool` against the tab you *meant* to
  inspect (not just any tab) is what actually revealed this — the DOM measurements were
  fine all along, only the screenshot calls were pointed at the wrong place.
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
- **A default `#[tokio::test]` runs on one OS thread with cooperative scheduling, which can
  silently mask a real race condition.** Found session 5: the first attempt at the
  budget-race adversarial test used the default single-threaded runtime and did *not*
  reproduce the bug ("admitted: 1", assertion passed) — which looked like proof the race
  didn't exist, and would have been a false negative reported as a clean result. Tasks
  scheduled on one thread tend to run each `check()`-then-`spend()` pair to completion
  before yielding, hiding exactly the interleaving that causes the bug. Fixed by adding
  `#[tokio::test(flavor = "multi_thread", worker_threads = 8)]` and a
  `tokio::sync::Barrier` to force genuinely simultaneous execution — reproduced 8/8 runs
  once real parallelism was forced. Any test whose entire point is proving a concurrency
  bug needs to justify why single-threaded scheduling can't be hiding the thing it's
  trying to prove.
- **A gitignore's blanket `*.pem` rule blocks a legitimate throwaway test fixture just as
  effectively as a real secret.** The Vertex AI JWT tests needed a committed (fake,
  locally-generated, never-connected-to-a-real-account) RSA key to sign test assertions
  against. Fixed with a narrow, explicitly-commented exception line immediately below the
  blanket rule — not by renaming the file to dodge the pattern, which would have quietly
  defeated the rule's actual intent for the next real `.pem` that shows up.
- **A reservation-and-settlement design is easy to accidentally double-correct.** Session
  6's first cut of the atomic-budget fix had `Reservation::settle()` apply
  `actual - projected` to the counter *and* `usage::emit()` separately apply
  `actual - reserved` — both real, both plausible in isolation, together silently
  double-moving the counter. Caught by a test written specifically to check for this shape
  of bug (`emit_does_not_double_count_a_reserved_request`), not by inspection. Any design
  with a "reserve now, correct later" split needs exactly one place that ever applies the
  correction — verify that with a test before trusting the design, not after.
- **`jsonwebtoken`'s RS256 path uses `ring` internally, not the `rsa` crate** — `cargo tree
  -p jsonwebtoken` proves it. Adding `rsa` as a dev-dependency purely to *generate* test
  keypairs at runtime pulls in a crate `cargo audit` flags (RUSTSEC-2023-0071, no fix
  available) for a job static `openssl genrsa`-generated PEM fixtures loaded via
  `include_bytes!` do just as well, with zero runtime dependency footprint.
- **A dependency can sit in `Cargo.lock` — and get flagged by `cargo audit` — without ever
  being compiled**, via another crate's *inactive* optional-feature dependency (here:
  `sqlx-mysql`'s `rsa` dep, gated behind a `mysql` feature nothing in this workspace
  enables). `cargo tree -e normal -p <pkg>` printing nothing is the proof it has zero
  active build-graph edges. `cargo update`/`cargo build` will not prune it while the
  optional-feature-bearing crate remains workspace-visible — the correct fix is a scoped,
  evidence-cited ignore in **`.cargo/audit.toml`** (a root-level `audit.toml` does *not*
  work; the location is load-bearing, confirmed by testing both).

---

## Session Log

Newest first.

### 2026-09-04 — Session 13 — Claude Opus 5

Three pieces, each closing something that was live and wrong rather than merely absent.

**A credentialed-CORS hole, which is the most serious thing found in this project to
date.** `router::cors_layer` called `AllowOrigin::mirror_request()` alongside
`allow_credentials(true)`. Mirroring reflects the caller's own `Origin` back and tells it
credentials are permitted — so any page a signed-in user visited could `fetch()` the
gateway with their session cookie attached and read the response: keys, usage, budgets,
members. Two paths reached it. One was `app_url.contains("localhost")`, which
`https://localhost.evil.com` satisfies. The other was worse: an `AEGIS_APP_URL` that
failed to parse logged a warning and *then mirrored*, so one typo in one production
environment variable silently removed the boundary — failing open on the exact control
that exists to fail closed. Replaced with an explicit predicate: production allows exactly
the configured dashboard origin; development additionally allows loopback and private-LAN
origins parsed properly via `url::Host` (which is what keeps the new `getApiUrl()` LAN mode
working), and nothing else, ever, as a consequence of misconfiguration. `Config::validate`
now refuses to start a production-like process whose `app_url` is unparseable. Six tests,
including the `localhost.evil.com` case the old substring check admitted.

**Per-person attribution, end to end.** `AuthContext::from_key` hardcoded `user_id: None`,
and `usage_records` had no `user_id` column — so for API-key traffic, which is all the
billable traffic there is, no request carried a human identity and "what did this employee
spend" had no answer at any layer. Migration 0009 adds `api_keys.assigned_to_user_id`
(nullable: a shared project key genuinely has no person behind it, and `NULL` is the right
answer rather than a guess) and `usage_records.user_id`, stamped from the key's assignee at
request time and frozen — never resolved back through the key later, because a key can be
reassigned and last month's spend must not move when it is. Threaded through `KeyContext`,
`AuthContext`, `UsageEvent`, the writer's INSERT, and both streaming paths. `POST /api/keys`
accepts `assigned_to_user_id`, refuses it from a non-admin naming somebody else, and refuses
an assignee who is not a member of the organisation (otherwise an admin could stamp another
tenant's user id onto this tenant's billing records).

That assignment also created a permission problem, so it is fixed in the same change: every
reader could list every key in the org, which was defensible when a key belonged only to an
organisation and stops being defensible the moment keys carry a named person and their
personal budget. `list_keys` now returns everything for owners and admins, and own-plus-
shared for everyone else.

**Routing modes made real.** `RoutingHint::Cheap` was parsed from the header and then never
branched on — the router only ever tested `Passthrough` — so a customer sending the
documented `cheap` header to save money got default behaviour and no indication they had
been ignored. Replaced with a five-mode ladder (`passthrough` / `quality` / `balanced` /
`economy`, plus `auto` as the default alias for balanced), expressed as a table on
`RoutingHint::target_tier` and consumed by the router. `cheap` is retained as a synonym for
`economy` rather than broken. The complex band returns `None` in every mode *and* the router
returns before tier logic runs: the quality guarantee is now enforced twice on purpose, so a
mode added later cannot opt out of it by accident. Documented on the `/connect` page as a
fourth tab, which is the first time these were documented anywhere.

895 tests passing (840 lib + 55 integration), `fmt --check` clean, `clippy --all-targets -D
warnings` clean, web `tsc`/`eslint`/design-token guard clean, new tab verified rendering in
a real browser with no console errors.

**Not verified, and worth stating plainly:** the three new database-level tests
(`a_key_issued_to_a_person_attributes_its_usage_to_them`, `a_shared_key_records_no_person`,
`a_member_sees_only_their_own_keys_and_the_shared_ones`) compile and skip cleanly but have
not executed their assertions here — Docker is still down on this machine (the
`sailor-ingest.sock` reparse-point failure from session 12 — see the corrected diagnosis
below, which is worse than "needs a restart").

**Correction to the Docker diagnosis, established at the end of session 13.** Sessions 12
and 13 both recorded this as a stale socket left by an unclean shutdown, fixable by
restarting the machine. That is wrong, and the correction matters because it changes who
can fix it. Evidence: `%LOCALAPPDATA%\Docker\run` was renamed aside so Docker would start
from a clean directory. Docker then started, recreated `run`, created a brand-new
`sailor-ingest.sock` — and failed on that *newly created* file with the identical "file
cannot be accessed by the system" error, timestamped six minutes old at the moment of
inspection. A second, independent socket directory (`%LOCALAPPDATA%\docker-secrets-engine`)
failed the same way and was renamed aside too.

So the problem is not a leftover file. **This machine cannot create usable AF_UNIX socket
reparse points**, which points at a filesystem filter driver — an antivirus or EDR product
— or a damaged Windows AF_UNIX subsystem. Deleting the sockets cannot help, because they
are broken at creation. A restart may or may not clear it; the realistic fixes are Docker
Desktop's "Reset to factory defaults", excluding `%LOCALAPPDATA%\Docker` from the security
product's real-time scanning, or reinstalling Docker Desktop. None of these are reachable
from a shell session.

Two inert directories were left behind by the attempts and can be deleted once Docker
works: `%LOCALAPPDATA%\Docker\run.stale` and `%LOCALAPPDATA%\docker-secrets-engine.stale`.
They contain only the unusable sockets.
They run in CI, which provisions Postgres. Migration 0009 has therefore never been applied
to a live database.

### 2026-09-01 — Session 12 — Claude Sonnet 5

**The standing blocker since session 6 is gone: the founder installed Docker.** Everything
below happened live, against real Postgres/Redis/Qdrant, for the first time in this
project's history — not a claim, a fact worth flagging because so much of this handoff
document has had to say "not verified, no database on this machine" up to this point.

Docker Desktop installed to a non-standard path
(`C:\Users\Acer\AppData\Local\Programs\DockerDesktop\`), not on `PATH` in this session's
shells yet — invoked by full path, works fine (`docker version` confirms 29.7.2, engine
4.88.1). `infra/docker-compose.yml`'s stack (`aegis-postgres-1`, `aegis-redis-1`,
`aegis-qdrant-1`) was already up and healthy, apparently started by the founder before
asking. Two orphaned processes from an earlier session were squatting on ports 8080 and
3000 (`aegis-gateway.exe`, a stray `node`) — killed, not a code issue, just leftover
processes `preview_stop` hadn't tracked. `.claude/launch.json`'s `gateway` entry gained an
`"env"` block (`DATABASE_URL`, `REDIS_URL`, `QDRANT_URL`, `QDRANT_GRPC_URL`) pointing at
the compose stack's standard ports/credentials — first time this was needed, since every
prior session ran without a database at all.

**Gateway boot, for the first time ever with everything connected:** Redis connected,
migrations already applied, 3 usage partitions ready, **32 models loaded from the database**
(not the seed fallback), Qdrant reachable over gRPC, all 6 background workers started
including this session's own `pricing_refresh`. `/health` reports `"database":{"status":"ok"}`.

**Live-verified end to end, all real, all for the first time:**
- Signup → real user + org created in Postgres, real session, redirected straight into
  the dashboard — which rendered correctly, first real authenticated-page confirmation of
  the whole Session 10 design system (previously only confirmed on marketing/auth pages).
- API key creation — a real `aegis_sk_...` key minted, shown once, later calls confirmed
  it authenticates against `/v1/models` (which returned the real 32-model catalogue from
  Postgres, correctly shaped).
- Granted the test account `is_admin` directly via `psql` (the documented, only way — never
  through the API) and drove all three admin pricing endpoints from Session 11 against
  real data for the first time: `GET /api/admin/pricing` → 32 models,
  `unverified_count: 0`, `loaded_from: "database"`; `POST .../reload` →
  `{"reloaded": true, "models": 32}`; `POST .../openrouter/refresh` → `{"fetched": 417,
  "stored": 417}`, independently confirmed via `psql` that all 417 rows actually landed
  in `openrouter_pricing_reference` (412 with a parseable price) — the whole pricing
  hot-reload and OpenRouter reference feature, untestable all last session for lack of a
  database, worked exactly as designed on the first real attempt.
- Budget creation — a real $500/month hard-limit org budget, persisted, rendered correctly
  in the table with the right enforcement badge; the anomaly-detection card above it (this
  session's `DisclosureMeter` component) rendered real backend data ("0 of 7 days needed
  before a baseline means anything," a live z-score bar) instead of the empty/mock state
  every prior verification of that component was limited to.

**Also newly confirmed, correcting something written down as fact in an earlier session's
runbook**: `GET /v1/models` requires an API key. `docs/runbooks/feature-verification.md`
used to say "no key needed to browse" for this endpoint — that line was wrong, fixed;
caught only because a `curl` without a key returned `401` during this session's walkthrough,
not by re-reading the code.

**Next turn — the founder tried to log in with the credentials just handed to them and
couldn't.** Two real, separate bugs, found by actually trying to use what had just been
described as working rather than trusting the description:

1. **`dev@aegis.local`'s password never actually worked, and had never actually worked.**
   The em-dash finding from above turned out not to be cosmetic at all — it was one symptom
   of the same root cause as this. `scripts/seed.sql` hardcoded an argon2id-*shaped* hash
   literal for this account and commented it as "argon2id hash of
   \"aegis-development-password\"", but the literal was fabricated text that happened to
   look like a real hash (correct `$argon2id$v=19$m=...$salt$hash` shape, valid base64,
   right length) — nobody had ever actually run `crypto::hash_password` to produce it.
   Confirmed precisely with a throwaway test calling `verify_password` against the literal
   before touching anything: `false`. Fixed by generating a genuine hash the same way a
   real signup would (`hash_password("aegis-development-password")`) and writing that into
   both `scripts/seed.sql` (for every future `psql -f` run) and the already-seeded row in
   this machine's live database directly (fixing the file alone would not have fixed the
   row that already existed). **Added a regression test** —
   `crypto::seed_sql_dev_account_hash_verifies_against_its_documented_password` — that
   reads the actual file, extracts the actual hash literal, and asserts it verifies against
   the actual documented password, so this exact class of bug (a promise in a comment that
   the code next to it does not keep) cannot silently recur. Writing that test itself hit
   the same trap twice more, worth remembering: a first version counted quoted SQL fields
   positionally and broke the moment the test's own explanatory comment used an apostrophe
   ("codebase's"); a second version searched for a bare `$argon2id$` substring and matched
   an illustrative example *inside* that same comment before it ever reached the real
   literal. Fixed by anchoring the search on `'$argon2id$` — the opening quote a real SQL
   string literal always has and prose in a comment never does — and by not writing a fake
   example hash into the comment in the first place once that was the second time it caused
   a problem.
2. **The `unverified_count` finding from earlier this session was not cosmetic — it was
   this exact bug, from the other side.** The stored `source` text for the five genuinely
   `UNVERIFIED` pricing rows had the same em-dash corruption (`"UNVERIFIED ??? re-check..."`
   instead of `"UNVERIFIED — re-check..."`), introduced when `scripts/seed.sql` — which
   itself has always had the correct em dash, confirmed by reading the file directly — was
   originally loaded into this machine's database through a Windows console whose encoding
   mangled it in transit. Aegis's own `unverified` detection does an exact substring match
   against `metering::pricing::UNVERIFIED`, which does contain the real em dash, so it
   silently matched nothing: the feature meant to surface stale pricing was failing
   specifically on the rows it exists to catch. Fixed by writing the correction as a `.sql`
   file (not a shell argument — the same class of encoding trap, avoided by never routing
   the character through several layers of shell quoting) and running it inside the
   container with `PGCLIENTENCODING=UTF8` explicit. Verified twice: the corrected text
   round-trips visibly through the terminal now, and `GET /api/admin/pricing` (after
   `POST .../reload`, no restart needed — this session's own hot-reload feature closing the
   loop on itself) now reports `unverified_count: 5`, matching `seed.sql`'s own documented
   count exactly, instead of the `0` it silently reported before.

Verified: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
--all-targets` clean — 811 passing (810 + 1 new). Both fixes confirmed live against the
real database, not just by the test suite: `dev@aegis.local` / `aegis-development-password`
now returns `200` from `/api/auth/login` with the correct org attached, and the pricing
endpoint's staleness count is now honest. Real code changes this time, not just a
verification pass — `scripts/seed.sql` and `apps/gateway/src/crypto.rs` — but, like
everything else this session, not yet committed.

**Later the same session — a collaborator's work reconciled in, nothing lost on either
side.** All ~28 files above were first committed intact on a new branch,
`session-11-12-fixes` (commit `fee6fd0`), specifically so that pulling could not silently
discard local work regardless of how the merge went. `git fetch` then showed
`origin/main` 11 commits ahead (`bdfedca`) — a collaborator's work: scheduler jobs moved to
IST (UTC+05:30), live multi-provider model discovery during credential verification,
credential-probe fixes, a full pricing verification pass (every `UNVERIFIED` row in
`model_pricing` replaced with real dated prices — see the runbook's new "Launch Blocker
Resolved" section), ONNX embedder cache activation, and `scripts/seed_dummy_users.sql`.
Local `main` fast-forwarded onto it cleanly (nothing local was on `main` itself — only the
working tree had changes, and those were already safely on the side branch).

Merging `session-11-12-fixes` onto the new `main` produced exactly one real conflict, in
`main.rs` — both sides had independently written a `PricingRow → ModelPricing` converter:
the collaborator's a free function (`into_model`), this session's a `From` trait impl (see
`pricing.rs`). Checked call sites before resolving rather than guessing: the actual startup
code path (`rows.into_iter().map(Into::into).collect()`) already used the trait impl, and
`into_model` had zero callers anywhere in the merged tree — so the resolution kept the
`From` impl and deleted the now-genuinely-dead free function, losing no behavior on either
side. Every other overlapping file (`pricing.rs`, `providers/{anthropic,google,openai}.rs`,
`scripts/seed.sql`, `docs/runbooks/pricing-update.md`, `workers/scheduler.rs`,
`routes/{management,openai_compat}.rs`) auto-merged with no conflict — the pre-merge
line-range analysis (different sections of the same files) held up in practice.

Re-verified the merge result from scratch rather than trusting a clean `git merge` exit
code: `cargo build --all-targets` clean, `cargo clippy --all-targets -- -D warnings` clean.
First `cargo test` pass reported 851 passing — but with `DATABASE_URL`/`AEGIS_TEST_REDIS_URL`
unset in that shell, every Postgres/Redis-gated integration test (`auth_and_billing.rs`,
`durable_cache.rs`, `tenant_isolation.rs`, `redis_concurrency.rs`) had silently taken its
`skip()` early-return rather than actually running — caught while writing this entry's
verification numbers into `.aegis/state.json`, not by re-reading the test code. Re-ran with
both variables pointing at the already-running Docker stack
(`postgres://aegis:...@localhost:5432/aegis`, `redis://localhost:6379`): same **851
passing, 0 failed**, this time genuinely exercising real Postgres and real Redis, including
`redis_concurrency.rs`'s four atomicity tests. `cargo fmt --check` flagged two pre-existing,
whitespace-only diffs in the collaborator's `embed.rs`/`main.rs` additions (not this
session's code) — fixed with `cargo fmt` and re-verified the build stayed clean, committed
separately (`fc07c12`) so the reformat is distinguishable from substantive changes. Web app
also re-verified post-merge since `pricing/page.tsx` was one of the overlapping files:
`npm run lint` (including the design-token check) and `npm run build` both clean, all 21
routes prerendered.

`main` now sits 3 commits ahead of `origin/main`
(`fee6fd0` → `01e1758` merge → `fc07c12` fmt) — reconciled, tested, and formatted, but
**not yet pushed**; push needs the founder's go-ahead since it updates a branch a
collaborator is actively pushing to.

### 2026-09-01 — Session 11 — Claude Sonnet 5

Founder asked how model pricing is maintained and, once told, asked the sharper follow-up:
what happens if a provider changes a price while a request is in flight — does the exact
rate at the exact moment get used, what's the latency cost of checking, and how do we get
this exactly right for every provider. Answered, then built the structural gap the
question exposed.

**The answer, briefly, because it shapes what got built:** no LLM provider exposes a live
pricing API — prices are prose on marketing pages, not a queryable service — so there is
no way to check "is this still real" per request, or even per hour, against the provider
itself. That gap is not solvable by engineering; a human still has to read the page
(`docs/runbooks/pricing-update.md` says exactly this and exactly why: a scraper that is
silently wrong produces a confidently wrong invoice). What *is* engineerable, and was not
built, is the gap between "a human has verified a price and committed it to the database"
and "the gateway is serving it" — which was, until this session, "at the next deploy",
because `AppState.pricing` was a bare `Arc<PricingTable>` loaded once at process startup
with no way to replace it short of a restart. `AppState`'s own doc comment already claimed
"refreshed periodically from the database" — aspirational, not true; no such worker
existed. Confirmed live in a race-condition unit test rather than assumed: a request that
has already read the table is provably immune to a later swap (a computed cost is baked
into the `UsageEvent` at the moment of computation, never re-derived from "whatever the
table currently says" — so a price change five minutes after a request was served can
never retroactively change that request's bill, by construction, not by convention).

**What shipped:**

1. **[`metering/pricing.rs`](apps/gateway/src/metering/pricing.rs)** — added
   `PricingSnapshot` (table + `loaded_at` + `PricingSource::{Database,SeedFallback}`) and
   moved the database-row-to-`ModelPricing` conversion here as `impl
   From<db::repo::PricingRow> for ModelPricing`, out of a private free function in
   `main.rs`, so the new refresh worker and the once-at-startup load share one conversion
   instead of two copies that could drift apart.
2. **[`lib.rs`](apps/gateway/src/lib.rs)** — `AppState.pricing` is now `Arc<RwLock<PricingSnapshot>>`
   instead of a bare `Arc<PricingTable>`, with `AppState::pricing()` (an `RwLock` read plus
   an `Arc` clone — nanoseconds; not meaningfully different in cost from the network call
   to a provider every request already makes) as the read path every handler now calls,
   and `AppState::set_pricing()` as the one place a swap happens. Same pattern already
   established for `AppState::db()` in the 503 fix two sessions ago — a `pub` field plus a
   wrapper method, not a private field, because `main.rs` constructs `AppState` via a
   struct literal from outside the crate.
3. **[`workers/pricing_refresh.rs`](apps/gateway/src/workers/pricing_refresh.rs)** — new
   worker, same shape as the other five in `workers/`: re-reads `model_pricing` every five
   minutes (`REFRESH_INTERVAL`) and swaps it in. An empty result is treated as a likely
   mistake (a truncated table, a half-run migration) and refused rather than swapped in —
   serving a few-minutes-stale table is a strictly better failure mode than pricing every
   request at zero. Spawned in `main.rs` alongside the other DB-dependent workers, only
   when a database is configured.
4. **`POST /api/admin/pricing/reload`** — manual trigger for "I just ran the runbook, I
   don't want to wait five minutes", added to `routes/admin.rs` and `router.rs` next to
   the existing `pricing_table` read endpoint. 503s explicitly when no database is
   configured, rather than a silent `{"models": 0}` that would read as "the reload is
   broken" instead of "this replica never had anything to reload from".
5. **`GET /api/admin/pricing`** extended with `loaded_at`, `loaded_from`
   (`database`/`seed_fallback`), and `unverified_count` — the admin console's staleness
   banner now has something real to read instead of nothing.
6. **`docs/runbooks/pricing-update.md`** — step 5 ("restart the gateway") replaced with
   the reload endpoint; added a short note up top on what the automation does and does not
   cover, and pointed at `workers::scheduler::check_pricing_drift` (a nightly DB-vs-memory
   diff job that already existed, was already wired up, and is now mostly a canary for "is
   the refresh worker itself running" rather than the only thing closing this gap).

**Two real compile-time consequences worth recording, not just fixed:** first, three call
sites (`openai_compat.rs` twice, `management.rs` once) chained `.all()`/`.get()` directly
off `state.pricing()`'s return value and then used the borrowed result in a later
statement — the returned `Arc<PricingTable>` is a temporary now, where it used to be a
field read, so the borrow-checker correctly rejected what used to compile; fixed by
binding `let pricing = state.pricing();` first in each. Second, `cache::embed::ProviderEmbedder`
and two test-only `AppState` literal constructions (`openai_compat.rs`'s `test_state()`,
`tests/overhead_under_load.rs`) built the `pricing` field directly and needed updating to
the new `Arc<RwLock<PricingSnapshot>>` shape — none of these were caught by `cargo build`,
only by `cargo test --all-targets` / `cargo clippy --all-targets`, because they live in
separate test-binary targets. Worth remembering next time a field on `AppState` changes
shape: `cargo build --bin aegis-gateway` is not sufficient proof nothing broke.

Verified: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test
--all-targets` all clean — 786 passing (784 baseline + 2 new: the no-database-is-a-no-op
case and the in-flight-request-is-immune-to-a-later-swap race-condition proof). Not
verified live: no Postgres on this machine (same standing blocker as every session since
6), so the actual `POST /api/admin/pricing/reload` HTTP path and a real refresh cycle were
never exercised against a running database — only the underlying `refresh_once` function,
directly, in-process, against `AppState::for_tests()`.

**Same session, continued — founder pushed further**: asked whether a human can be
bypassed entirely, specifically proposing OpenRouter's live API as a source. Answered
honestly: no third-party aggregator (OpenRouter, Portkey's model repo, or anything else)
can substitute for reading the actual provider's page, because none of them confirm their
number is free of markup or lag — pointing at a reseller instead of the provider just
changes whose error you inherit. The one thing that *is* categorically different: the
provider's own settled invoice/usage API, which this session did not build (a real, larger
piece of work — flagged as the next real lever, not attempted this session) but which
`workers/reconciliation.rs` already has the exact right shape for, structurally.

Founder then asked to actually use OpenRouter's live API — specifically to fetch it and
store it in Postgres, "for now". Built exactly the reference-only role from the
conversation above, deliberately not `model_pricing`:

1. **[`metering/openrouter_reference.rs`](apps/gateway/src/metering/openrouter_reference.rs)**
   (new) — fetches `GET https://openrouter.ai/api/v1/models` (public, no auth) and converts
   each model's decimal-string per-token USD price into the same micro-cents-per-Mtok unit
   `model_pricing` uses, so a future comparison needs no unit conversion. `None` vs `Some(0)`
   kept distinct throughout — "OpenRouter didn't report this price" and "OpenRouter reported
   an actual zero" are different facts, and collapsing them would misrepresent free models
   as unpriced or vice versa. A malformed price on one field, or one unparseable model in a
   response of hundreds, is logged and skipped rather than failing the whole batch.
2. **Migration `0005_openrouter_pricing_reference.sql`** — a brand new table, not a
   modification to `model_pricing`. Its own top-of-file comment states plainly that nothing
   in the request path or router may ever read it without a deliberate decision to revisit
   that.
3. **`db::repo::replace_openrouter_pricing_reference`** — a full delete-and-reinsert inside
   one transaction each fetch, not an incremental upsert, so a model OpenRouter retires
   actually disappears from here instead of going silently stale.
4. **`POST /api/admin/pricing/openrouter/refresh`** (admin-gated, same guard pattern as
   `reload_pricing`) — fetches and replaces the snapshot on demand.

**Real bug caught by the tests, not by review**: the module's own test for "a malformed
price field doesn't take down the other field on the same model" asserted the wrong
expected value (`Some(1_000)` instead of the correct `Some(1_000_000)` for a $0.000001/token
rate) — an arithmetic slip in the *test*, not the implementation, caught immediately because
the test failed. Left in the log because it is exactly the class of mistake this whole
feature exists to make visible instead of silent.

**Validated against real, live data, not just the hand-written fixture**: fetched the actual
current OpenRouter response (`curl`, this machine has outbound internet even without
Postgres) and ran the parser against it directly — 420 real models parsed with zero panics,
415 had a parseable price, 21 were genuinely free. `openai/gpt-4o` came back at
$2.50/$10.00 per Mtok input/output — an exact match to this project's own existing seed
value for the same model, real independent corroboration rather than an assumption. The
temporary test and the live-fetched fixture file were both deleted before committing — not
meant to ship, since a test depending on a live network call and an uncommitted local file
would break the hermetic-test convention every other test in this codebase follows.

Verified: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
--all-targets` clean — 794 passing (786 + 8 new). Not verified: the actual database write
path (`replace_openrouter_pricing_reference` against a real Postgres) — no database on this
machine, so only the fetch-and-parse half was ever exercised against reality; the
insert-side SQL is reviewed and modeled closely on `upsert_pricing`'s existing transaction
pattern but has not itself been run.

**Then: a full rigorous pass across the whole application**, founder's explicit request, not
scoped to just today's changes. Backend: `cargo fmt --check` / `clippy --all-targets -D
warnings` / `test --all-targets` all clean (794/0), `scripts/verify-phase.sh 2` — all 10
automated gates pass. Frontend: `npm run lint` (eslint + `check-design-tokens.mjs`) clean,
`npm run build` clean — all 23 routes statically generate with no errors. Live, both
servers running together: homepage hero/routing-simulator/savings-calculator all verified
interactive (clicking a scenario or a workload option correctly re-renders, confirmed via
DOM text extraction, not just a screenshot) with zero console errors; a real login form
submission end-to-end confirmed the Session 9 503 fix still holds (`POST
/api/auth/login` → 503, `ErrorState` renders it correctly) via `read_network_requests`, not
assumed; the dashboard auth-gate correctly caught the resulting 401 and redirected an
unauthenticated visit to `/login`; all three admin pricing endpoints (including both new
ones from this session) correctly return 401 without credentials, confirmed live via `curl`
after an in-browser `fetch()` attempt mysteriously failed for unrelated reasons (see
Gotchas); mobile viewport (375×812) checked on the homepage — header nav collapses
correctly, hero and stat grid reflow, footer intact.

**Not verified, stated plainly rather than glossed over**: every authenticated dashboard
page (Settings, Keys, Providers, Team, Billing, Usage, Requests, Budgets, Models,
Policies) — still impossible without a real Postgres-backed login session, the same
blocker every session has hit since session 6. The k6 load test and the restore drill
remain untouched for the same reason. `replace_openrouter_pricing_reference`'s actual SQL
has not run against a real database, as noted above.

**Same session, continued once more**: founder pasted the `/connect` page's "Your stack,
unchanged" compatibility list (Cursor, Claude Code, Continue, Cline, Roo, VS Code; OpenAI
SDK, Anthropic SDK, Python, Node.js, LangChain, LlamaIndex; Claude CLI, aider, curl) and
asked directly whether the implementation actually delivers all of it correctly. It mostly
does — every one of those 15 items reduces to "is `/v1/chat/completions` a faithful
OpenAI-compatible surface and is `/v1/messages` a faithful Anthropic-compatible one",
since none of those tools get bespoke code; `NormalizedRequest` models `tools`,
`tool_choice`, `response_format`, and vision content blocks as first-class fields, and a
`#[serde(flatten)] extra: BTreeMap` catch-all round-trips anything unmodeled (seed,
logit_bias, etc.) — genuinely careful compatibility engineering, confirmed by reading the
actual translation code, not assumed from the claim.

**But found one real, confirmed bug in the process, not a hypothetical**: streaming +
tool/function calling was broken on the OpenAI-compatible surface for all six providers
that share `providers::openai::parse_stream_chunk` via the `openai_compatible_provider!`
macro (OpenAI itself, OpenRouter, DeepSeek, Mistral, Groq, Moonshot). The chunk-emptiness
check that correctly drops OpenAI's harmless role-only opening chunk
(`{"delta":{"role":"assistant"}}`, an intentional, tested, correct decision — see
`role_only_first_chunk_is_ignored`) only ever inspected `delta.content` — so a tool-call
delta chunk, which typically has empty/absent `content` on every single one of its chunks
and carries its actual payload under `delta.tool_calls` instead, looked exactly like that
same harmless opener and was silently discarded. The client's tool call simply never
arrived mid-stream — no error, nothing to explain why — which is precisely the failure
mode that would hit Cursor, Continue, Cline, and Roo hardest, since agentic coding tools
are built around streaming + tool-calling together. Non-streaming tool-calling was
unaffected (`parse_response` correctly extracts `/message/tool_calls`) — this was
specifically the streaming path.

**Fixed**: added a `has_tool_call_delta` check alongside the existing content/finish/usage
checks in `providers/openai.rs::parse_stream_chunk`. The fix was narrow because the hard
part was already built correctly — `raw: Some(data.to_string())` was already captured
unconditionally, and the forwarding loop in `routes/openai_compat.rs` already preferred
`chunk.raw` verbatim over reconstructing from `delta` — so the single wrong condition was
the entire bug; once a tool-call chunk is no longer dropped, byte-faithful passthrough
already does the rest. Two new tests reproduce the exact wire shape (a tool-call-only
delta, and an argument-fragment-only delta) and assert both are now kept with `tool_calls`
intact in `raw`.

**Found but deliberately not fixed this session, and said so rather than rushing it**: the
native Anthropic surface (`/v1/messages`) has a *deeper* version of the same class of gap.
`anthropic.rs::parse_stream_chunk` only handles `content_block_delta` events shaped as
`{"delta":{"text": "..."}}` — Anthropic's tool-use streaming uses a *different* delta
shape on the same event type (`{"delta":{"type":"input_json_delta","partial_json":"..."}}`)
that this function's `_ => Ok(None)` catch-all silently drops, along with the
`content_block_start` event that announces a tool_use block's `id`/`name` in the first
place. Worse, `routes/anthropic_compat.rs`'s outbound SSE reconstruction doesn't consult
`chunk.raw` at all for this surface — it hardcodes a single `index: 0, type: "text"`
content block for the entire response, so even after fixing the parser, the outbound
stream has no code path for a second, tool_use-typed content block. Properly fixing this
needs real multi-block index/state tracking matching Anthropic's actual event-ordering
semantics (`content_block_start` → `content_block_delta`* → `content_block_stop`, repeated
per block, text and tool_use interleaved) — a materially bigger, higher-regression-risk
change than the OpenAI-side fix, and one wrong index or a missed `content_block_stop`
could produce a stream that confuses the official Anthropic SDK's own accumulation logic
worse than simply not streaming tool calls at all. Flagged as the clear next priority
rather than attempted under time pressure. Non-streaming Anthropic tool-use is unaffected
(`anthropic.rs::parse_response` correctly extracts `tool_use` content blocks).

Verified: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
--all-targets` clean — 796 passing (794 + 2 new). Hit a real-but-mundane snag mid-session:
`cargo test --all-targets` failed with "Access is denied" removing `aegis-gateway.exe` —
the live preview gateway process (started earlier for browser verification) still held the
binary locked. Not a code problem; stopped the preview server, reran clean, restarted it
after. Worth remembering: a locked-binary test failure on Windows during an active preview
session is an environment collision, not a compile error, and the fix is `preview_stop`
first, not debugging the code.

**Same day, next turn — founder said "do all the integrations"**: finished the Anthropic
streaming + tool-use gap flagged above, and along the way found the bug was bigger than
first scoped — a third provider (Google/Gemini) had the identical class of defect
(`functionCall` parts silently dropped in streaming, same root cause: the emptiness check
only ever looked at the plain-text field), and the `chunk.raw` byte-for-byte passthrough
optimization on the OpenAI-compatible surface turned out to be unsafe in general, not just
for tool calls: `state.providers.for_model()` picks a provider purely by which model the
router selected, completely independent of which endpoint (`/v1/chat/completions` vs
`/v1/messages`) the caller used — so an OpenAI-shaped request routed to the Anthropic
adapter (a real, ordinary routing outcome, not an edge case) was forwarding raw
Anthropic-shaped SSE bytes straight to an OpenAI SDK client, and the reverse case existed
too. Fixed properly rather than patched around:

1. **`types.rs`** — added `WireShape` (`OpenAiCompatible | Anthropic | Google`) and
   `ToolCallDelta` (`index`/`id`/`name`/`arguments_fragment`), and gave `StreamChunk` two
   new fields: `source_shape` (which format `raw` is actually written in) and `tool_call`
   (the normalised fragment, when the chunk carries one). `derive(Default)` added to
   `StreamChunk` so the ~8 existing construction sites across `mock.rs`/tests only needed
   `..Default::default()`, not every field enumerated.
2. **All three streaming parsers fixed the same way** — `providers/openai.rs`,
   `providers/anthropic.rs`, `providers/google.rs`: each now extracts a `ToolCallDelta`
   from its provider's own tool-call wire shape (OpenAI's `delta.tool_calls[0]`,
   Anthropic's `content_block_start`/`content_block_delta` with `input_json_delta`,
   Gemini's `functionCall` part) instead of treating a chunk with no *text* as a chunk
   with *nothing*, and each stamps its own `source_shape`.
3. **`routes/openai_compat.rs`** — the streaming loop's raw-passthrough now checks
   `source_shape == OpenAiCompatible` before trusting `raw`, falling back to
   reconstruction otherwise (closing the cross-shape bug above); `to_openai_stream_chunk`
   now renders `delta.tool_calls` from a normalised `ToolCallDelta`, correctly omitting
   `id`/`type`/`function.name` on every fragment after the one that opens a call.
4. **`routes/anthropic_compat.rs`** — the bigger piece. Replaced the old
   single-hardcoded-text-block reconstruction with `BlockTracker`, a small state machine
   (pulled out as its own pure, directly-unit-tested type specifically because this was
   the part of the fix with real room to get subtly wrong) that opens/closes real
   Anthropic content blocks — text and tool_use, correctly interleaved and independently
   indexed — driven by the normalised `delta`/`tool_call` fields rather than raw
   passthrough (which this endpoint never used anyway, so there was no shape-check needed
   here, just the actual multi-block logic). A real bug in the tracker itself was caught
   by its own test, not by review: a tool call's fragments resuming after being
   interrupted by a different block was incorrectly reopening the already-closed original
   block (a protocol-invalid second `content_block_start` for a finalized index) — fixed
   by forgetting a block's source-index mapping the moment it closes, confirmed by a test
   that specifically exercises the interruption-then-resume sequence.
5. **`providers/mock.rs`** — added `MockBehavior::SucceedWithToolCall` (text preamble,
   then a tool call whose arguments arrive across multiple fragments deliberately, so a
   test can confirm fragments land on one block rather than several) so this whole path
   has an in-process way to be exercised without a live provider, in future tests that
   want to drive the full HTTP handler rather than just the pure functions.

Verified: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
--all-targets` clean — 810 passing (796 + 14 new: 3 in `openai.rs`, 3 in `anthropic.rs`, 2
in `google.rs`, 6 in `anthropic_compat.rs`'s `block_tracker` module, 3 for
`to_openai_stream_chunk`'s new tool_calls rendering). Not built: an end-to-end test that
drives the real `/v1/messages` HTTP handler with `MockBehavior::SucceedWithToolCall` and
reads the actual SSE byte stream — `BlockTracker` and the two outbound renderers are each
directly unit-tested, which is what actually caught the real bug above, but nothing yet
exercises the full pipeline (routing → mock provider → streaming loop → HTTP body) in one
test. Reasonable next addition, not attempted this session for time.

### 2026-08-31 — Session 10 — Claude Sonnet 5

Founder supplied a complete design spec ("Deskwork" — dark desk chrome, warm paper content
surfaces, hard zero-blur offset shadows, a strict ink/red/amber/positive/ochre semantic
color contract, three type voices) and asked for it to be integrated into `apps/web`, with
explicit emphasis that §7 of the spec — attribution chips, decision cards, stamps, a
disclosure meter — is "product logic expressed as design" and should be ported wholesale
since Aegis's routing engine is itself agent-like (it makes and narrates decisions on the
caller's behalf).

**What shipped, not just what was asked** — see [docs/design.md](docs/design.md) for the
full token table and component reference, this is the summary:

1. **`app/globals.css`** rewritten in place — every existing `--color-*` token *name* kept
   (so `scripts/check-design-tokens.mjs` needed zero changes and nothing else in the app
   had to be touched to pick up the new palette), only values changed: dark desk
   (`--color-bg`) as the page chrome, warm paper (`--color-surface`/`--color-surface2`) as
   content surfaces, red as the sole event/agent/danger color, a five-stop hard shadow
   ladder, a border-style-carries-meaning grammar (solid/dashed/double/rail), radius capped
   at 11px everywhere except pills.
2. **Fonts swapped** in `app/layout.tsx`: Space Grotesk replaced Plus Jakarta Sans as the UI
   voice; Spectral (serif) added for headings/document voice; Caveat added but deliberately
   *not* used yet anywhere — the dashboard is operational/tabular, not
   document-and-annotation, so there's no honest surface for a handwritten voice today.
   JetBrains Mono kept as-is for the machine voice.
3. **`components/ui.tsx` rewritten** — every shared primitive (`Card`, `Stat`, `Badge`,
   `Button`, `Table*`, `Field`, `EmptyState`, `ErrorState`, `CodeBlock`) re-themed, plus four
   new components that are the actual §7 port: `Stamp` (rotated severity/reason chip,
   border-matches-text-color), `AttributionChip` (you/agent/other — the direct visual home
   for the `X-Aegis-Requested-Model`/`X-Aegis-Served-By` header pair), `DecisionCard`
   (actor→action→timestamp→value→reason with a severity rail, built for narrating one
   routing decision as a stream item), `DisclosureMeter` (solid fill + diagonal hatch for
   "committed vs. not yet settled"). Staged-approval blocks and the MANUAL/APPROVAL/
   AUTONOMOUS mode grammar from the source spec were deliberately **not** ported — those
   model a human+agent co-authoring handoff, which isn't Aegis's interaction shape
   (automatic per-request routing, not turn-taking).
4. **Applied**, not just built: the homepage routing simulator now uses `Stamp`/
   `AttributionChip` for its live pipeline steps and audit receipt; the Requests page uses
   them for routing-reason badges and the requested/served model pair; the Budgets page uses
   `DisclosureMeter` on the anomaly card (today's spend vs. usual baseline — deliberately
   *not* used on the budget-limit rows themselves, since the API returns a limit but no
   current-spend-against-it figure, and faking a percentage against data that isn't there
   would misrepresent what's real). Every layout shell (`(marketing)/layout.tsx`,
   `(auth)/layout.tsx`, `(dashboard)/shell.tsx`) repainted to dark-desk chrome; every page
   file's hardcoded old-palette hex literals (352 across `app/`, 194 in `components/`, zero
   left in either now) replaced with the token set.
5. **One real bug found by live-verifying, not by reading the diff**: several marketing-page
   section headers have no card wrapper — they sit directly on the (now dark) page body —
   and were still using `--color-ink`/`--color-muted` (dark-on-paper colors), which is
   nearly invisible dark-on-dark. Caught via `getComputedStyle` spot-checks after a
   screenshot artifact (see Gotchas) made the bug visually obvious at one scroll depth;
   fixed by routing every bare-on-desk text node through the new `*-on-desk` token trio
   (`--color-paper-on-desk`/`--color-muted-on-desk`/`--color-faint-on-desk`) instead. This
   exact class of bug — and the fix — is written up in docs/design.md as the first thing to
   check if a new bare section is ever added.

Verified: `cargo`-side untouched (frontend-only session); `npx tsc --noEmit` clean, `npx
eslint .` clean, `node scripts/check-design-tokens.mjs` clean (29 tokens, all resolve); live
in the browser preview via `getComputedStyle` spot checks plus full-page screenshots at a
taller emulated viewport (900×1600) covering hero, routing simulator, and login. Did **not**
verify authenticated dashboard pages visually (Settings/Keys/Providers/etc.) beyond the hex-
literal sweep and typecheck — no live session exists without Postgres, same limitation
Session 9 hit. `docs/design.md` written per the founder's own suggestion, as contributor
documentation for the system, not an ADR (this is a visual-identity change, not a deviation
from `MASTER_BUILD.md`'s architecture).

**Correction, same session, right after the above shipped**: the founder then shared the
actual reference screenshot the "Deskwork" spec was inspired by (a whiteboard app called
Boardify) — a light, warm-cream canvas throughout, not a dark chrome anywhere. The "dark
desk" described above was invented, not sourced: the original spec text had scrolled out of
context by the time it was implemented, and the reconstruction guessed a dark outer chrome
that the real reference never had. Fixed by redefining six token *values* in
`app/globals.css` (`--color-bg` and `--color-desk-raised`/`--color-desk-line` to light warm
tones close to the original pre-redesign palette; `--color-paper-on-desk`/
`--color-muted-on-desk`/`--color-faint-on-desk` to dark ink-family values instead of light
ones) plus the body background-image gradient and `layout.tsx`'s `themeColor` — zero
component files touched, because every "text on bare chrome" spot had already been routed
through that token family rather than hardcoded per-file. The four intentionally-dark
terminal/machine-value panels (routing simulator's audit receipt, the API-key display, the
fee-calculation `<pre>` block, `CodeBlock`) were left dark on purpose — a dark code/terminal
accent panel on an otherwise light page is a distinct, legitimate voice, not the mistake.
Re-verified live: hero, login, routing simulator all now read as light paper-on-desk,
matching the reference. Lesson for next time, also in Gotchas: when a design spec scrolls
out of context mid-session, re-read it (or ask for it again) before implementing from
memory — a plausible reconstruction is not the same thing as the source.

### 2026-08-31 — Session 9 — Claude Sonnet 5

User asked for something this project's own tooling has never actually done in any prior
session: run both the real gateway and the real dashboard together, live, and walk the
actual user/org/admin workflows through a browser rather than describe them from source.
Added a `"gateway"` entry to `.claude/launch.json` (only `"web"` existed before) and
started both — `cargo run --bin aegis-gateway` on :8080, `next dev` on :3000, no
Redis/Postgres/Qdrant (Docker still unavailable on this machine).

**Found and fixed a real bug by doing this, not by reading code**: signing up against a
database-less gateway returned a bare `500 internal_error` with no actionable signal,
instead of the `503` every other database-dependent management endpoint already used.
Root cause: `AppState::db()` — the single function every one of those handlers calls —
used `AegisError::Internal` (documented as "anything genuinely unexpected") for a
condition that is neither unexpected nor even undetected — `/health` was already
reporting the identical "database not configured" fact clearly. Added a proper
`AegisError::ServiceUnavailable` variant (503, message passed through rather than
opaqued — `/health` already makes the same fact public, so nothing new is being
disclosed) and switched `AppState::db()` to it, fixing every caller at once by
construction. Two new regression tests (the error mapping itself, and a handler-level
test calling the real `signup()` against `AppState::for_tests()`). Re-verified live in
the browser after rebuilding: `503`, correct message, and the dashboard's signup page —
already built correctly — displayed it inline with zero frontend changes needed. Neither
`route_surface.rs` nor any DB-gated integration test had ever caught this, because CI's
DB-gated tests only run *with* a database present; the "database absent" path had never
actually been exercised by anything until it was exercised by hand.

Published a new artifact, [Aegis Journeys](https://claude.ai/code/artifact/196f024b-ffc7-4955-bfc9-e5cb7c175446)
— the individual/organization/admin workflow walkthrough the founder asked for, with every
step marked by how it was actually checked: live-verified (landing, signup, login, the
unauthenticated-dashboard redirect, CORS between the two real services, `/health`'s honest
dependency reporting) versus code-verified only (every authenticated dashboard page —
Policies, Budgets, Providers, Team, Billing — accurate per source, not clicked through,
since no database means no real session can ever be created here). Confirmed the org
role/policy/budget model already exists roughly as MASTER_BUILD.md describes it — API key
≈ individual actor, team/region/org/platform above it, `can_write()`/`can_read()` enforced
server-side per role, four roles (owner/admin/member/viewer), a structural guard against
ever demoting the last owner.

Live Gemini API key testing requested but not yet received — the founder said "we are
going to use Google Gemini" without pasting the actual key value. Plan once it arrives:
construct an `AppState`/`AuthContext` directly in-process (no HTTP auth needed, the same
way `AppState::for_tests()` already does it) and drive a real completion through the real
pipeline code — genuine, non-mocked verification of the Google provider adapter, pricing
accuracy, and caching, without needing Postgres at all.

### 2026-08-28 — Session 8 — Claude Sonnet 5

User handed over a five-item planning list in one message: a per-request token circuit
breaker, formalizing execution levels (individual vs. org), UI/infrastructure architecture,
IDE integration, and performance/scalability targets. Addressed each on its own footing
rather than treating the list as one undifferentiated task — two were real, scoped
engineering work; two were mostly already-decided reality worth confirming rather than
re-deciding; one is a target-setting exercise with two genuinely open numbers.

**Built the token circuit breaker** — `AEGIS_MAX_TOKENS_PER_REQUEST` (default 16,384),
enforced platform-wide in `routes/openai_compat.rs::execute_with_headroom`, before the
cache fingerprint is even computed. Closes a real gap budget checking didn't:
`middleware::budget::project_cost` only estimates output at ~1/3 of input when a request
sets no `max_tokens`, and the *aggregate* budget only catches an outlier after the fact
(the reservation is trued up post-hoc). A single request — no `max_tokens` set, or a
reasoning model given free rein — could still land as a large, surprising cost before that
happened. Now: no `max_tokens` gets the ceiling injected; a `max_tokens` above the ceiling
gets clamped down; a value already under it is left alone. Clamping is never a rejection
(matches the project's "serve it, bound it" pattern elsewhere) and is surfaced in
`x-aegis-routing-explanation` when it fires, not silent. Unit-tested for all four cases.
Committed and pushed (`70f3a26`).

**Built `apps/vscode-extension/`** — the IDE integration item, deliberately scoped down
from "build a coding assistant" to "make it trivial to point your *existing* one (Continue,
Cline) at Aegis." No chat panel, no in-editor proxy — Aegis already speaks the
OpenAI-compatible protocol every one of those tools expects; the extension's whole job is
holding the API key in VS Code's encrypted `SecretStorage` (never `settings.json`) and
generating the three connection values correctly. Six commands, real TypeScript, strict
tsconfig. Genuinely verified, not just written: `npm install`, `npm run typecheck`,
`npm run compile`, and `npm run lint` all pass clean against the real `@types/vscode`,
`eslint@9`, and `@typescript-eslint` APIs. **Not** run inside an actual VS Code Extension
Development Host — no way to launch VS Code itself here, so the six commands' interactive
behavior is reviewed by hand, not exercised live. JetBrains and Cursor are open — Cursor is
a VS Code fork and likely needs little or no change once verified there; JetBrains is a
genuinely separate platform (Kotlin/Java, its own plugin SDK) — deferred pending the
founder's priority call, not attempted blind.

**A real Windows-specific npm gotcha, worth remembering**: running two `npm install`
invocations against the same `node_modules` concurrently corrupts the loser's packages —
here, `eslint` ended up missing its own `package.json` and bin shim, `ENOTEMPTY`/`EPERM`
errors on Windows file locks during the collision. First background install actually
succeeded on its own (18 minutes — Windows Defender scanning each extracted file in real
time produced a long run of retried `TAR_ENTRY_ERROR` warnings before finishing clean); a
second, impatient foreground attempt against the same directory is what broke it. Fixed by
deleting `node_modules` entirely and running exactly one install, waited out fully before
touching the directory again (30s the second time, no contention). Also found and fixed a
real gap while verifying: no `eslint.config.js` existed at all — ESLint 9 requires the flat
config format and silently can't run without one; `.eslintrc.*` is no longer read.

**Answered directly rather than rebuilding**: UI/infrastructure was already decided (the
Control Room artifact, unchanged); execution levels mostly already exist (API key ≈
individual, team/region/org/platform above it) with one clearly-scoped gap flagged rather
than built blind — no person-level spend aggregation distinct from a key, a real but
additive feature pending the founder's call; performance/scalability got concrete proposed
targets (0.5ms/2ms P50/P99 gateway overhead, 99.9% uptime, the existing ~8-10-replica
Postgres wall) with the two genuinely open numbers (target RPS, whether 99.9% is the right
SLA bar) left for the founder rather than invented.

### 2026-08-27 — Session 7 — Claude Sonnet 5

User asked, in plain language, about the caching tradeoff: store content to cut cost, or
don't store it for security — direct quote of the dilemma: *"Or is there a way out where
you can store the top cash[ed] queries? In an encrypted format in our database and then
retrieve it... I get the best of the world's worlds."* Then, separately: *"we want it to
be as smart as possible and use caching as optimally as possible."* That second sentence
is the exact product decision the semantic-cache wiring had been waiting on since session
3 — flagged, not built, every session until the founder actually weighed in.

**Built the hybrid the founder described, plus wired the semantic cache it was bundled
with — one feature, three cache tiers:**

1. **Hot (Redis, unchanged)** — every cache-eligible response, plaintext, 24h TTL, as
   before.
2. **New: durable (Postgres, encrypted).** `migrations/0004_durable_cache.sql` +
   `cache::durable::DurableCache`. Only a fingerprint the hot tier has *already proven
   repeats* gets promoted — encrypted with the same per-tenant HKDF key BYOK credentials
   already use, sliding 30-day expiry (`AEGIS_DURABLE_CACHE_TTL_DAYS`), purged by a new job
   in `workers::scheduler` on the existing 04:00 UTC window. A one-off prompt never reaches
   this tier at all, which is what makes 30 days safe to keep despite Redis's 24h being
   deliberately short.
3. **Semantic (Qdrant), finally wired.** `cache/semantic.rs` was fully built and tested
   since session 3 and never called from the live pipeline — the exact "built but not
   wired" gap this file has tracked every session since. New `cache::embed::Embedder`
   trait + `ProviderEmbedder` generate the embedding on Aegis's own pooled credential
   (never a customer's BYOK key — see the module doc for why), gated to Pro/Enterprise
   plans per `MASTER_BUILD.md`'s own plan table. A semantic hit promotes its own wording
   into the exact tier too, so a repeat of that specific phrasing skips the embedding call
   next time.

Full design and the honest tradeoffs: `docs/adr/0008-tiered-durable-cache.md` — this is a
genuine deviation from `MASTER_BUILD.md` Part 5's two-tier cache spec, recorded per
CLAUDE.md rule 4, not applied silently.

**Every tier honors `zero_retention` identically** — proven by a test that configures an
embedder guaranteed to produce a hit and confirms a zero-retention org still never gets
one. **Free-tier traffic is structurally excluded** from both new tiers, proven the same
way, not just documented.

12 new tests across `cache/embed.rs`, `cache/durable.rs`, `routes/openai_compat.rs`, and a
new `tests/durable_cache.rs` (6 tests against real Postgres — tenant isolation, the
encrypted round trip, sliding expiry, and the purge job, gated on
`AEGIS_TEST_DATABASE_URL`, confirmed to skip correctly here). `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` clean, `cargo test` → 774 lib passing (was
762) + 814 full-suite passing (was 796), 0 failing, 0 ignored. P3.5 (semantic cache),
open since session 1 and unchecked by the session 3 audit, is checked off for the first
time with the capability it names actually true in production.

**Same session, continued: the founder pasted a detailed external brief targeting a 20ms
cache-hit path** (local ONNX embeddings, Qdrant HNSW tuning, AES-NI, `target-cpu=native`,
Bincode over JSON) and asked to evaluate and integrate it. Corrected the target first —
this project's own stated budget (`routes/openai_compat.rs`'s module doc) is ~1.45ms for
the *entire* gateway, not 20ms just for cache; 20ms would be a regression from what's
already promised, even though it would be a huge win over the current semantic path's
50-300ms remote embedding call. Went through the brief point by point rather than
adopting it wholesale — agreed (local embeddings is the real fix, and `cache::embed::Embedder`
was already the exact seam needed for it), corrected (per-org Qdrant collections, already
in place, give stronger tenant isolation than the brief's single-collection-plus-filter
suggestion; `aes-gcm` already autodetects AES-NI at runtime, no build flag needed for that
specifically), and flagged a real risk the brief treated as free (INT8 quantization can
shift which prompt pairs land above/below the 0.95 similarity threshold — a
false-positive-rate question, not just a speed one, deliberately not attempted this
session).

**Built `cache::onnx_embed::OnnxEmbedder`** — a second `Embedder` implementation (`ort` +
`tokenizers`, FP32, mean-pooled + L2-normalized), behind a new `local-embeddings` Cargo
feature that is **off by default** specifically so enabling it never makes `cargo build`
silently fetch a native binary — see `docs/adr/0009-local-onnx-embeddings.md` and
`docs/runbooks/local-embeddings-setup.md`.

**Then asked directly whether an NVIDIA open embedding model made sense here, and for a
comparison against the alternatives.** Researched rather than assumed (NVIDIA's open
embedding line moves fast — Nemotron 3 Embed shipped mid-July 2026, after this session's
training-knowledge cutoff for that specific release). Verdict: no — even NVIDIA's smallest
open, commercially-licensed variant (Nemotron 3 Embed 1B, OpenMDW-1.1) is 1.14B parameters,
decoder-based, and NVFP4-quantized for Blackwell-class GPUs, roughly 50x too large and the
wrong architecture family for a sub-5ms CPU cache lookup — ruled out on fit, not license.
Within the actual CPU-sized tier, **switched the target model from the originally-proposed
`all-MiniLM-L6-v2` to `BAAI/bge-small-en-v1.5`** (33M params, 384-dim, MIT) — multiple
current sources rank it above MiniLM on retrieval quality at nearly identical size/speed,
worth taking for a mechanism whose real risk is a false-positive cache hit. Two BGE-specific
correctness details, verified against the model's own docs rather than assumed: it mean-pools
by default (matches the code already written, no change needed — CLS pooling would have
produced an incompatible embedding space), and needs no query-instruction prefix for a
symmetric prompt-to-prompt comparison (that prefix is BGE's own recommendation for
asymmetric query-vs-document retrieval, a different job). Also found and generalized a
sharper point while researching this: BGE's own docs note unrelated-text similarity in its
embedding space sits noticeably above zero, which sharpens (not just repeats) the ADR's
existing warning that the 0.95 threshold's meaning isn't automatically portable across a
model change, quantization included.

Refactored `MAX_SEQUENCE_LENGTH` from a hardcoded MiniLM-specific constant into a real
constructor parameter (`OnnxEmbedder::load`'s new `max_sequence_length` argument, with
`BGE_SMALL_MAX_SEQUENCE_LENGTH = 512` as the named constant for this model) — silently
reusing one model's trained context length for a different model is exactly the kind of
quiet mismatch worth designing out rather than leaving as a trap for whoever swaps models
next. Re-verified after the swap: `cargo check`/`cargo clippy --lib`/`cargo clippy
--benches`, all with `--features local-embeddings`, still clean; default build still 774
lib / 814 full-suite, unaffected.

**Honest status, stated as precisely as the verification allows — this is categorically
different from everything else in this file:**
- `cargo check --features local-embeddings` **succeeds** — type-checks against the real
  `ort` v2.0.0-rc.10 and `tokenizers` v0.23 APIs (caught and fixed one real borrow-checker
  bug: the original draft dropped the session's `MutexGuard` before reading its output,
  which doesn't compile). `cargo clippy --features local-embeddings --lib` is clean too.
- `cargo build`/`cargo test` **with that feature fail to link**, confirmed directly rather
  than assumed: `ort-sys` emits a self-explanatory placeholder linker input when no ONNX
  Runtime binary is configured, and the linker fails loudly on it. This means **even the
  pure mean-pooling unit tests (hand-built vectors, no model needed) have never actually
  run** — Rust links whole test binaries, so the crate needs the native library resolvable
  regardless of which test is selected. Reviewed by hand; not run.
- **The default build is completely unaffected**, confirmed by re-running it after every
  step above: `cargo test` with no feature flags still shows 774 lib / 814 full-suite,
  identical to before this work started.
- Not attempted: wiring `OnnxEmbedder` into `AppState` anywhere real. The runbook is
  explicit that verification (including a false-positive-rate check against the 0.95
  threshold with real embeddings) comes before that, not after.

A criterion benchmark (`benches/semantic_embedding.rs`, `local-embeddings`-gated) exists
for p50/p95/p99 latency once a real model is available — also unrun here for the same
reason.

**Then asked to check which of four technologies were actually in use (`aes-gcm`,
`qdrant-client`, gRPC, `ort`) — answered by grepping the real repo rather than recalling
from memory: `aes-gcm` genuinely load-bearing since early sessions; `ort` added but
unverified (above); `qdrant-client`/gRPC not present at all, deliberately deferred earlier
this session in favor of the embedding fix.** Asked directly whether to pick gRPC back up
now — yes: `VectorStore` was already a trait, `qdrant-client` is pure Rust with no
native-binary problem, and now that the embedding step is fast, Qdrant's own REST/JSON
round trip is a proportionally bigger slice of what's left.

**Built `cache::qdrant_grpc::QdrantGrpcVectorStore`** — a second `VectorStore`
implementation alongside the existing REST-based one, chosen in `main.rs` by a new
`QDRANT_GRPC_URL` setting (deliberately separate from `QDRANT_URL`, not derived by
swapping the port — Qdrant Cloud and some self-hosted setups front REST and gRPC on
different hosts). REST kept, not deleted — it's the fallback when gRPC isn't configured.
Full reasoning: `docs/adr/0010-qdrant-grpc-client.md`.

**Fully verified this time — a stronger claim than the ONNX work could make, and worth
naming why:** `qdrant-client` has no native binary to provision, so unlike `ort`,
`cargo check`, `cargo clippy --all-targets -- -D warnings`, and the full `cargo test`
suite all actually ran with this dependency compiled in by default (not feature-gated).
Every API call in the new module matched the real `qdrant-client` v1.19.0 surface on the
first attempt. Default build: still 774 lib / 814 full-suite passing. What's *not*
verified: whether it actually works against a live Qdrant server — no Docker here, same
gap as everything else in this file.

**Caught a real `cargo audit` regression before it shipped.** `local-embeddings`'s
`tokenizers` dependency transitively pulls in the now-unmaintained `paste` crate
(RUSTSEC-2024-0436) — same lockfile-artifact shape session 6 found for `sqlx-mysql`/`rsa`
(confirmed: `cargo tree -i paste` finds nothing with default features, only appears under
`--all-features` via `tokenizers`). Would have broken CI's `cargo audit --deny warnings`
gate. Closed with a scoped, evidenced ignore in `.cargo/audit.toml`, matching the existing
entry's format; re-ran the exact CI command from the repo root afterward — clean.

**Also fixed something the ADR draft got wrong before it shipped**: the ADR initially
claimed the bundled self-hosted Qdrant container already exposed gRPC on 6334. It didn't —
checked `infra/docker-compose.yml` and `infra/docker-compose.self-hosted.yml` directly,
found only port 6333 (REST) was exposed in either, and added 6334 to both rather than
leave documentation describing infrastructure that didn't match reality.

**Then asked for five things in one message: a token circuit breaker, formalizing
execution levels (user vs org), UI/infrastructure decisions, IDE integration, and
10/10 performance/scalability targets.** Treated as a backlog to work through with
appropriate depth per item, not five things to build blind in one pass — checked what
already existed against the real code before proposing anything new for each.

**Built the token circuit breaker — the one item that was a clear, well-scoped gap.**
Budget checking (`middleware::budget`) bounds *aggregate* spend over a period; nothing
bounded a single request's `max_tokens` independent of that. A prompt with no
`max_tokens` (the more dangerous case, not the safer one — defers to the provider's own
default, which can be very large) or one requesting far more than the budget projection
assumed could still land as a large, surprising outlier before the aggregate counter
caught up. New `Config::max_tokens_per_request` (`AEGIS_MAX_TOKENS_PER_REQUEST`, default
16,384) enforced by a shared `clamp_max_tokens()` — clamps down, or injects the ceiling
when absent, unconditionally.

**Found and closed a real gap while implementing it**: the natural first instinct
(enforce inside `execute_with_headroom`, the shared pipeline function) would have missed
streaming entirely — `stream_chat` runs a completely separate pipeline that never calls
`execute_with_headroom`. Moved the authoritative enforcement to `handle_chat` and
`handle_messages` (the two HTTP-level entry points), before their budget reservation
call — covers streaming and non-streaming from one call site per endpoint, and fixes a
second, subtler issue for free: the budget *projection* itself now reflects the bound
that will actually be enforced, rather than projecting against a value the request will
never actually be allowed to reach. Proved directly, not assumed: extended
`MockProvider::RecordedCall` to capture the `max_tokens` a provider actually received, then
wrote a streaming-specific test (`the_token_circuit_breaker_also_protects_streaming_requests`)
that would have caught the gap had it shipped. 8 new tests, `cargo fmt`/`clippy --all-targets
-- -D warnings` clean, `cargo test` → 782 lib passing (was 774) + 822 full-suite (was 814).

**The other four items got direct answers, not code** — see the chat response for the
full treatment: Execution Levels and UI/Infrastructure were mostly already true, answered
by pointing at what's real (the 4 budget scopes, the 3 deployment paths, the [Control
Room](https://claude.ai/code/artifact/3e1c0060-2d00-4995-bbf6-3e113dcf6e87) artifact);
Performance/Scalability got concrete numeric targets grounded in what's already measured
rather than an unqualified "10/10"; IDE Integration is a genuinely new, large,
different-tech-stack commitment (TypeScript for VS Code/Cursor, Kotlin for JetBrains) —
scoped as a proposal, not started, pending which surface the founder actually wants.

### 2026-08-26 — Session 6 — Claude Sonnet 5

User's request, quoted in full because it set the scope for the entire session: *"The
brutal feedback that you have given me. I get it, and I want you to improve all the
metrics and get all the metrics to minimum of seven to eight, if not higher. Work on
everything that you said is not implemented and should make the product stand out the
most. and work on all of its lacunas and then give me a walkthrough at the end after you
work on all of the scope of improvements aspects...and complete it fully."* Followed, after
a PR-creation attempt was blocked by an invalid `gh` auth token, by: *"work on all the
things that remaining in development."*

**Closed nearly every open P0/P1/P2 finding from the session 5 audit, plus one severe bug
the audit missed entirely — eleven commits, all real code and real tests, all pushed
directly to `origin/main`** (the `gh pr create` flow was attempted first and failed on an
invalid token; direct-to-`main` pushes are the established pattern from prior sessions and
the user did not object):

1. **`33d9e62` — Atomic budget reservation, real fallback chains, streaming resilience
   [P2.2, P3.7]**, the two P0s the audit called highest-priority. `check_and_reserve`
   atomically reserves a projected cost against every applicable counter before admitting;
   `alternates_for()` populates real cross-provider fallback candidates where the call site
   used to hardcode `&[]`; streaming gained the same retry/fallback/health-recording path
   non-streaming already had; an outer request deadline was added. Caught and fixed a
   double-counting bug in the reservation design's own first draft before it shipped (see
   Gotchas).
2. **`a460082` — Cached-token and long-context pricing [P2.6]**, the last P0.
   `TokenUsage` gained `cached_input_tokens`/`cache_write_tokens`; each provider's
   `parse_usage()` normalizes that provider's own cache-token convention; `ModelPricing`
   gained real cache rates and a `LongContextTier` so Gemini 2.5 Pro's 200K-token tier is
   modeled instead of flattened.
3. **`a286381` — Graded provider health, outcome-informed routing, and a severe
   embedding-routing bug** found mid-session, not in the original audit:
   `Requirements::default()` had no `chat` field, so an embedding model could legitimately
   win a plain chat request on price alone. Fixed with `chat: bool` defaulting to `true`.
   Also replaced the binary circuit-breaker health signal with a graded `HealthScore`
   (rolling success rate + EWMA latency) that now feeds routing selection directly, and
   wired the previously write-only outcome bandit back into the router's own scoring.
4. **`7615b8e` — SSO callback and two compounding routing bugs [P1-03]**. OIDC discovery,
   JWKS fetch/verify, and the callback route (`/api/auth/sso/callback`) that was never
   registered are all in place now. SAML gets an explicit "not yet supported" error.
5. **`87a3034` — TOTP enforced end to end**. Repo layer, enroll/confirm/disable endpoints,
   a new per-user HKDF key namespace, and a real login-time check.
6. **`7615b8e`/`d9cb887` — Idempotency keys sent to every provider on retry [P1-04]**,
   generated once per fallback-chain attempt, reused only within that attempt's own
   retries.
7. **`c1a9ad1` — Vertex secret redaction and session purge [P2-07, P2-08]**. Vertex
   service-account keys now match the redaction patterns; expired sessions now actually get
   purged on a scheduled window.
8. **`7380c87` — Self-service SCIM tokens, three real `cargo audit` findings closed
   [P2-09]**. `/api/scim-tokens` added. Removed an `rsa` dev-dependency (replaced with
   static PEM fixtures — `jsonwebtoken` uses `ring` internally, not `rsa`), upgraded `lru`
   past a genuine unsoundness advisory, added one scoped documented ignore in
   `.cargo/audit.toml` for a phantom `sqlx-mysql`-via-inactive-feature lockfile entry.
9. **`ba94a5e` — Every atomicity claim proven against real Redis, not just `MemoryStore`
   [P1-02]**. New `tests/redis_concurrency.rs`, 4 tests, gated on `AEGIS_TEST_REDIS_URL`.
10. **`d178444` — A minimum-viable alerting pipeline from nothing [P1-05]**.
    `infra/prometheus/alerts.yml`, 8 rules across 4 groups.
11. **`03812be` — Admin audit log correctly org-scoped, customer audit export added
    [P2-10]**. `/api/admin/audit` was always scoped to the calling admin's own org — nearly
    useless for a staff member investigating a customer, since staff accounts are rarely
    members of the org they're helping. Added an `org_id` query parameter. Separately found
    no customer-facing audit export existed at all despite the compliance whitepaper
    describing one; added `GET /api/audit-log.jsonl`.

**Investigated and deliberately left alone, with reasons recorded rather than silently
skipped**: API-key read-scope narrowing (re-reading `MASTER_BUILD.md` confirmed org-scoped
management-API access via API key is the documented design, not a defect — narrowing it is
a product decision needing its own ADR); the semantic cache (still an embedding-latency
tradeoff, not a bug); SAML (explicit scope decision, not an oversight); live-provider and
live-infrastructure verification (same Docker/no-keys environment constraint as every prior
session).

**Housekeeping**: deleted the stale `fix/enterprise-audit-remediation` branch was
attempted but blocked by the permission classifier (0 unique commits vs. `main` — safe
whenever push access allows it); `gh auth` confirmed still broken
(`gh auth status` → "The token in default is invalid"), reported rather than worked around.

Verification run this session, on this machine: `cargo fmt --check` clean, `cargo clippy
--all-targets -- -D warnings` clean throughout, `cargo test` → 762 lib passing (was 708) +
796 full-suite passing (was 730), 0 failing, 0 ignored (the session-5 budget-race
`#[ignore]`d bypass proof was replaced by a passing proof-of-fix). All eleven commits pushed
to `origin/main`. Docker still unavailable this session (unconfirmed status carried since
session 3); no provider keys supplied.

### 2026-08-25/26 — Session 5 — Claude Opus 5 / Sonnet 5

User's request, quoted because the framing matters: *"Act as a brutally honest senior AI
infrastructure architect, SRE, security engineer, and enterprise SaaS reviewer. Thoroughly
audit this project as if we are preparing to sell it to large enterprise customers... Do
not assume that something works just because it exists in the codebase... trace the actual
execution paths, run tests where possible... Attempt to bypass these controls."* A 16-section
spec followed (pricing/metering, Vertex AI integration, Redis retention, RBAC, observability,
scalability, fallback/reliability, smart routing, budgets/rate-limiting, performance,
enterprise deployment, testing, security, a 0-10 scorecard, categorized findings, final
verdict).

**Built Vertex AI as a tenth provider first** (`providers/vertex.rs`) — service-account
JSON → self-signed RS256 JWT → OAuth2 exchange, cached per replica; delegates request/
response handling to the existing `google::` functions rather than duplicating them.
Required overriding `chat()`/`chat_stream()` entirely rather than extending the shared
`Provider` trait, since real auth is async and every other provider's auth is a
synchronous bearer token — the central design decision, recorded in
`docs/adr/0007-vertex-ai-jwt-signing.md`. 14 new tests. Added Vertex pricing rows (2
independently confirmed, 2 marked `UNVERIFIED`). Added a Vertex option to the dashboard's
provider-connection page (multi-line JSON textarea instead of a single-line key field).

**Then ran the audit** — direct code tracing plus four parallel focused investigations
(observability/Redis, RBAC/security, fallback/routing, scalability/testing), each required
to cite file:line evidence rather than assert conclusions, cross-checked against each
other and against hands-on adversarial testing. Full report:
**[Aegis Enterprise Readiness Audit](https://claude.ai/code/artifact/aa6e48d0-50e5-445f-a8c0-70417960003e)**
— scorecard, 16 sections, and a P0-P3 findings list with Problem → Evidence → Business
impact → Technical impact → Fix → Complexity for each.

**Three P0s were serious enough to fix during the audit itself:**

1. **SSRF, zero-privilege-signup to cloud metadata.** No provider adapter validated a
   BYOK `base_url` before honoring it as a literal override. Full chain: free signup (no
   review gate) → `POST /api/providers` with `base_url` pointed at
   `169.254.169.254` → `POST /api/providers/{id}/test` makes the gateway itself issue the
   request. Fixed with `middleware/ssrf_guard.rs` (resolve-then-classify against loopback/
   RFC1918/link-local/CGN/IPv6-unique-local/IPv4-mapped-IPv6), wired into
   `create_provider`. New integration test drives the real handler against a real database
   and proves the chain is now rejected and nothing is persisted, plus that a legitimate
   endpoint still works. Documented residual risk: validation-time, not connect-time, so
   classic DNS rebinding isn't fully closed — stated in the module's own doc comment.
2. **The metering-completeness metric couldn't detect metering incompleteness.**
   `usage::emit`'s `Result` was discarded at all three call sites in
   `routes/openai_compat.rs`; the Prometheus counter meant to catch "served but never
   billed" incremented regardless of whether the write actually succeeded. Fixed to gate
   the metric on genuine success and log failures with org/request context; mid-stream
   provider failures now carry a real `error_type` instead of looking like a clean 200.
   Independently corroborated: one of the four parallel investigations found the identical
   bug at the identical call sites before either knew of the other's finding.
3. **The compliance whitepaper's content-storage claim didn't match the code.**
   `docs/compliance/security-whitepaper.md` described `content_capture` as an opt-in,
   encrypted control. It's dead code — `zero_retention` is the only flag any code path
   reads, and the exact-match cache stores plaintext prompts/completions in Redis for 24h
   by default for every org that hasn't set it. Corrected in the document, with the
   correction left visible rather than silently edited away.

**Proved, did not fix — the budget-check race.** `budget::check()` reads spend and
compares to the limit; nothing reserves it atomically. First attempt to reproduce under
the default single-threaded test runtime did not trigger it (a methodologically important
near-miss — see Gotchas). Rebuilt with genuine multi-thread parallelism and a `Barrier`
forcing 20 simultaneous requests against a $1.00 hard limit with $0.05 headroom: **8 of 8
runs bypassed the limit**, admitting 2-5 requests (15-45% overshoot) where at most 1
should ever get through. Committed as `#[ignore]`d so `cargo test` stays green while the
proof stays runnable on demand. Not fixed: the honest fix is a reserve-then-true-up
redesign, not a bounded patch, and rushing it under audit time pressure would have traded
one unverified claim ("it's atomic") for another ("the redesign is correct").

**Found, not fixed, and now tracked** (full evidence in the artifact and in Known
Limitations above): the fallback chain's only production call site hardcodes zero
alternates, collapsing resilience to one attempt for exactly the highest-value traffic;
streaming has no retry/fallback and never updates circuit-breaker health; cached-token
pricing doesn't exist for Anthropic or OpenAI and Gemini 2.5 Pro's 200K-token tier isn't
modeled; the SSO callback route is never registered so login cannot complete;
`workers::reconciliation::run` and the budget-alert worker are never spawned; Redis's
atomic guarantees have never been tested against real Redis (only `MemoryStore`); no
distributed tracing, no alerting pipeline, no idempotency keys sent to providers; API keys
can read broad org-wide management data; Postgres connection math caps the fleet at
roughly 8-10 replicas.

Also fixed while merging in two pre-existing bugs surfaced by the two collaborator-added
Gemini models from session 4 landing in the pricing-drift tests: a substring-prefix
collision (`openai/gpt-4o` matching `openai/gpt-4o-mini`) in
`workers/scheduler.rs`'s drift-report matching, and the hardcoded provider/model counts in
two tests that needed to move from 9→10 providers and 7→9 unverified rows.

Docker still unavailable this session (status unconfirmed since session 3, not re-checked).
No provider keys supplied (Gemini or Vertex). Verification run this session, on this
machine: `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean,
`cargo test` → 708 lib passing + 730 full-suite passing, 0 failing, 1 intentionally
ignored; `npm run lint` and `npm run build` clean (23 static routes). Pushed both commits
(`d86cea9` Vertex AI, `453641b` the three fixes + the budget-race proof) to `origin/main`.

### 2026-08-24 — Session 4 — Claude Opus 5

User pushed the repo to GitHub (`kunalshinde1214/Aegis`, private) and asked me to pull
a collaborator's changes. One commit from Neel Shah (`bc3bacd`): added
`google/gemini-3.6-flash` and `google/gemini-3.1-pro-preview` to the pricing table
(sourced, dated 2026-08-24) and to the Google provider adapter's accepted-model list,
plus regenerated `package-lock.json`. Fast-forward pull, no conflicts.

**Found two real bugs while verifying the merge, neither the collaborator's fault:**

1. **A latent, order-dependent test bug in my own code from session 3.**
   `workers/scheduler.rs`'s pricing-drift tests picked "the first model" from
   `PricingTable::all()` (HashMap iteration — order is randomised per process in Rust)
   and matched drift-report lines against it with `line.starts_with(&model.model_id)`.
   That breaks whenever the picked model has a prefix-colliding sibling in the table
   (`openai/gpt-4o` vs `openai/gpt-4o-mini`, `gemini-2.5-flash` vs
   `gemini-2.5-flash-lite`, etc.) — `starts_with` doesn't stop at a word boundary. Adding
   two more models shifted which entry landed first often enough to finally hit an
   unlucky pairing and fail. Fixed by anchoring the match on `"{model_id}:"` (the colon
   the report format always appends) instead of the bare id. Verified stable across 13
   separate process runs (8 isolated + 5 full-suite), since each gets a fresh random
   hash seed.
2. **The collaborator's new adapter entries had no pricing.** `providers/google.rs`
   already listed `gemini-1.5-flash`/`gemini-1.5-pro` as accepted models with zero
   corresponding pricing rows. Every cost lookup in the pipeline falls back to
   `MicroCents::ZERO` on a pricing miss — so a real request served by either model would
   have been metered at exactly $0, understating a customer's baseline if requested and
   silently under-counting real spend against their budget if ever served. Added both as
   *retired* entries (Google has moved traffic to 2.x/3.x; these exist so a request can
   still be priced, not so the router selects them), marked `UNVERIFIED` rather than a
   remembered number.

Also noticed, flagged, not fixed: a fresh `npm ci` on the merged lockfile surfaces 3
pre-existing high-severity transitive vulnerabilities (`postcss`, `sharp`, both pulled in
by Next.js 15's own dependency tree) — `npm audit fix --force` would resolve them but
requires upgrading to Next 16, a major-version bump outside `package.json`'s current
`^15.1.0` range and a real breaking-change risk, not something to apply blind. Unrelated
to the collaborator's commit; CI's `npm audit --audit-level=critical` gate is set to
`critical` and would not have caught `high`-severity findings, which is why this went
unnoticed until now.

682 tests passing, clippy clean, web typecheck/lint/build clean. Pushed the fixes back to
`origin/main` on top of the merge.

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

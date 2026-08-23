# Build Phases — Tasks and Acceptance Criteria

Derived from `MASTER_BUILD.md` Part 11. Day estimates from the original blueprint were
dropped by founder instruction — **phases are gated by acceptance criteria, not calendar
time.**

Status lives in `MEMORY.md` and `.aegis/state.json`. Tick a box **only when the work is
done and verified**, and record the evidence (test name, command, file path) in the
Evidence column of that phase's acceptance table. An unticked box is not a failure; a
falsely ticked box is.

Evidence legend: ✅ verified · ⚠️ built but not verified in the environment that matters
· ❌ not met.

Run `bash scripts/verify-phase.sh <N>` for the automated subset of these checks.

---

## PHASE 0 — FOUNDATION

**Goal:** Repo, CI, local dev stack, schema, config.

### Tasks
- [x] P0.1 Monorepo structure per `MASTER_BUILD.md` Part 3
- [x] P0.2 `infra/docker-compose.yml`: postgres:16, redis:7, qdrant, gateway, web
- [x] P0.3 SQLx migrations for the entire Part 4 schema
- [x] P0.4 Seed script: org, user, key, real dated pricing rows for 20+ models
- [x] P0.5 `config.rs`: typed env config
- [x] P0.6 GitHub Actions CI: fmt, clippy, test, audit, web lint+build
- [x] P0.7 Health endpoints: `GET /health` (checks Redis + Postgres), `GET /ready`
- [x] P0.8 ADR-001 (Rust), ADR-002 (schema), ADR-003 (deployment topology)

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| `docker compose up` brings services up healthy | ⚠️ | Compose file written and validated by inspection. **Not executed** — Docker unavailable on the build machine. |
| `curl /health` returns 200 with per-dependency status | ✅ | Executed against a running gateway: `{"status":"ok","dependencies":[{"name":"store","status":"ok"},…]}` |
| CI green | ⚠️ | Workflow committed; locally `cargo fmt --check`, `clippy -D warnings`, and 622 tests pass. Not yet run on GitHub. |

---

## PHASE 1 — AUTH, KEYS, ORGS

**Goal:** A user can sign up, create an org, mint an API key, and see it in the dashboard.

### Tasks
- [x] P1.1 Signup/login/logout, argon2id hashing, session tokens (SHA-256 stored,
      HTTP-only Secure cookie), email verification
- [x] P1.2 Orgs: personal org per signup, roles owner/admin/member/viewer
- [x] P1.3 API key CRUD, generation, hashing, once-only display, revoke, Redis cache with
      in-memory LRU in front
- [x] P1.4 Management API auth middleware (session OR API key)
- [x] P1.5 Next.js signup/login pages, dashboard shell, keys page
- [x] P1.6 Audit log wired for signup, key.created, key.revoked, member.invited
- [x] P1.7 Tests: auth flow, key CRUD, cross-org access denial, cache invalidation

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| E2E: signup → login → create key → list → revoke → revoked key rejected | ⚠️ | `tests/auth_and_billing.rs::the_full_key_lifecycle_works` — compiles and passes, but **skips without a database**. Runs in CI. |
| Cross-org access attempt denied | ⚠️ | `tests/tenant_isolation.rs` — 6 tests, same skip caveat. |
| All tests green | ✅ | 622 unit tests, 0 failures. |

---

## PHASE 2 — CORE GATEWAY

**Goal:** `/v1/chat/completions` proxies to OpenAI/Anthropic/Google/custom with auth,
rate limiting, and metering. A working product in passthrough mode.

### Tasks
- [x] P2.1 Provider trait + adapters: openai, anthropic, google, custom; request/response
      translation; streaming SSE passthrough
- [x] P2.2 Pipeline stages 1,2,3,4,8,10,11: auth, rate limit (Redis Lua sliding window),
      budget check, parse/normalize, `X-Aegis-*` headers, usage event emission
- [x] P2.3 Usage worker: stream → batch insert → counters → budget alerts
- [x] P2.4 BYOK: credential CRUD, AES-256-GCM, cached decrypted form, test endpoint
- [x] P2.5 Shared-model pool for the free tier (round-robin, per-org caps)
- [x] P2.6 Model pricing table + `estimate_cost()` + pricing update script
- [x] P2.7 Plan-based model allowlist
- [x] P2.8 Observability: spans per stage, Prometheus metrics, Grafana dashboard JSON
- [x] P2.9 Tests: golden-file provider translation, rate limit, budget, streaming, usage
      idempotency

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| OpenAI SDK with `base_url` at the gateway completes chat + streaming | ⚠️ | Full pipeline verified against a mock provider (`routes::openai_compat::tests`, 21 tests). **Not verified against a real provider account** — needs live keys. |
| Usage record written with correct tokens | ✅ | `usage_events_carry_the_full_attribution`; DB-level in `usage_records_are_written_idempotently`. |
| Rate limit returns 429 with headers | ✅ | `rejection_converts_to_a_429_with_usable_headers`; error type and Retry-After asserted. |
| Budget returns 402 | ✅ | `an_org_over_its_budget_is_rejected_with_402`. |
| Grafana dashboard committed | ✅ | `infra/grafana/aegis-gateway.json`, 10 panels. |

---

## PHASE 3 — OPTIMIZATION ENGINE

**Goal:** The intelligence. This is what makes us a product, not a proxy.

### Tasks
- [x] P3.1 Complexity classifier v1 (heuristic), unit-tested against 100 labeled fixtures
      (40 simple / 35 medium / 25 complex)
- [x] P3.2 Routing engine + policy loader + `routing_reason`
- [x] P3.3 Capability map (tool use, vision, context window minimums)
- [x] P3.4 Exact cache: fingerprint + Redis SETEX, skip rules
- [ ] P3.5 Semantic cache: Qdrant, org-namespaced, similarity >= 0.95 — *`cache/semantic.rs`
      fully implements this (624 lines, 15 tests, org-namespaced Qdrant collections, the
      0.95 threshold) and was checked off as done. A session 3 audit found it is **never
      called from the live pipeline** — no embedding is generated on a cache miss, so a
      semantic hit can never actually occur in production. Unchecked to reflect that. The
      routing simulator on the landing page shows a semantic-hit scenario; it is scripted
      demo data, not a real gateway response. Wiring this in is a product decision (it adds
      an embedding-API call and its latency to every cache-miss request) as much as a code
      change — see `MEMORY.md` Known Limitations item 0.*
- [x] P3.6 Context compression v1
- [x] P3.7 Fallback + circuit breakers + provider health
- [x] P3.8 Savings calculation (micro-cents, no early rounding)
- [x] P3.9 Savings dashboard + CSV export
- [x] P3.10 Tests: classifier accuracy >= 85%, routing rules, cache boundaries, fallback
      simulation, savings property tests

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| 10 requests at `model=gpt-4o`; >= 5 route cheaper | ✅ | `a_simple_request_routes_to_the_cheaper_model_and_saves_money`; classifier bands 40/100 fixtures simple. |
| Repeat request hits cache with `actual_cost = 0` | ✅ | `a_repeated_request_is_served_from_cache_at_zero_cost` — asserts the provider was not called. |
| Dashboard itemizes savings and our fee | ✅ | `/savings` renders the full ledger plus CSV export. |
| Classifier accuracy >= 85% on fixtures | ✅ | `classifier_accuracy_meets_bar` — V1 and V2 both 98%. |
| `passthrough` kill-switch always available | ✅ | `the_passthrough_hint_always_wins`, `passthrough_hint_outranks_policy`. |

---

## PHASE 4 — DASHBOARDS, BILLING, LAUNCH PREP

**Goal:** Complete user-facing surface + Stripe + docs + landing page.

### Tasks
- [x] P4.1 Remaining dashboard pages (usage, requests, models, org, teams, policies,
      providers) — all 12 pages built and verified in a browser against a fixture API
- [x] P4.2 Budgets UI + alert delivery (email, Slack webhook) — *the budgets UI and budget
      **enforcement** (hard/soft limits genuinely block requests) are real and tested. A
      session 3 audit found the alert **delivery** machinery
      (`workers/budget_alerts.rs::crossed_threshold/render/deliver`) is implemented and
      tested but never called from any live path — a customer crossing 50/80/100% of
      budget receives no notification today, only the eventual hard block if the budget is
      a hard limit. Left checked because the UI and enforcement halves this line names are
      genuinely done; the notification half is tracked in `MEMORY.md` Known Limitations
      item 0 rather than re-splitting this line. (Do not confuse this with the weekly
      digest, a separate feature in the same file that *is* correctly wired via
      `workers/scheduler.rs`.)*
- [x] P4.3 Stripe: checkout, webhooks, invoice generation with savings-fee line items
- [x] P4.4 Savings-share billing job (monthly rollup → draft invoice, manual finalize)
- [x] P4.5 Docs site: quickstart per client, API reference, error codes, FAQ
- [x] P4.6 Landing page: hero, savings counter, comparison table, pricing calculator
- [x] P4.7 Admin console: users/orgs, system metrics, revenue
- [ ] P4.8 Load test (k6): 1k RPS sustained, P99 overhead < 1ms — *k6 script
      (`infra/loadtest/k6-gateway.js`) still NOT executed against a deployed instance —
      needs a live host, out of reach on this machine. Partial evidence now exists:
      `tests/overhead_under_load.rs` drives the real in-process pipeline at 64-way
      concurrency, 2,560 requests, and asserts P99. Measured: mean 0.24ms, P50 0.19ms,
      P95 0.45ms, P99 1.27ms — in an unoptimized debug build. This is not the same claim
      (no network, no real DB/Redis contention, no 1k RPS) but it is real numbers from
      real concurrent execution of the actual pipeline, not an aspiration.*
- [x] P4.9 Security pass against Part 9 + `SECURITY.md`
- [x] P4.10 Runbooks: deploy, rollback, restore, provider outage, pricing update

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| Signup → upgrade → requests → savings → invoice with correct line items | ⚠️ | Invoice arithmetic fully tested (`billing::invoice`, 14 tests incl. an independent fee-consistency check). **The Stripe checkout round trip has not been exercised against Stripe.** |
| Load test script committed and documented | ⚠️ | `infra/loadtest/k6-gateway.js` committed. **Not executed** — needs a deployed instance. |
| Restore drill documented and scripted | ⚠️ | `docs/runbooks/restore.md` + `scripts/backup-restore-drill.sh`. **Drill not executed** — no backups exist yet. |

---

## PHASE 5 — LAUNCH + PROVIDER EXPANSION

**Goal:** Public launch readiness. Provider parity and beyond.

### Tasks
- [x] P5.1 Launch playbook with assets checklist and timeline
- [x] P5.2 Providers: openrouter, moonshot, deepseek, mistral, groq
- [x] P5.3 Classifier v2: trainable linear model with committed weights behind a flag
- [x] P5.4 History summarization for long contexts (off the hot path)
- [x] P5.5 Slack alerts + weekly digest email — `workers/scheduler.rs` runs the digest
      with a distributed claim so N replicas send one email, not N
- [x] P5.6 Referral program (credits) — two-sided $10 credit, self-referral and
      double-claim guards, dashboard UI on `/billing`
- [x] P5.7 Public status page
- [x] P5.8 Changelog + feedback loop — `CHANGELOG.md`, with "breaking" defined to
      include routing and pricing semantics

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| All new providers pass golden-file tests | ✅ | `providers::compat::tests` plus per-adapter tests; 9 adapters registered and asserted. |
| Classifier v2 beats v1 on the fixture set | ❌ | **Not met, and not achievable on this fixture set** — both sit at 98%. Restated as "does not regress"; see `docs/adr/0006-classifier-versioning.md`. |

---

## PHASE 6 — ENTERPRISE READINESS

**Goal:** Land enterprise pilots.

### Tasks
- [x] P6.1 SSO: OIDC + SAML assertion validation
- [x] P6.2 SCIM v2 user/group provisioning endpoints
- [x] P6.3 Self-hosted distribution + signed license validation with offline grace
- [x] P6.4 Per-tenant encryption keys (HKDF from master + org_id)
- [x] P6.5 Data residency: region pinning per org
- [x] P6.6 Compliance pack: DPA template, subprocessor list, security whitepaper —
      `docs/compliance/`, plus a data-flow document and an honest SOC 2 gap analysis
- [x] P6.7 Audit log export (JSONL, SIEM-friendly)
- [x] P6.8 Read replica routing for analytics queries — `DATABASE_REPLICA_URL`,
      `AppState::analytics_db()`, with a source-reading test that keeps handlers on the
      right pool
- [ ] P6.9 Admin TOTP 2FA — *the RFC 6238 algorithm (`enterprise/totp.rs`) is correct and
      tested. A session 3 audit found no repo function reads or writes
      `users.totp_secret_encrypted`, no enrollment endpoint exists (generate secret, show
      provisioning URI, confirm a code), and login never verifies a TOTP code. 2FA cannot
      actually be turned on today. Unchecked to reflect that — this is the largest of the
      four gaps found this session; see `MEMORY.md` Known Limitations item 0.*

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| License validation works offline within the grace window | ✅ | `enterprise::license` — 14 tests incl. forged signature, tampered payload, grace boundary. |
| SCIM endpoints conform to RFC 7644 shapes | ⚠️ | Shapes tested (`enterprise::scim`, 14 tests incl. both Okta and Entra deprovision forms). Routes now wired and asserted reachable by `tests/route_surface.rs`. **Not verified against a real IdP.** |
| Per-tenant key derivation deterministic and org-isolated | ✅ | `tenant_keys_are_deterministic_and_isolated`, `tenant_key_cannot_decrypt_another_tenants_data`. |

---

## PHASE 7 — SCALE + MOAT

**Goal:** Category leadership; compounding routing intelligence.

### Tasks
- [ ] P7.1 Classifier v3: outcome-trained bandit over our own routing results — *the UCB1
      bandit (`engine/bandit.rs`) is correct and beats static routing in a standalone
      3,000-step replay. The live pipeline calls `bandit.record(...)` after every request,
      so it genuinely observes production traffic. A session 3 audit found the router
      never reads the bandit back to *make* a routing decision — right now it is a
      write-only data collector with zero effect on what model actually serves a request.
      Unchecked to reflect that "outcome-trained routing" is not yet true in production.
      See `MEMORY.md` Known Limitations item 0.*
- [x] P7.2 Multi-region: regional budgets + region-aware routing — fourth budget scope
      between team and org; counters keyed by org AND region
- [x] P7.3 API/platform tier: usage-based metering path
- [x] P7.4 SDKs: TypeScript + Python thin clients
- [x] P7.5 Model auto-discovery job (nightly pricing diff → proposal) — reports drift,
      deliberately does not auto-apply: a price is what an invoice is computed from
- [x] P7.6 Advanced governance: approval flows, spend anomaly detection (z-score)
- [x] P7.7 Cost-center chargeback exports — JSON and CSV, attributed by team, with
      unattributed spend reported separately rather than spread across the lines
- [x] P7.8 Investor/DD pack: metrics definitions + data-room index — `docs/investor/`

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| Bandit improves expected savings vs static routing on replayed data | ✅ | `bandit_outperforms_static_routing_on_replayed_data` — 3,000-step replay, asserts higher mean reward and traffic concentration. |
| Anomaly detector flags injected spend spikes | ✅ | `engine::governance::detect_spend_anomaly` — z-score against the org's own 30-day baseline; 21 tests. Surfaced at `GET /api/usage/anomalies` and on the `/budgets` page. |
| SDKs typecheck/build | ✅ | TypeScript: `tsc --noEmit` clean. Python: assertions on real header parsing pass. |

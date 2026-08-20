# Build Phases — Tasks and Acceptance Criteria

Derived from `MASTER_BUILD.md` Part 11. Day estimates from the original blueprint were
dropped by founder instruction — **phases are gated by acceptance criteria, not calendar
time.**

Status lives in `MEMORY.md` and `.aegis/state.json`. Tick a box **only when the work is
done and verified**, and record the evidence (test name, command, file path) in the
Evidence column of that phase's acceptance table. An unticked box is not a failure; a
falsely ticked box is.

---

## PHASE 0 — FOUNDATION

**Goal:** Repo, CI, local dev stack, schema, config.

### Tasks
- [ ] P0.1 Monorepo structure per `MASTER_BUILD.md` Part 3
- [ ] P0.2 `infra/docker-compose.yml`: postgres:16, redis:7, qdrant, gateway, web
- [ ] P0.3 SQLx migrations for the entire Part 4 schema
- [ ] P0.4 Seed script: org, user, key, real dated pricing rows for 20+ models
- [ ] P0.5 `config.rs`: typed env config
- [ ] P0.6 GitHub Actions CI: fmt, clippy, test, audit, web lint+build
- [ ] P0.7 Health endpoints: `GET /health` (checks Redis + Postgres), `GET /ready`
- [ ] P0.8 ADR-001 (Rust), ADR-002 (schema), ADR-003 (deployment topology)

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| `docker compose up` brings services up healthy | | |
| `curl /health` returns 200 with per-dependency status | | |
| CI green | | |

---

## PHASE 1 — AUTH, KEYS, ORGS

**Goal:** A user can sign up, create an org, mint an API key, and see it in the dashboard.

### Tasks
- [ ] P1.1 Signup/login/logout, argon2id hashing, session tokens (SHA-256 stored,
      HTTP-only Secure cookie), email verification
- [ ] P1.2 Orgs: personal org per signup, roles owner/admin/member/viewer
- [ ] P1.3 API key CRUD, generation, hashing, once-only display, revoke, Redis cache with
      in-memory LRU in front
- [ ] P1.4 Management API auth middleware (session OR API key)
- [ ] P1.5 Next.js signup/login pages, dashboard shell, keys page
- [ ] P1.6 Audit log wired for signup, key.created, key.revoked, member.invited
- [ ] P1.7 Tests: auth flow, key CRUD, cross-org access denial, cache invalidation

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| E2E: signup → login → create key → list → revoke → revoked key rejected | | |
| Cross-org access attempt denied | | |
| All tests green | | |

---

## PHASE 2 — CORE GATEWAY

**Goal:** `/v1/chat/completions` proxies to OpenAI/Anthropic/Google/custom with auth,
rate limiting, and metering. A working product in passthrough mode.

### Tasks
- [ ] P2.1 Provider trait + adapters: openai, anthropic, google, custom; request/response
      translation; streaming SSE passthrough
- [ ] P2.2 Pipeline stages 1,2,3,4,8,10,11: auth, rate limit (Redis Lua sliding window),
      budget check, parse/normalize, `X-Aegis-*` headers, usage event emission
- [ ] P2.3 Usage worker: stream → batch insert → counters → budget alerts
- [ ] P2.4 BYOK: credential CRUD, AES-256-GCM, cached decrypted form, test endpoint
- [ ] P2.5 Shared-model pool for the free tier (round-robin, per-org caps)
- [ ] P2.6 Model pricing table + `estimate_cost()` + pricing update script
- [ ] P2.7 Plan-based model allowlist
- [ ] P2.8 Observability: spans per stage, Prometheus metrics, Grafana dashboard JSON
- [ ] P2.9 Tests: golden-file provider translation, rate limit, budget, streaming, usage
      idempotency

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| OpenAI SDK with `base_url` at the gateway completes chat + streaming | | |
| Usage record written with correct tokens | | |
| Rate limit returns 429 with headers | | |
| Budget returns 402 | | |
| Grafana dashboard committed | | |

---

## PHASE 3 — OPTIMIZATION ENGINE

**Goal:** The intelligence. This is what makes us a product, not a proxy.

### Tasks
- [ ] P3.1 Complexity classifier v1 (heuristic), unit-tested against 100 labeled fixtures
      (40 simple / 35 medium / 25 complex)
- [ ] P3.2 Routing engine + policy loader + `routing_reason`
- [ ] P3.3 Capability map (tool use, vision, context window minimums)
- [ ] P3.4 Exact cache: fingerprint + Redis SETEX, skip rules
- [ ] P3.5 Semantic cache: Qdrant, org-namespaced, similarity >= 0.95
- [ ] P3.6 Context compression v1
- [ ] P3.7 Fallback + circuit breakers + provider health
- [ ] P3.8 Savings calculation (micro-cents, no early rounding)
- [ ] P3.9 Savings dashboard + CSV export
- [ ] P3.10 Tests: classifier accuracy >= 85%, routing rules, cache boundaries, fallback
      simulation, savings property tests

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| 10 requests at `model=gpt-4o`; >= 5 route cheaper | | |
| Repeat request hits cache with `actual_cost = 0` | | |
| Dashboard itemizes savings and our fee | | |
| Classifier accuracy >= 85% on fixtures | | |
| `passthrough` kill-switch always available | | |

---

## PHASE 4 — DASHBOARDS, BILLING, LAUNCH PREP

**Goal:** Complete user-facing surface + Stripe + docs + landing page.

### Tasks
- [ ] P4.1 Remaining dashboard pages (usage, requests, models, org, teams, policies,
      providers)
- [ ] P4.2 Budgets UI + alert delivery (email, Slack webhook)
- [ ] P4.3 Stripe: checkout, webhooks, invoice generation with savings-fee line items
- [ ] P4.4 Savings-share billing job (monthly rollup → draft invoice, manual finalize)
- [ ] P4.5 Docs site: quickstart per client, API reference, error codes, FAQ
- [ ] P4.6 Landing page: hero, savings counter, comparison table, pricing calculator
- [ ] P4.7 Admin console: users/orgs, system metrics, revenue
- [ ] P4.8 Load test (k6): 1k RPS sustained, P99 overhead < 1ms
- [ ] P4.9 Security pass against Part 9 + `SECURITY.md`
- [ ] P4.10 Runbooks: deploy, rollback, restore, provider outage, pricing update

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| Signup → upgrade → requests → savings → invoice with correct line items | | |
| Load test script committed and documented | | |
| Restore drill documented and scripted | | |

---

## PHASE 5 — LAUNCH + PROVIDER EXPANSION

**Goal:** Public launch readiness. Provider parity and beyond.

### Tasks
- [ ] P5.1 Launch playbook with assets checklist and timeline
- [ ] P5.2 Providers: openrouter, moonshot, deepseek, mistral, groq
- [ ] P5.3 Classifier v2: trainable linear model with committed weights behind a flag
- [ ] P5.4 History summarization for long contexts (off the hot path)
- [ ] P5.5 Slack alerts + weekly digest email
- [ ] P5.6 Referral program (credits)
- [ ] P5.7 Public status page
- [ ] P5.8 Changelog + feedback loop

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| All new providers pass golden-file tests | | |
| Classifier v2 beats v1 on the fixture set | | |

---

## PHASE 6 — ENTERPRISE READINESS

**Goal:** Land enterprise pilots.

### Tasks
- [ ] P6.1 SSO: OIDC + SAML assertion validation
- [ ] P6.2 SCIM v2 user/group provisioning endpoints
- [ ] P6.3 Self-hosted distribution + signed license validation with offline grace
- [ ] P6.4 Per-tenant encryption keys (HKDF from master + org_id)
- [ ] P6.5 Data residency: region pinning per org
- [ ] P6.6 Compliance pack: DPA template, subprocessor list, security whitepaper
- [ ] P6.7 Audit log export (JSONL, SIEM-friendly)
- [ ] P6.8 Read replica routing for analytics queries
- [ ] P6.9 Admin TOTP 2FA

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| License validation works offline within the grace window | | |
| SCIM endpoints conform to RFC 7644 shapes | | |
| Per-tenant key derivation deterministic and org-isolated | | |

---

## PHASE 7 — SCALE + MOAT

**Goal:** Category leadership; compounding routing intelligence.

### Tasks
- [ ] P7.1 Classifier v3: outcome-trained bandit over our own routing results
- [ ] P7.2 Multi-region: regional budgets + region-aware routing
- [ ] P7.3 API/platform tier: usage-based metering path
- [ ] P7.4 SDKs: TypeScript + Python thin clients
- [ ] P7.5 Model auto-discovery job (nightly pricing diff → proposal file)
- [ ] P7.6 Advanced governance: approval flows, spend anomaly detection (z-score)
- [ ] P7.7 Cost-center chargeback exports
- [ ] P7.8 Investor/DD pack: metrics definitions + data-room index

### Acceptance
| Criterion | Met | Evidence |
|---|---|---|
| Bandit improves expected savings vs static routing on replayed data | | |
| Anomaly detector flags injected spend spikes | | |
| SDKs typecheck/build | | |

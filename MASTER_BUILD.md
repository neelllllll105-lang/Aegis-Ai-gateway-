# AEGIS — MASTER BUILD INSTRUCTION SET
## The Complete, Final, Copy-Paste-Ready Blueprint

> **How to use this document:** This is the single source of truth for the project.
> Read it fully before writing any code. Follow the build phases in order.
> Never deviate from the non-negotiable principles in Part 1.
>
> **Deviations are permitted ONLY via an ADR in `docs/adr/`.** See `docs/adr/README.md`
> for every deviation taken so far and its rationale.

---

# PART 0: PROJECT CHARTER — WHAT WE ARE BUILDING AND WHY

## Product Identity

**Name:** Aegis
**Category:** AI Cost Optimization Platform (intelligent gateway + control plane)
**Tagline:** "Cut your AI bill by up to 90%. Keep the quality. Prove it."

## What Aegis Is

Aegis is a **commercial, closed-source, multi-tenant SaaS platform** that sits between
applications and AI model providers. Users point their existing tools (Cursor, Claude
Code, Cline, aider, VS Code extensions, the OpenAI SDK, the Anthropic SDK, custom apps)
at Aegis by changing one base URL. Aegis then:

1. **Routes** every request to the cheapest model that will preserve quality
2. **Caches** responses (exact + semantic) so repeats cost $0
3. **Compresses** context to reduce token counts
4. **Governs** spend with budgets, rate limits, and per-key policies
5. **Meters** every request: tokens, cost, latency, savings
6. **Proves** savings with per-request, auditable attribution
7. **Fails over** automatically across providers during outages

## What Aegis Is NOT

- Not a model provider (we never serve our own model weights)
- Not an observability-only tool (we act, not just report)
- Not open source (closed, commercial — all rights reserved)
- Not a wrapper around OpenRouter (direct provider integrations, BYOK settles directly
  with providers)

## Target Users (In Order of Acquisition)

1. **Individual developers** drowning in AI API bills (free tier -> $29/mo Pro)
2. **Startups/small teams** needing shared governance ($299/mo Team)
3. **Enterprises** needing compliance, self-hosting, SSO ($2k+/mo)
4. **API/platform customers** building on our infrastructure (usage-based)

## Business Model (Final — Do Not Change Without Founder Approval)

| Tier | Price | Included | Our Revenue Source |
|------|-------|----------|-------------------|
| **Free** | $0 | 10k requests/mo on shared models (GPT-4o-mini, Gemini Flash), basic dashboard | Acquisition cost only (~$2-5/user/mo) |
| **Pro** | $29/mo + 20% of verified savings | BYOK, unlimited requests, full optimization engine, semantic caching, savings dashboard | Subscription + savings share |
| **Team** | $299/mo + 15% of verified savings | Up to 10 seats, org features, roles, per-team budgets, Slack alerts | Subscription + savings share |
| **Enterprise** | $2,000+/mo + 10% of verified savings | SSO/SAML, SCIM, audit logs, data residency, self-hosted Docker, SLA 99.9%, dedicated support | Subscription + savings share |
| **API/Platform** | $0.0001/request + provider passthrough | Volume licensing, white-label options | Pure usage |

**Savings-share math (implement exactly):**

```
baseline_cost    = cost of serving request on the model the user REQUESTED (premium default)
actual_cost      = cost of serving request on the model we actually USED (or $0 on cache hit)
gross_savings    = baseline_cost - actual_cost
our_fee          = gross_savings * savings_share_rate (0.20 Pro / 0.15 Team / 0.10 Enterprise)
                   but ONLY when gross_savings > 0
customer_net     = gross_savings - our_fee
```

The savings dashboard shows: `baseline_cost`, `actual_cost`, `gross_savings`, `our_fee`,
`customer_net` — per request, aggregated per day/month/team/key.
**Transparency is the product.**

## Competitive Context (Why These Decisions)

- **Tokenator.ai** is in private beta, invite-only, no disclosed funding, cloud-only,
  6-7 providers, black-box savings claims. Our advantages: speed to market, sub-1ms Rust
  performance, self-hosted option, auditable savings, 20+ providers by Phase 5.
- **LiteLLM** is open-source Python, 140+ providers, but 10-50ms overhead and no
  quality-aware routing.
- **Portkey** is $2k-10k/mo, closed, log-based pricing.
- **OpenRouter** adds 5% markup, no governance.
- **Our wedge:** Fast (Rust) + Smart (quality-aware routing) + Transparent (auditable
  savings) + Self-hostable (regulated industries).

---

# PART 1: NON-NEGOTIABLE PRINCIPLES

**These rules override everything. If any code violates them, the code is wrong.**

1. **PERFORMANCE:** Gateway must add < 1ms P99 latency overhead to the request path.
   Nothing in the hot path may do synchronous I/O to PostgreSQL. Redis or in-memory only
   in the hot path.
2. **METER EVERYTHING:** Every single request through the gateway produces a usage record
   with full cost attribution. No request is ever untracked. Billing accuracy is
   financial-grade.
3. **SECURITY IS FOUNDATIONAL:** Provider keys encrypted at rest (AES-256-GCM), never
   logged, never returned to the browser. Tenant isolation enforced at the data layer
   (every query scoped by org_id). API keys stored only as SHA-256 hashes.
4. **ZERO CONTENT RETENTION BY DEFAULT:** We never persist prompt/response content unless
   a tenant explicitly opts in. Operational metadata only (model, tokens, cost, latency,
   cache status).
5. **SCALE WITHOUT REWRITE:** Stateless gateway, horizontally scalable, all state in
   Redis/PostgreSQL, multi-tenant from day one. Decisions must survive 1M+ users.
6. **COST DISCIPLINE:** Infrastructure target: <$50/mo for first 1,000 users (single
   Hetzner server), <$500/mo for 10,000 users. No AWS/GCP until >$10k MRR. No Kubernetes
   until >10k RPS sustained.
7. **CLOSED SOURCE:** No open-source license. No public repos. Proprietary code, all
   rights reserved. Do not copy code from GPL/AGPL projects.
8. **SIMPLICITY WHERE SUFFICIENT, SOPHISTICATION WHERE NECESSARY:** Monolith-first
   (modular Rust binary). Extract services only when a module independently needs to
   scale.

---

# PART 2: TECHNOLOGY STACK (FINAL — DO NOT DEVIATE)

| Component | Technology | Why (vs alternatives) |
|-----------|-----------|----------------------|
| **Gateway/Core** | Rust + Axum + Tokio | Sub-1ms overhead; no GC pauses; single binary deploys. Beats Python (LiteLLM, 10-50ms) and Go (GC pauses). |
| **HTTP Client** | reqwest + hyper | Battle-tested async HTTP with connection pooling, timeouts, streaming. |
| **DB Access** | SQLx (PostgreSQL) | Async-native, no ORM impedance, prepared statements only. |
| **Cache/Rate Limit** | Redis 7 (redis-rs) | Atomic Lua scripts for sliding-window rate limiting; sub-ms lookups. |
| **Vector Cache** | Qdrant | Rust-native vector DB for semantic cache similarity search (Phase 3). |
| **Frontend** | Next.js 15 (App Router) + TypeScript + Tailwind CSS | SSR for SEO/launch, mature ecosystem, fast dashboard dev. |
| **Database** | PostgreSQL 16 | ACID for billing; JSONB for metadata; partitioning for usage records. |
| **Auth** | Custom session tokens + hashed API keys; OIDC/SAML in Phase 6 | No Clerk/Auth0 vendor fees ($240+/mo at scale); full multi-tenant control. |
| **Payments** | Stripe (Phase 4) | Developer-standard, handles taxes/invoices. |
| **Observability** | OpenTelemetry + Prometheus + Grafana (self-hosted) | ~$0 incremental on our server vs $10k+/mo Datadog at scale. |
| **Email** | Resend (free tier: 100/day) then Postmark | Transactional email for alerts/verification. |
| **Deploy Platform** | Coolify on Hetzner | Self-hosted PaaS: push-to-deploy, SSL, backups — Heroku UX at Hetzner prices. |
| **Edge/Security** | Cloudflare Free Tier | DNS, SSL, CDN, DDoS protection, WAF — $0. |
| **CI/CD** | GitHub Actions | Free for private repos up to generous limits; matrix builds. |
| **Secrets** | Docker secrets + age-encrypted .env files; (Phase 6: Vault/KMS) | Simple now, upgradeable later. |

**Forbidden technologies:** MongoDB (wrong consistency model for billing), Kafka (overkill
until >50k events/sec — Redis Streams first), Kubernetes (until >10k RPS), microservices
(until a module independently needs scaling), any GPL/AGPL-licensed code (contaminates
closed source).

---

# PART 3: REPOSITORY STRUCTURE

```
aegis/
├── MASTER_BUILD.md              # This document (source of truth)
├── MEMORY.md                    # LIVING PROJECT STATE — read this first
├── CLAUDE.md                    # Claude Code behavioral instructions
├── .aegis/state.json            # Machine-readable phase/task status
├── .github/workflows/           # ci.yml, deploy.yml
├── apps/
│   ├── gateway/                 # RUST — the core product
│   │   ├── migrations/          # SQLx migrations
│   │   └── src/
│   │       ├── main.rs, config.rs, error.rs, telemetry.rs
│   │       ├── routes/          # openai_compat, anthropic_compat, management
│   │       ├── middleware/      # auth, rate_limit, budget
│   │       ├── engine/          # classifier, router, compressor, fallback
│   │       ├── cache/           # exact, semantic
│   │       ├── providers/       # openai, anthropic, google, ... custom
│   │       ├── metering/        # pricing, usage, savings
│   │       ├── billing/
│   │       └── workers/         # usage_writer, budget_alerts, reconciliation
│   └── web/                     # NEXT.JS — dashboard + marketing
├── infra/                       # docker-compose, coolify, cloudflare, grafana
├── docs/adr/                    # Architecture Decision Records (MANDATORY)
├── docs/runbooks/               # Operational runbooks
└── scripts/                     # Seed, pricing-table update, status helpers
```

---

# PART 4: DATABASE SCHEMA

Implemented as SQLx migrations in `apps/gateway/migrations/`. **Migrations are the
source of truth** for the schema. Tables:

- **users, sessions** — auth
- **organizations, org_memberships, teams, team_memberships** — tenancy root
- **api_keys** — SHA-256 hashed, prefix for UI, per-key limits/budgets/allowlists
- **provider_credentials** — BYOK, AES-256-GCM encrypted
- **routing_policies** — JSONB rules engine
- **usage_records** — PARTITIONED MONTHLY, the billing source of truth, micro-cent money
- **budgets, budget_alerts** — spend governance
- **audit_logs** — append-only, 7-year retention
- **invoices** — billing
- **model_pricing** — versioned, effective_from/effective_to
- **provider_health_events** — circuit breaker history
- **captured_content** — OPT-IN ONLY, encrypted

All money is stored in **micro-cents** (1 cent = 10,000 micro-cents) as BIGINT.
Floating point never touches a stored monetary value.

**PostgreSQL configuration requirements:** Enable `pg_stat_statements`. Set
`max_connections = 200`. All tenant-scoped queries MUST include `org_id` in WHERE —
enforced via code review and integration tests.

---

# PART 5: THE GATEWAY REQUEST PIPELINE (THE HEART OF THE PRODUCT)

```
[0]  EDGE: Cloudflare (DDoS/WAF/TLS) -> forwards to Hetzner gateway
[1]  AUTHENTICATION        (< 0.05ms)  LRU -> Redis -> Postgres, SHA-256 key hash
[2]  RATE LIMITING         (< 0.1ms)   Redis Lua sliding window, per-key + per-org
[3]  BUDGET ENFORCEMENT    (< 0.1ms)   Redis counters, 402 on hard exceed
[4]  PARSING/NORMALIZATION (< 0.5ms)   serde -> internal RequestFormat + fingerprint
[5]  CACHE LOOKUP                      [5a] exact (Redis)  [5b] semantic (Qdrant >= 0.95)
[6]  ROUTING DECISION      (< 1ms)     [6a] classifier  [6b] selection  [6c] compression
[7]  PROVIDER EXECUTION                retry (2x), circuit breaker, failover, streaming SSE
[8]  RESPONSE NORMALIZATION            provider -> OpenAI/Anthropic-compatible
[9]  COST & SAVINGS CALC   (< 0.05ms)  micro-cents, baseline vs actual, aegis fee
[10] USAGE EVENT EMISSION  (< 0.1ms)   Redis XADD + HINCRBY counters, non-blocking
[11] RESPONSE TO CLIENT                with X-Aegis-* headers
[12] BACKGROUND USAGE WORKER           batch -> Postgres, idempotent, budget alerts
```

**Latency budget (total gateway overhead P99 < 1ms excluding provider call):**
Auth 0.05 + RateLimit 0.1 + Budget 0.1 + Parse 0.5 + CacheLookup 0.1 + Routing 0.5 +
Emit 0.1. **Instrument and display `gateway_overhead_ms` on every request — we eat our
own dog food.**

**Complexity classifier v1 (heuristics — deterministic, fast):** weighted sum of message
count and total input tokens, presence of code blocks/keywords, reasoning verbs
(analyze/reason/prove/derive/debug/architect), tool use (floor 0.7), system prompt length.
Score 0.0-1.0. `< 0.35 simple | 0.35-0.7 medium | > 0.7 complex`.

**Routing rules:**
- simple  -> cheapest tier that passes the capability check
- medium  -> mid tier
- complex -> requested model (or frontier equivalent)
- **Conservative default: if unsure, use the requested model. NEVER degrade quality on
  complex requests. Savings come from the simple-request long tail.**

**Cache rules:** skip cache if temperature > 0.7, tools present, or org.zero_retention.
Fingerprint ALWAYS includes org_id — cache is tenant-scoped, never cross-tenant.

**Response headers on every request:** `X-Aegis-Model`, `X-Aegis-Requested-Model`,
`X-Aegis-Cost`, `X-Aegis-Baseline-Cost`, `X-Aegis-Savings`, `X-Aegis-Cache`,
`X-Aegis-Routing`, `X-Aegis-Latency`, `X-Aegis-Request-Id`.

---

# PART 6: COMPLETE API SPECIFICATION

## Public API (Gateway — OpenAI-compatible)

```
POST /v1/chat/completions     # Primary. Full OpenAI schema + streaming SSE.
                              # X-Aegis-Routing-Hint: auto|passthrough|cheap
POST /v1/completions          # Legacy text completions
POST /v1/embeddings           # Routed to cheapest embedding provider
GET  /v1/models               # Models available TO THIS ORG
```

## Public API (Anthropic-compatible)

```
POST /v1/messages             # Full Anthropic Messages API schema.
```

## Management API (all under /api, session or API-key auth, org-scoped)

```
Auth:       POST /api/auth/{signup,login,logout,verify-email,forgot-password,reset-password}
Keys:       GET|POST /api/keys ; GET|PATCH|DELETE /api/keys/:id
Org:        GET|PATCH /api/org ; GET /api/org/members ; POST /api/org/members/invite
            PATCH|DELETE /api/org/members/:userId
Teams:      GET|POST /api/org/teams ; PATCH|DELETE /api/org/teams/:id
            POST /api/org/teams/:id/members
Providers:  GET|POST /api/providers ; PATCH|DELETE /api/providers/:id
            POST /api/providers/:id/test
Policies:   GET|POST /api/policies ; PATCH|DELETE /api/policies/:id
Usage:      GET /api/usage ; /api/usage/summary ; /api/savings/report[.csv]
            GET /api/requests
Budgets:    GET|POST /api/budgets ; PATCH|DELETE /api/budgets/:id
            POST /api/budgets/:id/alerts
Billing:    GET /api/billing/plan ; POST /api/billing/{subscribe,cancel}
            GET /api/billing/invoices[/:id]
Admin:      GET /api/admin/{users,orgs,metrics,revenue} ; POST /api/admin/users/:id/disable
```

**All management endpoints:** org-scoped by default, audit-logged for mutations,
rate-limited, JSON errors with machine-readable `type` field and `docs_url`.

---

# PART 7: FRONTEND SPECIFICATION

Next.js 15 App Router. Route groups: `(marketing)`, `(auth)`, `(dashboard)`, `(admin)`.

| Route | Purpose |
|-------|---------|
| `/` | Landing: hero, live savings counter, 3-step how-it-works, comparison table, pricing, FAQ |
| `/pricing` | 4 tiers + API tier, interactive savings calculator |
| `/docs` | Integration guides: OpenAI SDK, Anthropic SDK, Cursor, Claude Code, Cline, aider, raw HTTP |
| `/login`, `/signup` | Auth forms |
| `/dashboard` | **The money page.** Spend, savings, requests, cache hit rate, savings ribbon chart |
| `/usage` | Time-series explorer |
| `/savings` | Itemized savings report + CSV export |
| `/requests` | Searchable metadata log (NO content column by design) |
| `/keys` | Key CRUD, create modal shows key once |
| `/providers` | BYOK management, test button, never displays stored keys |
| `/models` | Model performance comparison from our aggregate data |
| `/org` | Members, roles, teams, budgets, routing policies |
| `/billing` | Plan, usage vs limits, Stripe portal, invoices |
| `/admin` | Internal: users/orgs, system metrics, revenue |

**Architecture rules:** server components for data fetching; HTTP-only cookie session;
Recharts for charts; dark-mode-first developer aesthetic; one accent color; monospace for
all numbers/code; no client-side secrets ever.

---

# PART 8: PROVIDER ADAPTER SYSTEM (PHASED)

Every provider implements the `Provider` trait in `apps/gateway/src/providers/mod.rs`:
`id()`, `supported_models()`, `chat()`, `chat_stream()`, `estimate_cost()`.

- **Phase 1 (launch):** openai, anthropic, google, custom (any OpenAI-compatible base URL)
- **Phase 2 (wk 9-12):** openrouter, moonshot, deepseek, mistral, groq
- **Phase 3 (mo 4-6):** aws_bedrock, azure_openai, google_vertex
- **Phase 4 (mo 6+):** together, fireworks, cohere, baseten, replicate

**Shared-model pool (free tier):** pool of our own GPT-4o-mini / Gemini Flash keys
(round-robin) for free-tier users. Per-org daily caps enforce the 10k/month limit.
Monitor pool burn daily — this is our only real COGS on free users.

---

# PART 9: SECURITY ARCHITECTURE

Implement all from Phase 1. Security review gates every phase merge.

1. **API keys:** `aegis_sk_<43 base62>`, OS CSPRNG, SHA-256 stored, shown once,
   constant-time comparison, prefix lookup for UI, rotation = create new + revoke old.
2. **Provider keys (BYOK):** AES-256-GCM with `AEGIS_MASTER_KEY` (32 bytes, base64).
   Master key NEVER in DB, never logged. Per-tenant keys via HKDF (Phase 6).
3. **Transport:** TLS everywhere, HSTS, no plaintext listener.
4. **Tenant isolation:** EVERY query includes `org_id`. Any query without org scoping
   blocks the merge. Integration tests attempt cross-tenant access (must 403/404).
5. **Content:** zero-retention default. Opt-in content encrypted per-tenant, toggles
   audit-logged.
6. **Secrets in logs:** redaction middleware — `sk-`, `aegis_sk_`, `Bearer ` -> `[REDACTED]`.
   Tested.
7. **Rate limits (abuse):** per-IP, per-key, per-org. 10MB body cap. 120s hard timeout.
8. **Injection:** SQLx prepared statements ONLY. No string-built SQL. React escaping + CSP.
9. **Headers:** strict CSP, X-Content-Type-Options, X-Frame-Options DENY, Referrer-Policy.
10. **Audit log:** every mutation of keys/credentials/policies/budgets/members.
    Append-only.
11. **Dependency security:** `cargo audit` + `npm audit` in CI, fail on critical.
12. **Admin access:** separate role flag, TOTP 2FA (Phase 6), all actions audit-logged.

---

# PART 10: DEPLOYMENT ARCHITECTURE (COST-OPTIMIZED)

## Phase 1 stack — target <= $50/month total

```
[Internet] -> [Cloudflare FREE: DNS, SSL, CDN, DDoS, WAF]
                  |
       [Hetzner CPX31: 4 vCPU / 8GB / 80GB NVMe — EU]
       |  Coolify (self-hosted PaaS) manages:
       ├─ aegis-gateway  (Rust container, 2 replicas)
       ├─ aegis-web      (Next.js container)
       ├─ postgres:16    (volume + nightly encrypted backup to B2)
       ├─ redis:7        (AOF, maxmemory 1gb allkeys-lru)
       ├─ qdrant         (Phase 3 semantic cache)
       └─ prometheus + grafana + loki
```

Budget **$30-45/month** for CPX31-class (Hetzner raised prices mid-2026). CPX21
(2 vCPU/4GB) suffices for the first 500 users.

## Scaling Path (do NOT jump ahead)

| Trigger | Action | New Cost |
|---------|--------|----------|
| >60% CPU sustained or P99 > 3ms | 2nd Hetzner server, gateway-only | ~$80/mo |
| Redis > 512MB or >5k cmd/s | Dedicated Redis server | +$20/mo |
| Postgres > 60% CPU or > 20GB | Dedicated DB server + read replica | +$35/mo |
| > 5k RPS or multi-region | 3rd region gateway + CF load balancing | +$40/mo |
| > 10k RPS sustained | NOW consider Kubernetes — not before | — |
| Enterprise demand | Same images -> customer VPC (self-hosted license) | Revenue |

Deployment procedure: `docs/runbooks/deploy.md`.

---

# PART 11: BUILD PHASES

| Phase | Goal |
|-------|------|
| **0** | Foundation: repo, CI, local dev stack, schema, config, health endpoints, ADRs |
| **1** | Auth, keys, orgs: signup -> org -> mint key -> dashboard |
| **2** | Core gateway: `/v1/chat/completions` proxying with auth, rate limit, metering, BYOK |
| **3** | Optimization engine: classifier, routing, exact + semantic cache, compression, fallback, savings |
| **4** | Dashboards, billing, launch prep: full UI, Stripe, docs, landing, admin, load test |
| **5** | Launch + provider expansion: PH launch, 5 more providers, classifier v2, Slack, referrals |
| **6** | Enterprise readiness: SSO/SAML, SCIM, self-hosted distribution, residency, compliance |
| **7** | Scale + moat: classifier v3 (outcome-trained), multi-region, API tier, SDKs, auto-discovery |

Detailed task lists and acceptance criteria per phase: `docs/PHASES.md`.
Live status: `MEMORY.md` and `.aegis/state.json`.

---

# PART 12: CODING STANDARDS (ENFORCED IN CI)

**Rust:** `cargo fmt` + `clippy -D warnings` clean. No `unwrap()`/`expect()` outside tests
and startup. Every public fn documented. All I/O async. No GPL/AGPL crates. Dependency
additions require an ADR note.

**TypeScript:** strict mode, ESLint + Prettier, no `any` in committed code. Server
components by default; `"use client"` only for interactivity.

**Tests (gate every merge):** Unit tests for all business logic (>= 80% on
engine/metering/billing). Integration tests for every API endpoint (happy + auth-fail +
rate-limit + cross-tenant). Golden-file provider translation tests. Load test script
committed.

**Commits:** Conventional commits (`feat:`, `fix:`, `chore:`). Every PR references a phase
task. No direct pushes to main.

**Errors:** Unified error enum -> JSON
`{"error": {"type": "budget_exceeded", "message": "...", "docs_url": "..."}}`.

---

# PART 13: WHAT MUST NEVER BE COMPROMISED

1. **Billing accuracy.** Micro-cent integer math, idempotent writes, daily reconciliation
   job (Redis counters vs Postgres rollups — alert on drift > $0.01).
2. **Tenant isolation.** One cross-tenant leak = company-ending.
3. **Zero content retention default.**
4. **Latency honesty.** We display our own overhead on every request. Never game it.
5. **The passthrough escape hatch.** Users can always force their requested model.
6. **Backups tested by restore.** An untested backup is not a backup.
7. **No secrets in logs, ever.**
8. **Pricing table integrity.** Every cost number traceable to a dated source.

---

# PART 14: PRODUCTION READINESS DEFINITION

A phase is "production ready" only when ALL are true:

- [ ] All phase acceptance criteria verified and output logged
- [ ] CI green: fmt, clippy, tests, audit, frontend build
- [ ] Load test at 2x expected traffic passes latency/error targets
- [ ] Restore-from-backup drill executed successfully this month
- [ ] Runbook exists for every new operational surface
- [ ] Security checklist (Part 9) re-verified
- [ ] Grafana dashboards show new components; alerts configured
- [ ] Error budget: zero unresolved P1s
- [ ] Reconciliation job green

---

# PART 15: CLAUDE CODE BEHAVIORAL CONTRACT

1. **Read this entire document before writing any code.** Re-read the relevant phase
   section before each phase begins.
2. **Build phases strictly in order.** Never start Phase N+1 while Phase N acceptance
   criteria are unmet. If blocked, report the blocker — do not silently skip.
3. **Act as a senior engineer, not a code generator:** when this document is ambiguous or
   wrong, STOP, write an ADR proposing the resolution, and flag it for founder approval.
4. **Every module gets tests in the same PR.** Untested code is unfinished code.
5. **Security is a gate, not a task:** re-check Part 9 at every phase boundary.
6. **Write documentation as you build:** ADRs for decisions, runbooks for operations,
   inline rustdoc/JSDoc for public functions.
7. **Never introduce a technology not in Part 2** without an ADR and founder approval.
8. **Optimize for the reader:** clear code, real names, no cleverness in billing or auth.
9. **Money is integers.** All currency in micro-cents (i64/i128). Floating point never
   touches a stored monetary value.
10. **When in doubt, be conservative:** route to the requested model, keep the data, log
    the decision.

---

*End of Master Build Instruction Set. This document is proprietary and confidential.
Version 1.0 — Final.*

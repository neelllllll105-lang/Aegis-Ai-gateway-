# MEMORY.md — Living Project State

> **If you are an AI agent or a new engineer picking this project up: start here.**
> This file is the handoff protocol. It tells you where the project is, what works, what
> does not, what was decided and why, and exactly what to do next.
>
> **Last updated:** 2026-08-20 — Session 1 (bootstrap)
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
| 0 | Foundation: repo, CI, dev stack, schema, config | 🟡 in progress |
| 1 | Auth, keys, orgs | ⚪ not started |
| 2 | Core gateway (proxy + metering) | ⚪ not started |
| 3 | Optimization engine | ⚪ not started |
| 4 | Dashboards, billing, launch prep | ⚪ not started |
| 5 | Launch + provider expansion | ⚪ not started |
| 6 | Enterprise readiness | ⚪ not started |
| 7 | Scale + moat | ⚪ not started |

Legend: ⚪ not started · 🟡 in progress · 🟢 complete · 🔴 blocked

Machine-readable equivalent: `.aegis/state.json`.

---

## Current Focus

Bootstrapping the repository: directory structure, the source-of-truth documents, and the
handoff mechanism itself (this file, `CLAUDE.md`, `.aegis/state.json`, `scripts/`).

---

## What Actually Works Right Now

Nothing is runnable yet — the repo was created in this session. This section will list
only **verified** capabilities (things actually executed, not things merely written).

---

## Blockers

| Blocker | Impact | Workaround |
|---------|--------|------------|
| Docker Desktop not running on the build machine | Cannot run Postgres/Redis/Qdrant locally; cannot run integration tests against real services | Gateway is built to compile and unit-test with no database. Integration tests are gated behind `AEGIS_TEST_DATABASE_URL` and run in CI where Postgres is a service container. |

---

## Next Steps (do these in order)

1. Finish Phase 0 scaffolding (Cargo workspace, config, error types, health endpoints).
2. Write the full schema as SQLx migrations.
3. Stand up CI.
4. Then proceed to Phase 1 per `docs/PHASES.md`.

---

## Key Decisions (and why)

Full records in `docs/adr/`. Highlights:

- Nothing recorded yet.

---

## Gotchas / Traps

- Nothing recorded yet.

---

## Session Log

Append a dated entry per working session. Newest first.

### 2026-08-20 — Session 1 — Claude Opus 5
Repository bootstrap started. Created directory structure, `MASTER_BUILD.md`,
`CLAUDE.md`, this file.

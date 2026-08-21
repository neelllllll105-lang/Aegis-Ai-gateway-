# Aegis

**Cut your AI bill by up to 90%. Keep the quality. Prove it.**

An AI cost-optimization gateway. Applications point at Aegis instead of OpenAI, Anthropic,
or Google by changing one base URL. Every request is then authenticated, rate limited,
budget checked, cached, routed to the cheapest model that preserves quality, metered, and
attributed — in under a millisecond of added latency.

> Proprietary and confidential. All rights reserved. Not open source.

---

## Start here

| If you are… | Read |
|---|---|
| Picking this project up cold | **[`docs/HANDOFF.md`](docs/HANDOFF.md)** |
| An AI agent working in this repo | **[`CLAUDE.md`](CLAUDE.md)** (loaded automatically) |
| Looking for current state | **[`MEMORY.md`](MEMORY.md)** |
| Looking for the product spec | [`MASTER_BUILD.md`](MASTER_BUILD.md) |
| Wondering why something is built that way | [`docs/adr/`](docs/adr/README.md) |

```bash
bash scripts/status.sh    # what the repository actually contains right now
```

---

## Run it

Works on a fresh clone with nothing installed but Rust:

```bash
cargo test --lib          # 622 tests, ~3 seconds, no database required
cd apps/gateway && cargo run
curl http://localhost:8080/health
```

With the full stack:

```bash
docker compose -f infra/docker-compose.yml up -d
export DATABASE_URL=postgres://aegis:aegis_dev_password@localhost:5432/aegis
export REDIS_URL=redis://localhost:6379
cd apps/gateway && cargo run

cd apps/web && npm install && npm run dev
```

---

## Layout

```
apps/gateway/     Rust + Axum. The product.
apps/web/         Next.js 15. Dashboard and marketing.
docs/             ADRs, runbooks, phase plan, handoff guide
infra/            docker-compose, load test, deployment config
scripts/          status, seed, classifier training, memory freshness
sdks/             Thin TypeScript and Python clients
```

The single most useful file to read first is
`apps/gateway/src/routes/openai_compat.rs` — it contains the whole request pipeline, and
everything else exists to make that function correct and fast.

---

## How it works

Three mechanisms, each independently auditable and each individually disableable:

1. **Quality-aware routing.** A classifier scores request complexity. Simple requests go
   to a cheap model, complex ones go to exactly what was asked for. The router resolves
   every ambiguity toward the requested model — savings come from the long tail of easy
   requests, never from degrading hard ones.
2. **Exact and semantic caching.** Identical requests cost nothing. Near-identical ones
   match by embedding similarity above a strict threshold. Cache keys hash the
   organisation id, so cross-tenant hits are structurally impossible rather than filtered.
3. **Context compression.** Duplicated system prompts, redundant whitespace, and stale
   history are removed before the request leaves — never touching code blocks, and never
   silently truncating history without telling the model.

Every request returns its own receipt:

```
X-Aegis-Model:            openai/gpt-4o-mini
X-Aegis-Requested-Model:  openai/gpt-4o
X-Aegis-Cost:             $0.000450
X-Aegis-Baseline-Cost:    $0.007500
X-Aegis-Savings:          $0.007050
X-Aegis-Latency:          842ms (overhead: 0.371ms)
```

---

## The rules that hold everything together

Each is enforced by a test that fails loudly, not by convention:

1. **Money is integers.** Micro-cents, `i64`, everywhere. A million small charges sum
   exactly, so a customer recomputing an invoice from the CSV export gets our number to
   the micro-cent.
2. **Every query is scoped by `org_id`.** The repository layer makes it structural, and a
   test reads its own source to verify it.
3. **Every request produces exactly one usage record** — including rejected ones. The
   reconciliation worker alerts on any gap.
4. **No secrets in logs.** Nine credential formats scrubbed, tested against a realistic
   full-request log line.
5. **Never downgrade a complex request.** Trust outranks savings, and the passthrough
   escape hatch is permanent.

---

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                     # unit; integration tests skip without a database
cd apps/web && npm run lint && npm run typecheck && npm run build
bash scripts/check-memory-freshness.sh
```

CI additionally runs the integration suite against real Postgres and Redis, audits
dependencies, and fails the build on any GPL or AGPL dependency.

---

## Known limitations

Kept honest in [`MEMORY.md`](MEMORY.md). The one that matters most:

> **Seed pricing is unverified.** The model prices were transcribed during the initial
> build, not fetched from provider price sheets. Every savings figure depends on them.
> Follow [`docs/runbooks/pricing-update.md`](docs/runbooks/pricing-update.md) before
> billing anyone.

# HANDOFF.md — Picking this project up cold

You have just been handed the Aegis repository. This document gets you from nothing to
productively contributing. It assumes no prior context and no conversation with whoever
worked on it last.

If you read only one other file, read **`MEMORY.md`**.

---

## 1. Orient (10 minutes)

Run this first. It reports what the repository *actually* contains right now, which is more
trustworthy than any prose — including this document.

```bash
bash scripts/status.sh
```

Then read, in this order:

| File | What it tells you | Trust it? |
|---|---|---|
| `MEMORY.md` | Where the project is, what works, what is broken, what to do next | Mostly — check its date |
| `MASTER_BUILD.md` | What we are building and why. The product blueprint | Yes — it is the contract |
| `docs/PHASES.md` | Task list and acceptance criteria per phase | Yes |
| `docs/adr/README.md` | Why things are the way they are, including deviations | Yes |
| `.aegis/state.json` | Machine-readable phase status | Yes — CI validates it |

**When `MEMORY.md` and `scripts/status.sh` disagree, the script is right.** Fix the memory
file as your first contribution.

---

## 2. Get it running (5 minutes)

### Without Docker

This works on a fresh clone with nothing installed but Rust. It is the fastest way to
confirm your toolchain is sane.

```bash
cargo test --lib
```

You should see several hundred tests pass in a few seconds. The gateway compiles and
unit-tests with no database, no Redis, and no API keys — that is deliberate
(`docs/adr/0004-runtime-checked-sql.md` and `0005-store-abstraction.md` explain why).

You can also just run it:

```bash
cd apps/gateway && cargo run
curl http://localhost:8080/health
```

It starts in degraded mode: an in-process store, no persistence, and management endpoints
returning an error. That is enough to exercise the gateway pipeline.

### With Docker (full stack)

```bash
docker compose -f infra/docker-compose.yml up -d
export DATABASE_URL=postgres://aegis:aegis_dev_password@localhost:5432/aegis
export REDIS_URL=redis://localhost:6379
export QDRANT_URL=http://localhost:6333
cd apps/gateway && cargo run
```

Migrations run automatically at startup. To run the DB-backed integration tests:

```bash
export AEGIS_TEST_DATABASE_URL=postgres://aegis:aegis_dev_password@localhost:5432/aegis
cargo test --tests
```

Without that variable those tests skip themselves rather than failing, so a contributor
without Docker still gets a green `cargo test`.

### Dashboard

```bash
cd apps/web && npm install && npm run dev
```

---

## 3. The five things you must not break

These are not style preferences. Each one is a specific failure that would be very
expensive, and each is enforced by a test that will fail loudly.

1. **Money is integers.** Micro-cents, `i64`, everywhere. A float anywhere near a billing
   figure is a bug. See `money.rs` and ADR-002.
2. **Every query is scoped by `org_id`.** `db/repo.rs` makes this structural — functions
   take an `org_id` and filter on it, and a test reads the module source to verify it. A
   cross-tenant leak is, per Part 13, company-ending.
3. **Every request produces exactly one usage record.** Including rejected ones. The
   reconciliation worker compares request and event counters and alerts on any gap.
4. **No secrets in logs.** `telemetry::redact` handles nine credential formats and is
   tested against a realistic full-request log line. Anything that might contain
   user-supplied text goes through it.
5. **Never downgrade a complex request.** The router resolves every ambiguity toward the
   model the customer asked for. Savings come from the simple-request long tail; degrading
   a hard request is how the account is lost.

---

## 4. Where the code lives

```
apps/gateway/src/
  routes/openai_compat.rs   ← START HERE. The whole pipeline, stages [1]-[12].
  engine/                   ← The intelligence: classifier, router, policies, fallback
  metering/                 ← Pricing, usage events, savings attribution
  cache/                    ← Exact (Redis) and semantic (Qdrant) caches
  providers/                ← Nine adapters. openai.rs holds the shared translation layer
  middleware/               ← Auth, rate limiting, budgets, security headers
  db/repo.rs                ← Every query. The tenant isolation boundary
  workers/                  ← Usage writer, reconciliation, budget alerts
  enterprise/               ← Licensing, SSO, SCIM, TOTP, residency
```

`routes/openai_compat.rs::execute` is the function to read first. Everything else exists to
make it correct and fast.

---

## 5. How to make a change

```bash
# 1. Confirm you are starting from green
cargo test --lib

# 2. Make the change, with tests in the same commit. Untested code is unfinished.

# 3. Verify
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test

# 4. Update the handoff state — this is not optional
#    Edit MEMORY.md (Current Focus, What Works, Next Steps) and .aegis/state.json
bash scripts/check-memory-freshness.sh

# 5. Commit with a conventional message referencing the phase task
git commit -m "feat(engine): add semantic cache warmup [P3.5]"
```

CI fails if formatting drifts, clippy complains, tests fail, a GPL dependency appears, or
`MEMORY.md` falls more than ten code commits behind.

---

## 6. Things that will surprise you

Collected from actually building this. Each cost real time.

- **Do not put the real pricing table in pipeline tests.** The router will correctly find
  that a real provider is cheaper and your test will make live API calls to OpenAI. Test
  pricing tables must contain only mock models.
- **`serde(flatten)` on `NormalizedRequest::extra` is load-bearing.** Without it, unknown
  provider parameters are silently dropped instead of passed through.
- **Store JSON floats as `f64`, not `f32`.** `temperature: 0.2` as an `f32` serialises as
  `0.20000000298023224`, which is ugly at best and rejected at worst.
- **Anthropic requires `max_tokens`.** A request that is perfectly valid against OpenAI
  fails with a 400 unless the adapter supplies a default. It does.
- **Gemini calls the assistant role `model`** and puts the model name in the URL path, not
  the body.
- **`command -v python3` is not enough on Windows.** The App Execution Alias shim is on
  PATH and exits with an install prompt. The scripts test by running it.

---

## 7. Before you touch billing

Read `docs/adr/0002-money-as-integers.md`, then note the open launch blocker in
`MEMORY.md`: **the seed pricing table has not been verified against provider price
sheets.** Every savings figure and every invoice line depends on those numbers. Follow
`docs/runbooks/pricing-update.md` before charging anyone.

---

## 8. If you are an AI agent

`CLAUDE.md` is loaded automatically and is your behavioural contract. The short version:

- Read `MEMORY.md` first, every session.
- Phases are strictly ordered. Do not start Phase N+1 with Phase N unmet — record the
  blocker instead.
- Any deviation from `MASTER_BUILD.md` needs an ADR. Never deviate silently.
- **End every session by updating `MEMORY.md` and `.aegis/state.json`.** If you write code
  and do not update the memory, the next agent starts blind and your work is effectively
  lost.
- Report honestly. A test you did not run is not a test that passed, and an unverified
  number is not a verified one. `MEMORY.md` has a "Known Limitations" section precisely so
  there is somewhere to put uncomfortable truths rather than omitting them.

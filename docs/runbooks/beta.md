# Runbook: closed beta

The stage before `docs/runbooks/launch.md`. A public launch amplifies whatever state the
product is in; a beta's entire purpose is to find out what that state actually is, with
real users, before that amplification happens. Skipping straight to a public launch
without this step means the first 500 signups do the finding for you, in public.

---

## Why a beta, specifically, before launch

Three things are true right now that a beta is designed to catch and a code review cannot:

1. **Nothing has been verified against a real provider account.** The entire pipeline —
   routing, caching, cost computation — has only ever run against a mock provider. The
   first real request to a real model is the first time the savings arithmetic is checked
   against a real bill.
2. **A feature-wiring audit found several capabilities that are built and tested in
   isolation but not connected to live traffic** — semantic caching, outcome-informed
   routing, budget threshold alerts, TOTP 2FA (full list: `MEMORY.md`, Known Limitations
   item 0). None of these being absent breaks anything a beta user would notice, but a
   beta is exactly the moment to decide which of them are worth finishing before launch,
   informed by what real usage patterns actually look like.
3. **No customer has ever seen the dashboard with their own real data in it.** Every
   screenshot and every verification so far used fixture data or a mock provider. A beta
   user's first login is the first real test of the whole loop: signup → key → request →
   savings appearing correctly → invoice line items matching.

## Prerequisites — hard gate

Do not invite anyone until every one of these is true. Inviting a real person before this
list is done converts their trust into the thing you're testing, which is not a fair trade.

- [ ] Docker Desktop working on a machine you can deploy from (`docker version` succeeds —
      see `MEMORY.md` Blockers; the last check found only leftover installer files, no
      working daemon).
- [ ] `docker compose -f infra/docker-compose.yml up -d` brings up Postgres, Redis, Qdrant
      healthy.
- [ ] At least one real provider key configured (`SHARED_GOOGLE_KEYS` for Gemini, per the
      user's earlier choice, or a customer BYOK key for testing).
- [ ] `cargo test --tests` run with `AEGIS_TEST_DATABASE_URL` set — the integration suite
      that only runs against a real database, never yet executed.
- [ ] One real end-to-end request served and its savings figure checked by hand against
      the provider's own dashboard billing for that call.
- [ ] `scripts/backup-restore-drill.sh` run at least once. A beta with unrestorable data
      is a beta that can lose a real person's account.
- [ ] Free-tier pooled key budget set deliberately (`AEGIS_FREE_TIER_MONTHLY_REQUESTS`) —
      low enough that 5–10 enthusiastic beta users cannot produce a surprise bill.

## Who to invite, and how many

**5 to 10 people. Not more.** The goal is depth of feedback per person, not statistical
coverage — that comes later, if at all, from the public launch. Prioritise:

- 2–3 people who already spend real money on OpenAI/Anthropic/Gemini API calls and would
  feel a genuine saving, not people doing this as a favour.
- 1–2 people technical enough to read the `X-Aegis-*` response headers and question a
  number that looks wrong — they will find bugs faster than anyone just reading the
  dashboard.
- At least 1 person setting up BYOK with their own provider key, since that path (encrypt,
  store, decrypt, use, never display again) has real security weight and deserves a real
  human clicking through it, not just a test asserting the ciphertext round-trips.

Do not invite anyone who cannot tolerate the product breaking. Say so explicitly when
inviting: "this is pre-launch, expect rough edges, tell me the moment something looks
wrong rather than working around it."

## What to watch during the beta window (aim for 1–2 weeks)

| Signal | Where | What a problem looks like |
|---|---|---|
| Savings arithmetic | Beta user's own provider billing dashboard, checked against ours | Any discrepancy, however small — this is the whole product's credibility |
| Gateway overhead | `/metrics`, `X-Aegis-Latency` header | Materially above the in-process measurement (P99 1.27ms on a debug build) once real network + database latency is in the loop |
| Routing decisions | `X-Aegis-Routing` header, `/requests` log | A complex or tool-using request ever downgraded — this must never happen; treat one occurrence as a stop-ship bug |
| Cache behaviour | `X-Aegis-Cache` header | Exact-cache hits work as expected. Remember semantic cache is not wired yet — a user asking "why didn't my rephrased question hit cache" has found a real, known gap, not a bug to chase |
| Budget/rate limits | Dashboard `/budgets` page | Hard limits actually block; soft limits actually record without blocking |
| Metering completeness | `workers/reconciliation.rs` output | Any gap between the Redis stream and `usage_records` — an unbilled request |
| Free-tier burn | `/api/admin/metrics` | Confirm the cap you set is actually enforced under real concurrent beta traffic |

## Feedback loop

- One shared channel (a chat group, not a form) — beta feedback dies in a form nobody
  reopens.
- Respond to every report within the same day. The point of a small beta is that this is
  achievable; if it isn't, the beta is already too big.
- Keep a running list in `MEMORY.md` under a `## Beta Findings` heading (add it when the
  first finding lands) so a finding is never only in a chat scrollback.
- Any finding that touches the savings arithmetic or tenant isolation is a same-day fix,
  not a backlog item — those two are the properties `CLAUDE.md` calls company-ending if
  they break.

## Exit criteria — when the beta is over

Move to `docs/runbooks/launch.md`'s pre-launch gate once:

- Every beta user has completed signup → BYOK or free tier → at least 10 real requests →
  checked their own savings figure by hand, and confirmed it is correct.
- No open finding touches money correctness or tenant isolation.
- A decision has been made, explicitly, on each of the four "built but not wired" items —
  ship without them, or finish wiring the ones beta users actually asked about.
- The prerequisites list above still holds (re-verify — a beta week is exactly when a
  provider key expires or a database gets reset without anyone noticing).

Write the retrospective into `MEMORY.md`: what broke, what the actual free-tier cost per
active beta user turned out to be, and which of the four unwired features beta users
actually noticed the absence of. That last one is the real signal for what to prioritise
next — it is worth more than a guess.

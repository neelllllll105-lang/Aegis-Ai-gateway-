# CLAUDE.md — Behavioral Contract for AI Agents on Aegis

> This file is loaded automatically by Claude Code (and readable by any other agent) at
> the start of every session in this repository. It is the entry point for resuming work.

## STOP — Read these four files before writing any code

Read them **in this order**, every session, no exceptions:

1. **`MEMORY.md`** — the living project state. Where we are, what is done, what is next,
   what is broken, what was decided and why. **This is the most important file.**
2. **`MASTER_BUILD.md`** — the immutable product blueprint (principles, stack, schema,
   pipeline, API, phases). The source of truth for *what* to build.
3. **`docs/PHASES.md`** — the detailed task list and acceptance criteria per phase.
4. **`docs/adr/README.md`** — the index of architecture decisions, including every
   deviation from `MASTER_BUILD.md` and the reason for it.

Then run `bash scripts/status.sh` (or `pwsh scripts/status.ps1`) for a machine-generated
snapshot: current phase, test counts, build state, next task.

## The one rule that keeps this project resumable

**Every session must end with `MEMORY.md` and `.aegis/state.json` updated.**

If you write code and do not update the memory, the next agent starts blind and the work
is effectively lost. Updating memory is not documentation overhead — it *is* the handoff
protocol. Treat an out-of-date `MEMORY.md` as a build failure.

Run this before you finish, always:

```bash
bash scripts/update-memory.sh
```

It refreshes the generated sections (file counts, test counts, git log, phase status).
The hand-written sections — "Current Focus", "Known Issues", "Next Steps", "Gotchas" —
you must edit yourself. The script tells you which ones are stale.

## Working agreement

1. **Phases are strictly ordered.** Never start Phase N+1 while Phase N acceptance
   criteria are unmet. If blocked, write the blocker into `MEMORY.md` under "Blockers"
   and report it — do not silently skip.
2. **Every module ships with tests in the same commit.** Untested code is unfinished code.
3. **Deviating from `MASTER_BUILD.md` requires an ADR.** Write it in `docs/adr/`, add it
   to the ADR index, and note it in `MEMORY.md`. Never deviate silently.
4. **Never introduce a technology outside Part 2 of `MASTER_BUILD.md`** without an ADR.
5. **Money is integers.** Micro-cents (`i64`), always. Floating point never touches a
   stored monetary value. See `apps/gateway/src/metering/pricing.rs`.
6. **Every DB query is scoped by `org_id`.** No exceptions. This is the tenant-isolation
   boundary; a leak here is company-ending.
7. **No secrets in logs, ever.** The redaction layer in `telemetry.rs` is tested — keep it
   that way.
8. **When in doubt, be conservative:** route to the requested model, keep the data, log
   the decision.

## Commit conventions

Conventional commits, referencing the phase task:

```
feat(gateway): add sliding-window rate limiter [P2.2]
fix(metering): floor gross_savings at zero on cache miss [P3.8]
docs(memory): update state after Phase 3 [P3]
```

Commit at every meaningful checkpoint, not just at the end. Small commits make the history
a usable log of *how* the system got here — which is itself part of the handoff.

## Local development

```bash
docker compose -f infra/docker-compose.yml up -d   # postgres, redis, qdrant
cd apps/gateway && cargo run                       # gateway on :8080
cd apps/web && npm run dev                         # dashboard on :3000
```

Without Docker you can still build and run the full unit test suite — the gateway
compiles and tests without a database. See `docs/HANDOFF.md` -> "Working without Docker".

## Verification commands (run before declaring anything done)

```bash
cd apps/gateway && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd apps/web && npm run lint && npm run build
bash scripts/verify-phase.sh <N>
```

## What "done" means for a phase

A phase is done when `scripts/verify-phase.sh <N>` passes, its acceptance criteria in
`docs/PHASES.md` are checked off with evidence, `MEMORY.md` records it, and
`.aegis/state.json` marks it `complete`. Nothing less.

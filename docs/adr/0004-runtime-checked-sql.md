# ADR-004: Runtime-checked SQL instead of SQLx compile-time macros

- **Status:** Accepted
- **Date:** 2026-08-20
- **Deviates from:** `MASTER_BUILD.md` Part 2 ("Compile-time checked SQL") and Part 12
  ("SQLx compile-time queries")

## Context

Part 2 selects SQLx partly for its compile-time checked queries, and Part 12 mandates
their use. The `query!` and `query_as!` macros connect to a live PostgreSQL instance at
**compile time**, verify the SQL against the real schema, and generate correctly typed
result structs. When it works, it catches an entire class of bug — a renamed column, a
type mismatch, a typo in a WHERE clause — before the code ever runs.

The cost is that compilation itself becomes dependent on a database. Concretely:

- `cargo build` fails with no `DATABASE_URL`, so a new contributor cannot compile the
  project until they have installed Docker, started Postgres, and run migrations.
- `cargo test` fails for the same reason, including for the several hundred tests that
  touch no database at all.
- CI needs a database service before the *formatting* check can run.
- The offline alternative, a committed `.sqlx` cache, must be regenerated whenever a query
  changes, and a stale cache produces confusing failures that look like schema drift.

This project has an explicit, unusual requirement that makes those costs heavier than
normal: **any engineer or AI agent must be able to pick the repository up and continue**.
The whole handoff mechanism — `MEMORY.md`, `CLAUDE.md`, `scripts/status.sh` — is built
around someone arriving cold. A first step of "install Docker, start Postgres, run
migrations, and only then can you compile anything" is a meaningful barrier, and during
this build Docker was in fact unavailable, which would have blocked all work.

## Decision

Use `sqlx::query` and `sqlx::query_as::<_, T>` with `#[derive(FromRow)]` structs, and
runtime column mapping.

Crucially, this preserves the property that actually matters for security: **queries are
still prepared statements with bound parameters.** Part 9 item 8 forbids string-built SQL,
and nothing here relaxes that. No user input is ever concatenated into a query.

What is given up is compile-time verification that a column name and type match the
schema. That gap is covered by:

1. **Integration tests against a real database** in `apps/gateway/tests/`, gated behind
   `AEGIS_TEST_DATABASE_URL`. They run in CI against a Postgres service container and
   exercise every repository function, so a schema mismatch fails the build — just at test
   time rather than compile time.
2. **A structural test** (`repo::tests::every_tenant_scoped_query_names_org_id`) that reads
   this module's own source and asserts every org-scoped function actually filters on
   `org_id`. That catches the specific bug class the compile-time macros would *not* have
   caught anyway: a query that is perfectly valid SQL but forgets the tenant boundary.

## Consequences

**Cost.** A renamed column now fails in CI rather than in the editor. The feedback loop is
minutes instead of seconds, and a contributor who does not run the integration tests
locally will not see the failure until they push.

**Benefit.** `cargo build`, `cargo test`, `cargo clippy`, and `cargo fmt` all work on a
fresh clone with nothing installed but Rust. 622 unit tests run in under four seconds with
no external service. This is what makes the project genuinely resumable.

**Mitigation, deliberately not taken yet.** `cargo sqlx prepare` would produce a `.sqlx`
offline cache giving compile-time checking without a live database. It was not adopted
because the cache must be regenerated on every query change and committed, and a
contributor who forgets gets an error that looks like schema drift rather than a stale
cache. That trade may be worth revisiting once the schema stabilises and the team is
larger than one.

## When to revisit

- The schema has stabilised and query changes have become rare.
- More than three engineers are working on the repository, making a shared `.sqlx` cache
  practical to maintain.
- A production incident is traced to a column mismatch that compile-time checking would
  have caught. If that happens, this decision was wrong and should be reversed.

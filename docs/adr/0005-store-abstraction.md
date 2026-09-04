# ADR-005: Key-value store behind a trait

- **Status:** Accepted
- **Date:** 2026-08-20
- **Extends:** `MASTER_BUILD.md` Part 2 (Redis remains the production store)

## Context

Redis carries the hot path: authentication caching, rate limiting, budget counters, the
exact-match cache, and the usage event stream. Principle 1 forbids synchronous Postgres I/O
on the request path, so there is no alternative to a fast store in front.

Testing that directly is awkward. Rate limiting, budget enforcement, cache behaviour, and
usage emission are among the most correctness-critical logic in the product, and requiring
a running Redis to test any of it means either every contributor installs Redis or the
logic goes untested.

## Decision

Define a `KvStore` trait covering the operations the hot path needs, with two
implementations:

- `RedisStore` — production. Atomic Lua for the sliding window, streams for usage events.
- `MemoryStore` — development and tests. Identical semantics, single process only.

`Config::validate` **refuses to start** a staging or production environment without
`REDIS_URL`, so the in-memory implementation cannot reach production by accident. This
matters more than it might appear: with two replicas each keeping their own counters, a
60/minute rate limit silently becomes 120/minute, and nothing surfaces the error.

## Consequences

**Cost.** Two implementations to keep in agreement. A behavioural difference between them
produces tests that pass and production that does not — the classic failure mode of a test
double. The mitigation is that the trait surface is small and semantically precise, and the
tests assert *semantics* (a sliding window that actually slides, atomicity under
concurrency) rather than implementation details.

**Benefit.** The entire pipeline is testable with no infrastructure. The concurrency test in
`store.rs` — 50 racing callers against a limit of 10, exactly 10 admitted — runs on a fresh
clone in milliseconds and would be much harder to write against a live server.

**Unexpected benefit.** It also makes the gateway genuinely runnable for local development
with zero setup, which turned out to matter when Docker was unavailable during the initial
build.

## When to revisit

If the two implementations drift far enough that a bug appears in production which the
memory store could not reproduce. At that point, run the same test suite against both
implementations in CI rather than deleting the abstraction.

# ADR-001: Rust and Axum for the gateway

- **Status:** Accepted
- **Date:** 2026-08-20
- **Confirms:** `MASTER_BUILD.md` Part 2

## Context

The gateway sits in the request path of every customer AI call. Principle 1 sets a hard
budget: under 1ms of added P99 latency. That number is not arbitrary — it is what lets us
claim the gateway is free in latency terms, and it is the specific thing LiteLLM (10-50ms,
Python) cannot match.

Three properties follow from being in the hot path of somebody else's production traffic:

1. **No GC pauses.** A garbage-collected runtime will occasionally add tens of
   milliseconds to a request. Averaged out that looks fine; at P99 it is the whole budget.
2. **Predictable memory.** A gateway holding thousands of concurrent streaming connections
   must not have memory growth the operator cannot reason about.
3. **A single deployable artifact.** Part 10 targets a $50/month single server. A runtime,
   a package manager, and a dependency tree at deploy time all work against that.

## Decision

Rust with Axum, Tokio, and reqwest.

Axum specifically, over actix-web or hyper directly: it is built on hyper and tower, so the
middleware ecosystem is shared with the rest of the Rust HTTP world, and its extractor
model keeps handlers readable. Handler signatures state exactly what they need, which
matters in a codebase where a reviewer must be able to see at a glance that a handler
takes an authenticated context.

## Consequences

**Cost.** Rust is slower to write than Python or TypeScript, and the pool of engineers who
can be productive in it immediately is smaller. Async Rust in particular has sharp edges
around lifetimes in streaming code — the streaming handler in `routes/openai_compat.rs` is
the most intricate code in the project for exactly that reason.

**Benefit.** Measured per-stage overhead well inside the budget, no GC, a single small
container, and a type system that makes the money and tenancy invariants enforceable at
compile time rather than by convention. `MicroCents` cannot be accidentally added to a
token count.

**What this makes harder.** Rapid prototyping of the control plane. That is precisely why
the dashboard is Next.js rather than a Rust template engine — the two halves have genuinely
different constraints, and using one language for both would compromise one of them.

## When to revisit

If the sub-1ms claim is dropped as a differentiator, most of this reasoning evaporates and
a Go gateway would be materially cheaper to staff.

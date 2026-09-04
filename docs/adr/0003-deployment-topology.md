# ADR-003: Single-server Hetzner topology

- **Status:** Accepted
- **Date:** 2026-08-20
- **Confirms:** `MASTER_BUILD.md` Part 10

## Context

Principle 6 sets an infrastructure budget of under $50/month for the first 1,000 users.
That rules out managed AWS or GCP services, whose per-service baseline costs alone exceed
the entire budget before a single request is served.

The competing pressure is Principle 5: decisions must survive 1M+ users without a rewrite.

## Decision

A single Hetzner CPX31 running Coolify, hosting the gateway, dashboard, Postgres, Redis,
Qdrant, and the observability stack. Cloudflare free tier in front for DNS, TLS, CDN, and
DDoS protection.

The two principles are reconciled not by over-building now, but by ensuring nothing in the
*architecture* assumes a single machine:

- The gateway is stateless. All state is in Redis or Postgres, so a second replica is a
  configuration change rather than a refactor.
- Rate limits and budgets are enforced through Redis, not process memory, so they stay
  correct across replicas. `Config::validate` refuses to start a production environment on
  the in-memory store precisely to stop somebody accidentally deploying a topology where
  limits silently stop working.
- Circuit breakers are deliberately per-process (see the note in `engine/fallback.rs`),
  because a breaker is a local judgement about what this instance is observing.

## Consequences

**Cost.** A single machine is a single point of failure. A disk failure or a bad kernel
upgrade is a full outage, and the recovery path is a restore from backup — which is why
Part 13 item 6 requires the restore to be *tested*, not merely configured.

**Benefit.** Roughly $40/month all-in, and an operational model one person can hold in
their head. No control-plane bills, no per-gigabyte egress surprises, no cloud-specific
services to unpick later.

**The scaling path is pre-decided.** Part 10 lists the trigger for each next step. The
discipline this record encodes is that the steps are taken *in order and only on the
trigger* — adding Kubernetes at 100 RPS because it feels more professional is how the
budget goes from $40 to $400 for no user-visible benefit.

## When to revisit

Any Part 10 trigger firing: sustained CPU over 60%, P99 overhead over 3ms, Redis over
512MB, or a customer contract requiring a multi-region SLA.

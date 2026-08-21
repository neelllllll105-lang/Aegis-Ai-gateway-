# Changelog

Notable changes to Aegis. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

**What "breaking" means here.** The gateway is a drop-in replacement for provider APIs, so
the compatibility surface is wider than a normal library's. A change is breaking if it
alters:

- the shape of any `/v1/*` request or response,
- the meaning of any `X-Aegis-*` header,
- the semantics of a routing decision (which model serves a given request),
- how a cost or fee is computed.

That last one is why routing and pricing changes are versioned as carefully as API changes.
A customer's bill changing because we shipped something is a breaking change even though no
schema moved.

---

## [Unreleased]

### Added

- **Anthropic-format streaming** on `POST /v1/messages` — the full named-event sequence
  (`message_start` → `content_block_*` → `message_delta` → `message_stop`), so the official
  Anthropic SDKs work against the gateway rather than failing on an unsupported stream.
- **SCIM 2.0 and SSO endpoints** wired and reachable (`/scim/v2/*`, `/api/sso/*`).
  Deprovisioning a user revokes every API key they created.
- **Spend anomaly detection** (`GET /api/usage/anomalies`) — flags spend that is within
  budget but statistically unlike the organisation's own history.
- **Cost-center chargeback** in JSON and CSV (`GET /api/usage/chargeback[.csv]`), attributed
  by team.
- **Referral credits** — two-sided, $10 each, with self-referral and double-claim guards.
- **Regional budgets** — a fourth budget scope between team and organisation, so one region
  cannot consume a multi-region organisation's entire monthly allowance.
- **Read replica support** (`DATABASE_REPLICA_URL`) — analytics queries move off the pool
  serving request authentication.
- **Scheduler** for periodic jobs, with a distributed claim so N replicas send exactly one
  weekly digest rather than N.
- **Nightly pricing drift check** — reports divergence between the live pricing table and
  the database. Deliberately does not auto-apply: a price is what an invoice is computed
  from.
- **Dashboard pages**: models catalogue, routing policies, budgets, people & teams,
  billing & chargeback.
- **Self-hosted stack** — `infra/docker-compose.self-hosted.yml` plus a web image, for
  deployments where data may not leave the customer's network.
- **Compliance pack** — security whitepaper, subprocessor list, DPA template, data-flow
  document, and an honest SOC 2 gap analysis.

### Fixed

- **Dashboard rendered against nine design tokens that did not exist.** A palette rename
  left `var(--color-base)` and friends dangling. CSS custom properties fail silently, so
  `tsc`, `eslint` and `next build` all passed while pages rendered with transparent
  backgrounds. `npm run lint` now fails on any dangling token.
- **Authenticated pages were indexable** and inherited the marketing site's `<title>`,
  because the dashboard layout was a client component and those cannot export metadata.
  Dashboard is now `noindex, nofollow`; each page has its own title.
- **Unknown API keys returned 500 instead of 401** when running without a database.
- Two accent colours were in use across the product; folded into one.

### Changed

- Router assembly moved from `main.rs` into the library so integration tests can reach it.
  `tests/route_surface.rs` now asserts every advertised route is wired, that an unwired
  path really does 404, and that every tenant-data route refuses anonymous callers.

---

## [0.1.0] — 2026-08-20

First complete build. Phases 0–7 of `MASTER_BUILD.md`.

### Added

- **Gateway**: Rust + Axum, OpenAI- and Anthropic-compatible surfaces, 12-stage request
  pipeline, sub-millisecond self-overhead target with the measurement published rather than
  gamed.
- **Routing**: complexity classification (heuristic V1, sign-constrained linear V2), policy
  overrides, UCB1 bandit for outcome-driven selection, per-provider circuit breakers.
- **Metering**: integer micro-cent arithmetic end to end — floating point never touches a
  stored monetary value. Savings floored at zero; fees as basis points.
- **Caching**: exact and semantic, fingerprinted with `org_id` so cross-tenant collision is
  impossible.
- **Tenant isolation**: every tenant-scoped query takes `org_id`, enforced by a test that
  reads the repository's own source.
- **Security**: AES-256-GCM credential encryption under per-tenant derived keys, argon2id
  passwords, SHA-256 key hashes with constant-time comparison, TOTP, and a credential
  redaction layer with a startup self-check.
- **Enterprise**: SSO, SCIM, offline licence validation that degrades to a warning rather
  than an outage, data residency enforcement.
- **Dashboard and marketing site**: Next.js 15, React 19, Tailwind 4.
- **Handoff mechanism**: `MEMORY.md`, `CLAUDE.md`, `.aegis/state.json`, status and
  verification scripts, and a CI gate that fails the build when memory goes stale.

### Known limitations at 0.1.0

Recorded here rather than discovered later:

- No third-party security attestation.
- Load test and restore drill written but never executed.
- SSO and SCIM verified against their specifications, not against a real identity provider.
- Stripe checkout round trip not exercised against Stripe.

---

## How to file feedback

- **Bug or regression**: open an issue with the request ID from the `X-Aegis-Request-Id`
  response header. Every request carries one, and it is the fastest path from a report to
  the exact routing decision that produced it.
- **Routing quality**: if Aegis served a cheaper model and the answer was worse, that is the
  single most valuable report we can receive. Include the request ID and what you expected.
  Routing that quietly degrades quality is the failure mode this product cannot afford.
- **Pricing accuracy**: prices carry a source and a check date. If one is wrong, say which
  model and link the provider page.
- **Security**: `security@aegis.dev`, not a public issue.

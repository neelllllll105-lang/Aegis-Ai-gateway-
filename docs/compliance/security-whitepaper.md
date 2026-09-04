# Aegis security whitepaper

**Version 1.0 · Status: pre-certification**

Every control described here is implemented and covered by a test unless the section says
otherwise. Sections marked **Designed, not proven** describe intent; do not present them to
a customer as controls.

---

## 1. What Aegis is, in security terms

Aegis sits between a customer's application and their LLM providers. A request arrives with
an Aegis API key, is authenticated, checked against limits, possibly served from cache,
routed to a model, forwarded to a provider, and returned. That places Aegis in the data
path for prompts and completions — the most sensitive data most customers have.

The design consequence: **the safest thing Aegis can do with content is not keep it.** By
default it does not.

## 2. Data classification

| Class | Examples | Stored? | Encrypted at rest |
|---|---|---|---|
| Content | Prompts, completions, embeddings input | **Yes, in the exact-match cache, by default** — see below | **No** |
| Credentials | Provider API keys | Yes | AES-256-GCM, per-tenant key |
| Authenticators | Aegis API keys, passwords, TOTP secrets | Hash only | SHA-256 (keys), argon2id (passwords) |
| Metadata | Token counts, costs, model names, latency, routing reasons | Yes | Database-level |
| Identity | Email, name, org membership | Yes | Database-level |

Token counts and costs are metadata, not content. A usage record can tell you a request
used 4,120 input tokens and cost $0.031; it cannot tell you what was asked.

**Correction — an earlier version of this document claimed `content_capture` gates
whether content is stored. That was wrong, and was caught by an internal audit rather
than by a customer, which is not good enough but is the honest sequence of events.** The
schema has two flags: `zero_retention` and `content_capture`. Only `zero_retention` gates
anything at runtime — `cache/fingerprint.rs::cacheability()` checks it, and it alone,
before allowing a request into the exact-match cache. `content_capture` is read and
written by the API but consulted by no code path; the encrypted, per-tenant-keyed
`captured_content` table its own migration comment describes is never written to by
anything in the codebase today. Concretely:

- The **exact-match cache** stores the full prompt and completion as **plaintext JSON in
  Redis** (`cache/exact.rs`), with no application-level encryption — unlike provider
  credentials, which are AES-256-GCM encrypted before they ever reach storage. This is
  **on by default** for every organisation, for up to the configured TTL (24 hours by
  default), for the sole purpose of serving a byte-identical repeat request without
  paying a provider again.
- Setting `zero_retention: true` on an organisation is what actually removes content from
  this path — confirmed by `cache/fingerprint.rs`, not by the flag's name alone.
- `content_capture` should be treated as **not implemented** until this is corrected in
  code: either wire it to the table it was designed for, or remove the flag and the claim
  entirely rather than leave a control in the schema that does nothing.

Until the code is fixed, tell a customer the accurate thing: **content lives in Redis in
plaintext for any organisation that has not set `zero_retention`, for up to the cache TTL.
Set `zero_retention` if this is not acceptable.**

## 3. Authentication and authorisation

**API keys.** Generated with a CSPRNG using base62 rejection sampling — not modulo
reduction, which biases the output distribution (`crypto.rs::generate_api_key`). Stored as
a SHA-256 hash; the plaintext is displayed once at creation and is not recoverable. Lookup
is constant-time (`crypto.rs::secure_compare`), so response timing does not leak how much
of a guessed key was correct.

**Dashboard sessions.** HTTP-only, `SameSite`, `Secure` cookies. No token ever reaches
JavaScript, which removes session theft from the consequences of an XSS.

**Passwords.** argon2id with parameters set for interactive login. Never logged, never
returned.

**Two-factor.** TOTP (RFC 6238) with a replay window (`enterprise/totp.rs`).

**SSO.** OIDC and SAML assertion validation checks audience, issuer, expiry, and replay
(`enterprise/sso.rs`). **Designed, not proven:** never tested against a real identity
provider.

**SCIM.** RFC 7644 user provisioning and deprovisioning (`enterprise/scim.rs`,
`routes/enterprise.rs`). SCIM tokens live in their own table with their own bearer
namespace — a SCIM token can deprovision every member of an organisation, so it is
deliberately not an ordinary API key. Deprovisioning revokes every API key the removed user
created, because removing a membership while leaving their keys live is exactly the access
a departed employee should lose first.

## 4. Tenant isolation

This is the control that matters most, and it is enforced structurally.

- Every tenant-scoped repository function takes `org_id` and filters on it. There is no
  "get by id" that omits it.
- `db::repo::every_tenant_scoped_query_names_org_id` reads the repository's own source and
  fails if a tenant query is added without an `org_id` predicate. Convention is not relied
  on; the build enforces it.
- `apps/gateway/tests/tenant_isolation.rs` attempts real cross-tenant reads against a live
  database and requires each to fail.
- Cache fingerprints hash `org_id`, so two tenants sending byte-identical prompts get
  different cache keys. Cross-tenant cache collision is not possible even in principle.
- Rate limit, budget, and spend counters are all keyed by tenant. A regional spend counter
  keyed by region alone would aggregate every tenant in that region — that specific mistake
  has a test against it.

## 5. Credential handling

Provider keys are encrypted with AES-256-GCM. The encryption key is derived per tenant from
`AEGIS_MASTER_KEY` via HKDF, so compromising one tenant's derived key does not yield
another's. Ciphertext is stored in a column marked non-serialisable in Rust, meaning it
cannot be returned through an API response even by an accidental `SELECT *` reaching a JSON
encoder.

`AEGIS_MASTER_KEY` is supplied by the environment and never written to disk by the
application. Losing it renders every stored credential permanently unreadable. This is
intended: a recovery path for us is a recovery path for an attacker.

## 6. Logging and redaction

No secret is ever logged. This is not a coding guideline — it is a tested layer.
`telemetry.rs::redact` applies nine credential patterns to log output, and the application
runs a **self-check at startup** that asserts redaction actually works before serving any
traffic. If redaction is broken, the process says so immediately rather than quietly
writing keys into a log aggregator for a month.

Because of this, `RUST_LOG=debug` is safe to enable in production.

## 7. Transport and network

- TLS required in production; the configuration validator refuses to start a
  production-like environment with an `http://` app URL.
- HSTS, `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy`, and a
  restrictive `Permissions-Policy` on every response (`middleware/security_headers.rs`).
- CORS is scoped to the configured dashboard origin, with credentials allowed only there.
- Request bodies are capped before parsing, so an oversized body is rejected without being
  read into memory.

## 8. Availability and failure behaviour

The failure modes are chosen deliberately, and they are not all "fail closed":

- **Budget check fails open.** If Redis is unreachable, spend reads as zero and the request
  proceeds. During an outage of *our* infrastructure we would rather serve traffic and
  reconcile afterwards than reject paying customers. Exposure is bounded by outage duration
  and surfaced by the reconciliation worker.
- **Licence validation fails open.** An expired or unreachable licence degrades to a
  warning. A licensing problem must never be an outage.
- **Provider circuit breakers fail over.** Five consecutive failures open a provider's
  circuit for 30 seconds, then a single probe request tests recovery.
- **Routing fails conservative.** Any doubt routes to the model the caller asked for. Aegis
  never downgrades a request classified complex, and never downgrades a tool-using request.

## 9. Data residency

An organisation can pin itself to a region. Each gateway instance knows its own region and
refuses to serve an organisation pinned elsewhere (`enterprise/residency.rs`). Geographic
routing is a load-balancer concern; this module is the backstop that makes a misrouted
request fail loudly rather than quietly process data in the wrong jurisdiction.

## 10. Known gaps

Stated plainly, because a security reviewer will find these anyway and it is better they
find them here.

| Gap | Impact | Status |
|---|---|---|
| No SOC 2 / ISO 27001 attestation | Blocks regulated buyers | See `soc2-readiness.md` |
| No penetration test | Unknown unknowns | Not scheduled |
| SSO/SCIM never tested against a real IdP | Enterprise onboarding may fail | Shapes tested; integration unproven |
| Backups never restored | Restore path unproven | Runbook + script exist; drill not run |
| No formal incident response exercise | Response time unknown | Runbooks exist |
| Load test never executed | P99 overhead claim unverified at scale | Script committed |

## 11. Reporting a vulnerability

Email `security@aegis.dev`. We will acknowledge within two business days. Please do not
open a public issue.

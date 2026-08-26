# ADR-007: `jsonwebtoken` for Vertex AI service-account auth

- **Status:** Accepted
- **Date:** 2026-08-24
- **Deviates from `MASTER_BUILD.md`:** Yes — introduces a dependency outside Part 2's
  stack list. CLAUDE.md rule 4 requires this record.

## Context

Every existing provider authenticates with a single opaque string — an API key sent as a
bearer token or query parameter, checked synchronously in `Provider::auth_headers()`.
Vertex AI does not offer that surface for `generateContent`. Google's IAM model for
Vertex requires either a live gcloud session or a service-account **JSON key**, exchanged
for a short-lived OAuth2 access token by POSTing a self-signed JWT assertion to
`https://oauth2.googleapis.com/token`. That assertion must be RS256-signed with the
service account's RSA private key.

No crate already in this workspace can produce an RS256 signature — `sha2`, `hmac`, and
`aes-gcm` are all symmetric-key or hash primitives, and hand-rolling PKCS#1v1.5 RSA
signing is exactly the kind of security-sensitive code this project avoids writing itself
(the same reasoning ADR-002 applies to money: don't reimplement what a correct library
already does, when getting it subtly wrong is a real risk).

## Decision

Add `jsonwebtoken` (MIT licensed, so it clears the existing `cargo license` GPL/AGPL CI
gate without a policy change). It provides `EncodingKey::from_rsa_pem`, which consumes a
service account's PEM-formatted private key directly with no manual key parsing, and
`encode()` for RS256 signing.

The blast radius is deliberately contained to one module:

- `providers/vertex.rs` is the only file that imports `jsonwebtoken`.
- The shared `Provider` trait is **unchanged**. `auth_headers()` stays synchronous for
  every provider; Vertex overrides `chat()` and `chat_stream()` entirely instead of using
  the trait's default implementation, doing its async token acquisition internally before
  either ever calls into shared HTTP-execution code (`super::openai::open_stream` for
  streaming, matching how Google's adapter already borrows it).
- A customer's "API key" for the `vertex` provider is the full service-account JSON key
  file, pasted as text into the same `Credential.api_key` field every other provider
  already uses. No change to the credential storage schema, the encryption path, or the
  BYOK UI's data model — just a different provider choosing to interpret that opaque
  string as structured JSON instead of a bare key.

## Consequences

**Cost.** One more dependency to keep patched — covered by the existing `cargo audit` CI
gate, same as every other crate. Access tokens must be cached per service account
(~1 hour lifetime) or every single gateway request would pay for a token-exchange round
trip to Google first; this cache is new state (a `DashMap` on `VertexProvider`) that
didn't exist in any prior provider.

**What is not covered by this ADR.** Only Gemini models served through Vertex's
`publishers/google` namespace. Vertex's model garden also serves Anthropic, Llama, and
other third-party models, each with its own request/response schema quirks on that
surface — a materially different, larger scope, deliberately deferred rather than
attempted alongside this change.

**Unverified.** The JWT assertion's construction and signature are unit-tested directly
(decode what we produce, check every claim). The actual OAuth2 token exchange and the
live `generateContent` call have not been exercised against a real GCP project — this
project has never had a live credential for any provider (see `MEMORY.md` Blockers), and
Vertex is no exception. Treat it the same as every other "compiles and is tested against
a mock, never run for real" gap already tracked there.

## When to revisit

If Anthropic-on-Vertex or Llama-on-Vertex support is ever requested — that needs its own
scoping, not an extension of this one. Or if Google ships a simpler auth surface for
Vertex (API-key support has been discussed in their own release notes at various points);
if that ships, this entire token-exchange path could be deleted in favor of the same
`auth_headers()` pattern every other provider uses.

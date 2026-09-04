# Data flow

What data exists, where it goes, and how long it lives. Written for a DPIA or a security
review, so it follows one request end to end rather than describing components.

## A single request, stage by stage

```
Customer app
    │  POST /v1/chat/completions   { model, messages }
    │  Authorization: Bearer ak_live_...
    ▼
┌─────────────────────────────────────────────────────────────────┐
│ Cloudflare            TLS termination, WAF, DDoS                │
│                       Sees: IP, headers, encrypted body         │
│                       Stores: request metadata in edge logs     │
└─────────────────────────────────────────────────────────────────┘
    ▼
┌─────────────────────────────────────────────────────────────────┐
│ [1] Authenticate      SHA-256 the bearer token, look up the key │
│                       Touches: token hash. Never the plaintext. │
│ [2] Rate limit        Redis counter keyed by org + key          │
│ [3] Budget            Redis spend counters. Fails OPEN.         │
│ [4] Parse             Body → NormalizedRequest, in memory       │
│                       Content exists here. Not yet stored.      │
│ [5] Cache lookup      Fingerprint = hash(org_id + normalised    │
│                       content). Only if the org enabled caching.│
│ [6] Route             Classify complexity, pick a model.        │
│                       Reads content. Writes nothing.            │
│ [7] Provider call     Forward under the org's own key (BYOK)    │
│                       or a pooled key (free tier only)          │
└─────────────────────────────────────────────────────────────────┘
    ▼
┌─────────────────────────────────────────────────────────────────┐
│ LLM provider          Sees full prompt and returns completion.  │
│                       Under BYOK this is the customer's own     │
│                       contract; Aegis is not a party to it.     │
└─────────────────────────────────────────────────────────────────┘
    ▼
┌─────────────────────────────────────────────────────────────────┐
│ [8]  Normalise        Provider response → common shape          │
│ [9]  Cost & savings   Integer arithmetic on token counts        │
│ [10] Emit usage       Redis stream. Token counts, costs,        │
│                       model names, latency. NO CONTENT.         │
│ [11] Respond          Body returned to the caller               │
│ [12] Persist          Background writer: stream → Postgres      │
└─────────────────────────────────────────────────────────────────┘
    ▼
Customer app
```

The content of the request exists in gateway memory for the duration of the request and is
written to durable storage in exactly two circumstances, both opt-in: semantic caching, and
content capture.

## What is stored, and for how long

| Data | Store | Retention | Contains content? |
|---|---|---|---|
| Usage records | Postgres, partitioned monthly | 13 months | No |
| Daily aggregates | Postgres | Indefinite | No |
| Audit log | Postgres | 24 months | No |
| Spend/rate counters | Redis | 35 days (TTL) | No |
| Usage event stream | Redis | Capped length, minutes | No |
| Semantic cache entries | Qdrant + Redis | Org-configured TTL, default 24h | **Yes** — only if enabled |
| Captured content | Postgres | Org-configured, default **off** | **Yes** — only if enabled |
| Provider credentials | Postgres, AES-256-GCM | Until deleted | N/A |
| Account identity | Postgres | Until deletion request | No |
| Backups | Object storage, encrypted | 35 days | Mirrors the above |

## Personal data in request content

Aegis cannot know whether a prompt contains personal data — that is entirely determined by
what the customer's application sends. This is why the default is not to store it.

A customer processing personal data through Aegis should:

1. Leave `zero_retention` on (the default).
2. Leave semantic caching off, or accept that cache entries hold content for the configured
   TTL.
3. Use BYOK, so the provider relationship is under their own DPA.
4. Pin to a region if jurisdiction matters.

With those four settings, the only personal data Aegis durably stores is the account
identity of the people who administer the account.

## Deletion

**Organisation deletion** removes account records, credentials, usage records, audit
entries, and cache entries. Billing records survive for the statutory 7 years; they contain
invoice totals and no content.

**Cache invalidation** is immediate on request — cache entries are keyed by a fingerprint
including `org_id`, so an organisation's entries can be dropped by prefix without touching
anyone else's.

**Backups** are not selectively editable. Deleted data persists in backups until they
rotate out, at most 35 days. This is stated in the DPA rather than pretended away.

## Cross-border

Default processing region is EU (`eu-central`). An organisation pinned to a region is
refused by any gateway instance in a different one — enforced in
`enterprise/residency.rs`, not by configuration convention.

The one unavoidable cross-border element on the hosted service is Cloudflare's edge
network. An EU-resident configuration is available and is what an EU-pinned organisation
should be on.

Self-hosted deployments have no cross-border element at all beyond the provider endpoints
the customer chooses.

# Subprocessors

**Last reviewed: 2026-08-21**

A subprocessor is any third party that may process customer personal data on our behalf.
This list is an annex to the [DPA](dpa-template.md) and is what a customer's privacy team
will diff against their approved-vendor register.

Two categories, and the difference matters more than the list itself.

---

## Category A — Always in the data path

These process data for every customer on the hosted service. Removing one is not a
configuration change.

| Subprocessor | Purpose | Data processed | Location | DPA |
|---|---|---|---|---|
| Hetzner Online GmbH | Compute and managed Postgres/Redis hosting | All stored metadata, encrypted credentials | Germany (EU) | Standard DPA, EU SCCs not required (EU-to-EU) |
| Cloudflare, Inc. | CDN, WAF, DNS, TLS termination | Request metadata, IP addresses in transit | Global edge; EU-resident config available | DPA + EU SCCs |

## Category B — Customer-directed LLM providers

**These are the important ones, and their status is different from Category A.**

When a customer uses BYOK, they hold the contract with the provider directly. Aegis
forwards the request under the customer's own credential and never becomes a party to that
relationship. Aegis is not a subprocessor of that data — the customer's existing agreement
with the provider governs it.

When a customer uses Aegis pooled keys (free tier only), Aegis *is* forwarding under its own
account, and the provider is a genuine subprocessor for that traffic.

| Provider | Subprocessor when BYOK? | Subprocessor when pooled? | Retention (provider's policy) |
|---|---|---|---|
| OpenAI | No — customer's own contract | Yes | Zero-retention available on API tier |
| Anthropic | No | Yes | Zero-retention by default on API |
| Google (Gemini) | No | Yes | Varies by tier — check current terms |
| DeepSeek, Mistral, Groq, Moonshot, OpenRouter | No | Yes | Varies |

A customer who needs a short, closed subprocessor list should use BYOK. That is the honest
answer, and it is also the cheaper one for them.

## Category C — Operational, no content access

| Subprocessor | Purpose | Data processed | Location |
|---|---|---|---|
| Stripe, Inc. | Payment processing | Billing contact, payment method, invoice amounts. **No usage content.** | US/EU; DPA + SCCs |
| Resend | Transactional email (alerts, digests, invitations) | Email address, message body (spend figures only) | US; DPA + SCCs |

Stripe never receives usage data. It receives an invoice total and a customer record.

## Self-hosted deployments

A self-hosted installation has **no subprocessors from us at all.** Compute, database,
cache and vector store all run inside the customer's own boundary. The only external
network calls are to the LLM providers the customer configures, under the customer's own
credentials. For an organisation whose data may not leave its own network, this is the
deployment to buy.

## Change notification

Material additions to Category A or C are announced at least 30 days before they take
effect, via the address on file for the organisation and in `CHANGELOG.md`. A customer who
objects may terminate for convenience within that window without penalty.

Category B is not subject to this notice: the customer chooses which providers they route
to, and adding support for a provider does not send anyone's data to it.

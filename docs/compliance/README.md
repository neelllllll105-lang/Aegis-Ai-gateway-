# Compliance pack

> **Status: pre-certification.** Aegis holds no third-party audit attestation today. This
> pack describes controls that are **implemented and tested in code**, and separates them
> explicitly from controls that are **designed but unproven**. Sending a prospect a
> document that blurs the two is how a security review turns into a legal problem.
>
> Every claim below links to the code or test that backs it. If a claim has no link, it is
> marked as an intention, not a control.

## What is in this pack

| Document | Purpose | Audience |
|---|---|---|
| [`security-whitepaper.md`](security-whitepaper.md) | How the system protects data, and what it does not protect against | Security reviewer, CISO |
| [`subprocessors.md`](subprocessors.md) | Every third party that can touch customer data | Privacy/legal, DPA annex |
| [`dpa-template.md`](dpa-template.md) | Data processing agreement, GDPR Art. 28 | Legal |
| [`data-flow.md`](data-flow.md) | What data exists, where it goes, how long it lives | DPIA, security review |
| [`soc2-readiness.md`](soc2-readiness.md) | Honest gap analysis against SOC 2 Type II | Founders, auditor |

## The three claims a buyer actually cares about

**1. Aegis does not store prompts or completions by default.**

Implemented. `zero_retention` defaults on for every organisation, and content capture is a
separate opt-in flag. Usage records carry token counts, costs and routing decisions — never
message content. See `apps/gateway/src/metering/usage.rs` and the `content_capture` column
in `apps/gateway/migrations/0001_initial_schema.sql`.

The semantic cache is the one place content is retained, and only when a customer enables
it: cache entries are keyed by a fingerprint that includes `org_id`, so one tenant cannot
read another's cached response even given an identical prompt. See
`apps/gateway/src/cache/fingerprint.rs`.

**2. One customer cannot see another customer's data.**

Implemented and enforced structurally rather than by convention. Every tenant-scoped
repository function takes an `org_id` and includes it in the `WHERE` clause. A test reads
the repository's own source and fails if a tenant query is added without one
(`db::repo::every_tenant_scoped_query_names_org_id`). A second suite
(`apps/gateway/tests/tenant_isolation.rs`) attempts real cross-tenant reads against a live
database and requires them to fail.

**3. Provider credentials cannot be read back, by us or by anyone.**

Implemented. Keys are encrypted with AES-256-GCM under a key derived per tenant via HKDF
(`apps/gateway/src/crypto.rs`). The ciphertext column is marked non-serialisable, so it
cannot leak through an API response even by accident. The API returns a four-character
hint and nothing else. Losing `AEGIS_MASTER_KEY` makes every stored credential permanently
unreadable — that is the intended property, not a limitation.

## What we do not claim

- **No SOC 2, ISO 27001, or HIPAA attestation.** See [`soc2-readiness.md`](soc2-readiness.md)
  for the gap list. Anyone who needs an attestation today should not buy yet.
- **No penetration test has been performed.**
- **No formal incident response exercise has been run.** The runbooks exist
  (`docs/runbooks/`); the drills have not happened.
- **Backups have never been restored.** `docs/runbooks/restore.md` is written and scripted.
  Until the drill runs, treat the restore path as untested.

Keeping this list accurate is more valuable than shortening it. A buyer who discovers an
overstated control stops believing the accurate ones too.

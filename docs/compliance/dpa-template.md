# Data Processing Agreement (template)

> **This is a template, not legal advice, and it has not been reviewed by counsel.**
> Have a qualified lawyer review it before sending it to a customer or signing it. The
> technical annexes are accurate; the legal framing needs a professional.

This DPA forms part of the Agreement between **[CUSTOMER]** ("Controller") and
**[AEGIS ENTITY]** ("Processor") and reflects the parties' agreement on processing of
Personal Data under Regulation (EU) 2016/679 ("GDPR").

---

## 1. Definitions

Terms defined in the GDPR — Personal Data, Processing, Controller, Processor, Data
Subject, Supervisory Authority, Personal Data Breach — carry those meanings here.

"Services" means the Aegis AI gateway as described in the Agreement.

## 2. Roles

The Controller determines the purposes and means of Processing. The Processor Processes
Personal Data only on the Controller's documented instructions, of which this DPA and the
Agreement are the complete set.

## 3. Subject matter and duration

**Subject matter.** Routing, metering, and cost optimisation of the Controller's API
requests to third-party language model providers.

**Duration.** For the term of the Agreement, plus the retention period in Section 8.

**Nature and purpose.** Authentication, rate limiting, budget enforcement, model routing,
usage metering, billing, and — only where the Controller enables it — semantic caching.

**Categories of Data Subject.** The Controller's personnel who administer the account, and
any Data Subject whose Personal Data the Controller includes in a request payload.

**Categories of Personal Data.**

*Always processed:*
- Account identity: name, email address, organisation membership, role
- Technical: IP address, user agent, timestamps
- Usage metadata: token counts, costs, model identifiers, latency, routing decisions

*Processed only where the Controller enables content capture or semantic caching:*
- Request and response content, which may contain any Personal Data the Controller places
  in it

**By default, the Processor does not store request or response content.** Content capture
and semantic caching are per-organisation opt-in settings, off unless the Controller turns
them on. See Annex II.

## 4. Processor obligations

The Processor shall:

(a) Process Personal Data only on documented instructions, including for international
transfers, unless required by Union or Member State law — in which case it will inform the
Controller before Processing, unless that law prohibits such notice;

(b) ensure that persons authorised to Process Personal Data are bound by confidentiality;

(c) implement the technical and organisational measures in Annex II;

(d) respect the conditions in Section 5 for engaging Sub-processors;

(e) assist the Controller, by appropriate technical and organisational measures, in
fulfilling its obligation to respond to Data Subject requests;

(f) assist the Controller with Articles 32–36 GDPR, taking into account the nature of
Processing and the information available to it;

(g) at the Controller's election, delete or return all Personal Data at the end of the
Services, and delete existing copies unless law requires storage;

(h) make available all information necessary to demonstrate compliance with Article 28 and
allow for and contribute to audits under Section 7.

## 5. Sub-processors

The Controller grants general authorisation to engage the Sub-processors listed in
[`subprocessors.md`](subprocessors.md).

The Processor shall give **30 days' notice** before adding or replacing a Sub-processor
that has access to Personal Data. The Controller may object on reasonable data-protection
grounds within that period; if the parties cannot resolve the objection, the Controller may
terminate the affected Services without penalty.

Where the Controller uses its own provider credentials ("BYOK"), the language model
provider is **not** a Sub-processor of the Processor: the Controller contracts with that
provider directly and the Processor merely transmits the request under the Controller's
credential.

## 6. International transfers

Personal Data is Processed in the EEA by default. Where a Sub-processor Processes Personal
Data outside the EEA, transfers are made under the Standard Contractual Clauses
(Commission Implementing Decision (EU) 2021/914), Module Two (Controller to Processor) or
Module Three (Processor to Processor) as applicable, incorporated by reference.

An organisation configured for EU residency has its requests refused by any gateway
instance outside its pinned region, enforced in code rather than by policy.

## 7. Audit

The Processor shall make available the information necessary to demonstrate compliance,
including the current version of [`security-whitepaper.md`](security-whitepaper.md) and,
once obtained, any third-party audit report.

The Controller may conduct an audit no more than once per twelve months, on 30 days'
notice, during business hours, subject to confidentiality, and at its own cost — except
where an audit reveals material non-compliance, in which case the Processor bears
reasonable costs.

> **Disclosure.** As of the date of this template the Processor holds **no third-party
> audit attestation** (no SOC 2, no ISO 27001). A Controller that requires one should not
> rely on this section as a substitute.

## 8. Deletion and return

On termination, or on the Controller's written request, the Processor shall delete all
Personal Data within **30 days**, except:

- Billing records, retained for **7 years** where required by tax law;
- Aggregated usage statistics containing no Personal Data and no content;
- Backups, purged on their normal rotation, not exceeding **35 days**.

Content, where captured, is deleted according to the Controller's configured retention
period, which defaults to **zero**.

## 9. Personal Data Breach

The Processor shall notify the Controller **without undue delay and in any event within 48
hours** of becoming aware of a Personal Data Breach affecting the Controller's Personal
Data, providing the nature of the breach, categories and approximate number of Data
Subjects and records affected, likely consequences, and measures taken or proposed.

## 10. Liability

Liability under this DPA is subject to the limitations in the Agreement, except where the
GDPR prohibits such limitation.

---

## Annex I — Details of Processing

As set out in Section 3.

**Controller contact:** [NAME, EMAIL]
**Processor contact:** privacy@aegis.dev

## Annex II — Technical and organisational measures

Measures are implemented and covered by automated tests unless marked otherwise. Full
detail in [`security-whitepaper.md`](security-whitepaper.md).

**Pseudonymisation and encryption (Art. 32(1)(a))**
- Provider credentials encrypted at rest with AES-256-GCM under per-tenant keys derived via
  HKDF; ciphertext is non-serialisable and cannot be returned by any API.
- API keys stored as SHA-256 hashes; plaintext shown once, never recoverable.
- Passwords hashed with argon2id.
- TLS 1.2+ required for all data in transit; production configuration refuses to start
  without it.

**Confidentiality, integrity, availability, resilience (Art. 32(1)(b))**
- Tenant isolation enforced structurally: every tenant-scoped query includes `org_id`, and
  a build-time test fails if one is added without it.
- Cache fingerprints hash `org_id`, making cross-tenant cache collision impossible.
- Credential redaction in logs is a tested layer with a startup self-check.
- Per-tenant rate limiting and budget enforcement.
- Circuit breakers isolate failing providers.

**Restoring availability after an incident (Art. 32(1)(c))**
- Automated daily database backups with 35-day retention.
- Documented restore runbook (`docs/runbooks/restore.md`) with a scripted drill.
- > **Disclosure:** the restore drill has not yet been executed. Until it has, the restore
  > path should be treated as untested.

**Testing and evaluation (Art. 32(1)(d))**
- 696 automated tests run on every commit, including tenant isolation, money arithmetic,
  redaction, and rate limiting under concurrency.
- CI enforces formatting, linting with warnings as errors, and the full test suite.
- > **Disclosure:** no third-party penetration test has been performed.

**Data minimisation**
- Zero content retention by default. Usage records carry token counts and costs, never
  message content.
- Content capture is a per-organisation opt-in, off unless deliberately enabled.

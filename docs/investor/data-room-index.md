# Data room index

What exists, where it is, and — the part most indexes omit — what does not exist yet.
An investor finds the gaps regardless; finding them listed by us is a different
conversation from finding them by accident.

**Prepared: 2026-08-21 · Stage: pre-seed, pre-revenue**

---

## 1. Company

| Document | Status | Location |
|---|---|---|
| Certificate of incorporation | ❌ Not incorporated | — |
| Cap table | ❌ | — |
| Founder agreements, vesting | ❌ | — |
| IP assignment agreements | ❌ | — |
| Board consents / minutes | ❌ No board | — |

**Everything in this section is outstanding.** Incorporation and IP assignment should
precede any external fundraising conversation; a term sheet cannot close without them, and
IP assignment in particular gets more awkward the longer it waits.

## 2. Product and technology

| Document | Status | Location |
|---|---|---|
| Product specification | ✅ Complete | `MASTER_BUILD.md` |
| Architecture decision records | ✅ 6 ADRs, incl. deviations and their reasons | `docs/adr/` |
| Phase plan with acceptance evidence | ✅ Evidence marked ✅/⚠️/❌ per criterion | `docs/PHASES.md` |
| Engineering handoff | ✅ Cold-start onboarding | `docs/HANDOFF.md`, `CLAUDE.md` |
| Live project state | ✅ Continuously maintained | `MEMORY.md` |
| API documentation | ✅ | `docs/API.md`, `/docs` on the marketing site |
| Runbooks | ✅ Deploy, restore, pricing update, launch | `docs/runbooks/` |
| Source code | ✅ ~700 tests, CI-gated | This repository |

This section is the strongest part of the data room, and unusually so for the stage. A
technical diligence reviewer can read `MEMORY.md` and `docs/PHASES.md` and know exactly
what is built, what is verified, and what is claimed but unproven — because those three
categories are marked separately throughout.

## 3. Security and compliance

| Document | Status | Location |
|---|---|---|
| Security whitepaper | ✅ | `docs/compliance/security-whitepaper.md` |
| Data flow / DPIA input | ✅ | `docs/compliance/data-flow.md` |
| Subprocessor list | ✅ | `docs/compliance/subprocessors.md` |
| DPA template | ⚠️ Written, **not reviewed by counsel** | `docs/compliance/dpa-template.md` |
| SOC 2 gap analysis | ✅ Honest, with costed remediation | `docs/compliance/soc2-readiness.md` |
| SOC 2 / ISO 27001 attestation | ❌ Not started | — |
| Penetration test report | ❌ Not performed | — |
| Information security policy set | ❌ Not written | — |

## 4. Financial

| Document | Status | Location |
|---|---|---|
| Metrics definitions | ✅ | `docs/investor/metrics-definitions.md` |
| Unit economics model | ⚠️ Formulas defined; **no actuals — pre-revenue** | Same |
| Historical financials | ❌ No operating history | — |
| Financial projections | ❌ | — |
| Bank statements | ❌ | — |

**Pre-revenue.** Any metric quoted today is a formula, not a measurement. The definitions
document is written so that the moment there is real traffic, the numbers can be produced
without anyone having to decide what they mean under pressure.

## 5. Commercial

| Document | Status | Location |
|---|---|---|
| Pricing and plans | ✅ | `MASTER_BUILD.md`, `/pricing` |
| Customer contracts | ❌ No customers | — |
| Pipeline | ❌ | — |
| Terms of service | ❌ Not drafted | — |
| Partner agreements | ❌ | — |

## 6. Team

| Document | Status | Location |
|---|---|---|
| Founder biographies | ❌ | — |
| Org chart / hiring plan | ❌ | — |
| Employment agreements | ❌ No employees | — |
| Option pool | ❌ | — |

---

## Honest summary for a reader

**What is real:** a complete, tested, deployable product. Roughly 700 automated tests
covering tenant isolation, integer money arithmetic, rate limiting under concurrency,
credential redaction, and the full request pipeline. A dashboard and marketing site that
build clean. Deployment and self-hosting configuration. Documentation good enough that a
new engineer — or a new AI agent — can resume work cold.

**What is not real yet:** the company. No incorporation, no customers, no revenue, no
attestations, no legal review. The engineering is materially ahead of the corporate
formation, which is an unusual shape and worth naming rather than hiding.

**The three things that most change this picture, cheapest first:**

1. Execute the restore drill and the load test. Both scripted, neither run. Together they
   convert the two largest ⚠️ marks in `docs/PHASES.md` into ✅ and remove the biggest
   disclosure from the DPA. Days of work.
2. Incorporate and assign IP. Weeks, mostly waiting.
3. Get the first paying customer. Everything in Section 4 and Section 5 is blocked on this
   and on nothing else.

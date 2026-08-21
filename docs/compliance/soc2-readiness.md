# SOC 2 Type II readiness

**Assessment date: 2026-08-21 · Assessor: internal · Status: not started formally**

An honest gap analysis. The purpose is to know what a real audit would cost and how long it
would take, not to look ready. Anything marked ✅ is implemented and evidenced in code;
⚠️ is partially there; ❌ has not been started.

Scope assumed: **Security** and **Availability** trust services criteria. Confidentiality
and Privacy are achievable later; Processing Integrity is not relevant to a gateway.

---

## Common Criteria

### CC1 — Control environment

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC1.1 Integrity and ethical values | ❌ | No code of conduct. Needed: a one-page document, signed at onboarding. |
| CC1.2 Board oversight | ❌ | No board. Pre-formation. |
| CC1.3 Organisational structure | ⚠️ | Two founders, roles understood but not written down. |
| CC1.4 Competence | ❌ | No documented hiring or background-check process. |
| CC1.5 Accountability | ❌ | No performance/accountability process. |

**Reality:** CC1 is almost entirely process, not engineering. It is the cheapest section to
close and the one most likely to be left until last.

### CC2 — Communication and information

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC2.1 Quality information | ✅ | Structured logging, Prometheus metrics, 10-panel Grafana dashboard (`infra/grafana/`). |
| CC2.2 Internal communication | ⚠️ | `CLAUDE.md`, `MEMORY.md`, ADRs are genuinely good internal comms. No formal security-policy communication. |
| CC2.3 External communication | ⚠️ | Security whitepaper and status endpoint exist. No published incident-communication policy. |

### CC3 — Risk assessment

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC3.1 Objectives specified | ✅ | `MASTER_BUILD.md` states principles and non-goals precisely. |
| CC3.2 Risk identification | ⚠️ | Risks identified and reasoned about in ADRs and code comments; no formal risk register. |
| CC3.3 Fraud risk | ❌ | Not assessed. |
| CC3.4 Change assessment | ⚠️ | ADR process covers architectural change well. No formal change-risk assessment. |

### CC4 — Monitoring

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC4.1 Ongoing evaluation | ⚠️ | CI runs 696 tests, clippy with warnings-as-errors, and a memory-freshness gate on every commit. No periodic control self-assessment. |
| CC4.2 Deficiency communication | ❌ | No formal process. |

### CC5 — Control activities

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC5.1 Control selection | ⚠️ | Controls exist and are tested; not mapped to a framework until this document. |
| CC5.2 Technology controls | ✅ | Extensively. See CC6. |
| CC5.3 Policy deployment | ❌ | No written policies. |

### CC6 — Logical and physical access

**The strongest section, and the one an auditor will spend least time on.**

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC6.1 Logical access security | ✅ | argon2id passwords, SHA-256 key hashes, constant-time comparison, TOTP, SSO, HTTP-only session cookies. |
| CC6.2 Registration and authorisation | ✅ | Role-based access (owner/admin/member/viewer); SCIM provisioning and deprovisioning. |
| CC6.3 Role-based access | ✅ | Enforced per endpoint; a route-surface test asserts every tenant-data route refuses anonymous callers. |
| CC6.4 Physical access | ✅ | Inherited from Hetzner. Their ISO 27001 certificate is the evidence. |
| CC6.5 Data disposal | ⚠️ | Deletion implemented; no documented media-disposal procedure (inherited from provider). |
| CC6.6 External threat protection | ✅ | Cloudflare WAF, rate limiting, body caps, security headers, TLS enforcement. |
| CC6.7 Data transmission | ✅ | TLS 1.2+ required; production refuses to start without it. |
| CC6.8 Malicious software | ⚠️ | No package manager or shell in the runtime image. No formal AV/EDR — arguably not applicable to a distroless container, but an auditor will ask. |

### CC7 — System operations

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC7.1 Vulnerability detection | ✅ | `cargo audit`, `cargo deny` licence check, and `npm audit` all run in CI (`.github/workflows/ci.yml`, `security` job). |
| CC7.2 Monitoring for anomalies | ✅ | Metrics, alerts, spend anomaly detection, reconciliation worker. |
| CC7.3 Incident evaluation | ⚠️ | Runbooks exist (`docs/runbooks/`). No formal severity classification. |
| CC7.4 Incident response | ⚠️ | Documented; **never exercised**. |
| CC7.5 Recovery | ⚠️ | Backups automated, restore runbook and script written; **drill never executed**. |

### CC8 — Change management

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC8.1 Change authorisation | ⚠️ | Conventional commits, ADRs, CI gates. No enforced PR review — single-contributor repository today. |

### CC9 — Risk mitigation

| Criterion | Status | Evidence / gap |
|---|---|---|
| CC9.1 Business disruption | ⚠️ | Circuit breakers, fail-open budget, graceful degradation without Redis or a database. No BCP document. |
| CC9.2 Vendor management | ⚠️ | Subprocessor list maintained. No vendor risk assessment process. |

---

## Availability criteria

| Criterion | Status | Evidence / gap |
|---|---|---|
| A1.1 Capacity monitoring | ⚠️ | Metrics exist. No capacity planning process; **load test never executed**. |
| A1.2 Environmental protection | ✅ | Inherited from Hetzner. |
| A1.3 Recovery testing | ❌ | **Never performed.** The single largest availability gap. |

---

## What to do, in order

Ranked by (audit value) ÷ (effort). The first two are days of work, not weeks.

1. **Run the restore drill.** Closes A1.3, CC7.5, and the largest disclosure in the DPA.
   The script already exists. This is the highest-value item on the list by a wide margin.
2. **Run the load test.** Closes A1.1 and substantiates the sub-1ms overhead claim we make
   publicly. Script is committed; it needs a deployed instance to run against.
3. **Write the policy set.** Information security, access control, incident response,
   change management, acceptable use, vendor management. Six documents, mostly template
   work, closes most of CC1/CC5/CC9.
4. **Enforce PR review.** Trivial once there are two people.
5. **Formal risk register.** Half a day; the reasoning already exists across the ADRs.
6. **Penetration test.** External spend. Do it once 1–5 are done, so the report is about
   real weaknesses rather than missing paperwork.
7. **Engage an auditor.** Type II requires an observation window — typically 3 months
   minimum, 6–12 typical. **Start the clock only after items 1–5, because the window
   observes the controls as they exist at the start.**

## Honest timeline

| Milestone | Realistic |
|---|---|
| Items 1–2 (engineering) | 1 week |
| Items 3–5 (documentation) | 2–3 weeks |
| Type I report | ~6 weeks from now |
| Type II observation window opens | ~7 weeks from now |
| Type II report | ~7 months from now |

The engineering is not the constraint. The observation window is, and no amount of
additional work shortens it — which is an argument for starting the paperwork sooner rather
than after the next feature.

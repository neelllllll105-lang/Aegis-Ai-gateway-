# Metrics definitions

The point of this document is that a number means the same thing in a board deck, a data
room, and the database. Where a metric can be computed two defensible ways, the choice is
stated and the reason given — because the fastest way to lose an investor's trust in
diligence is for two of your own documents to disagree about ARR.

Every definition names the query or code that produces it. A metric with no source is not a
metric.

---

## Revenue

### Gross savings

The difference between what a customer's traffic would have cost at the model they
requested, and what it actually cost at the model that served it.

```
gross_savings_mc = baseline_cost_mc − actual_cost_mc, floored at zero
```

Source: `apps/gateway/src/metering/savings.rs::SavingsBreakdown::compute`.

**Floored at zero, deliberately.** If routing picks something more expensive — which can
happen on a cache miss after a fallback — we record zero savings and charge nothing. We
never bill a negative saving as a positive one, and we never present a period's savings net
of losses without saying so.

### Aegis fee (revenue)

```
aegis_fee_mc = gross_savings_mc × savings_share_bp / 10_000
```

Basis points, integer arithmetic, micro-cents throughout. Source:
`apps/gateway/src/money.rs::savings_share_basis_points`.

**This is the revenue line.** Not gross savings — that is the customer's money, not ours.
Presenting gross savings as revenue would overstate it by 4–5×, and any diligence process
will catch it.

### Customer net

```
customer_net_mc = gross_savings_mc − aegis_fee_mc
```

What the customer actually kept. This is the number the dashboard leads with, because it is
the only one that answers "was this worth it".

### ARR

```
ARR = (trailing 30 days of aegis_fee_mc + subscription_mc) × 12.1667
```

Trailing 30 days annualised, not last-month-times-twelve and not best-month-times-twelve.

**Known weakness, stated up front:** usage-based revenue annualised from 30 days is noisy,
and for a young company it flatters a good month. Any deck showing ARR should show the
trailing 3-month average beside it. If those two numbers diverge by more than 20%, the
lower one is the honest one.

### Net revenue retention

```
NRR = (fee from cohort this period) / (fee from same cohort 12 months ago)
```

Cohort defined by first paid request month. Churned accounts stay in the denominator — a
company that drops churned customers from NRR is reporting expansion, not retention.

---

## Usage

### Requests

Rows in `usage_records`. Cache hits count as requests, because the customer made one and it
was served. Rejections (402 budget, 429 rate limit) count separately and are not in this
figure.

### Savings rate

```
savings_rate = gross_savings_mc / baseline_cost_mc
```

Weighted by spend, not averaged across customers. An unweighted average lets a customer
spending $5 a month with a 90% saving rate offset one spending $50,000 at 20% — which makes
the headline number describe nobody.

### Cache hit rate

```
cache_hit_rate = cache_hits / requests
```

Both exact and semantic hits count. Reported separately as well, because they have very
different quality profiles and an investor who has thought about it will ask.

### Gateway overhead

P50/P95/P99 of `gateway_overhead_ms` — Aegis's own added latency, with provider time
explicitly excluded (`OverheadClock` pauses for the upstream call).

**We publish this and do not game it.** Including provider time would make our overhead
look like a rounding error, which is precisely why we do not.

---

## Customer

### Active organisation

An organisation with at least one non-rejected request in the trailing 30 days. Not "has an
account" — signups are not customers.

### Paying organisation

An organisation with `aegis_fee_mc > 0` in the trailing 30 days, or a non-zero
subscription. An organisation that saved nothing and paid nothing is active, not paying.

### Logo churn / revenue churn

Both reported. Logo churn counts organisations; revenue churn counts fees. They diverge
sharply at this stage — losing ten free-tier accounts and zero revenue is a very different
month from the reverse — so reporting only one is misleading.

---

## Unit economics

### Gross margin

```
gross_margin = (aegis_fee − infrastructure_cost − pooled_provider_cost) / aegis_fee
```

Pooled provider cost is free-tier traffic on our own keys — a real COGS line, not a
marketing expense. Counting it as marketing would inflate gross margin and would not
survive diligence.

**BYOK traffic has no provider COGS**, which is the structural reason the margin profile is
good: on BYOK the customer pays the provider directly and we never touch that money.

### CAC and payback

Not yet meaningful — no paid acquisition has run. Stated as "not measured" rather than
estimated. An estimated CAC in a data room is worse than an absent one.

---

## Where these come from

| Metric | Source |
|---|---|
| gross_savings, fee, customer_net | `metering/savings.rs`, `usage_records` table |
| ARR, NRR | `daily_aggregates` table |
| Requests, cache hit rate | `usage_records` |
| Gateway overhead | Prometheus histogram, `metrics.rs` |
| Active/paying orgs | `organizations` ⋈ `usage_records` |
| Reconciliation delta | `workers/reconciliation.rs` |

## Reconciliation

Counters in Redis are fast and approximate. `usage_records` in Postgres is authoritative.
The reconciliation worker compares them and flags any material divergence
(`workers/reconciliation.rs`).

**Any figure quoted externally comes from Postgres, never from the Redis counters.** The
counters exist to enforce budgets in under a millisecond, not to be correct to the
micro-cent.

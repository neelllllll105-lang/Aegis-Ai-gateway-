# Runbook: public launch

`MASTER_BUILD.md` P5.1. Product Hunt, Tuesday, 12:01am PT.

---

## Before you schedule anything

Launch amplifies whatever state the product is in. A launch on a broken product buys
permanent reputational damage with a one-day traffic spike, and this audience does not
give second looks.

These are hard gates. Do not set a date until every one is true:

- [ ] **Pricing table verified.** `docs/runbooks/pricing-update.md` completed in full.
      Every savings number on the site depends on it, and the launch pitch is that our
      numbers are checkable.
- [ ] **Load test executed** against staging at 2× expected launch traffic, P99 overhead
      confirmed under 1ms. The landing page states this claim; it must be measured, not
      asserted.
- [ ] **Restore drill passed** this month.
- [ ] **A real request served end to end** through the production gateway with a real
      provider key, and the savings figure checked by hand against the provider's own
      billing.
- [ ] **Free-tier pooled keys funded** with enough headroom for a launch-day spike, and
      per-org caps confirmed working. This is the one cost that scales directly with
      signups.
- [ ] **Status page live** and linked from the footer.
- [ ] **Someone on call** for the full 24 hours who can deploy.

---

## Assets

| Asset | Notes |
|---|---|
| Logo | 240×240 PNG, transparent |
| Gallery (5 images) | Dashboard savings view first — it is the product. Then routing decision, request log, docs quickstart, pricing calculator. |
| Demo video (60s) | Screen recording, no voiceover, captions. Show the base-URL change, then a real request, then the savings appearing. |
| Tagline | "Cut your AI bill by up to 90%. Keep the quality. Prove it." |
| First comment | See below. |

### The gallery images matter more than the video

Most people scroll and never press play. Each image must be legible as a thumbnail and
carry one idea. The first should be the savings ledger with real numbers in it — not a
mockup, and not zeroes.

---

## The founder comment

Post it within a minute of going live. Write it in advance.

It should do three things, in this order:

1. **Say what problem you had.** Not the market opportunity — the actual bill that
   prompted this.
2. **Say how it works, specifically.** This audience can tell the difference between a
   technical explanation and a technical-sounding one. Mention the three mechanisms and
   that the classifier is deliberately conservative.
3. **Say what it does not do.** Name a real limitation. It is the single highest-trust
   move available, and the comments will find it anyway.

Do not thank people for the support in advance. Do not use the word "excited".

---

## Launch day

| Time (PT) | Action |
|---|---|
| 12:01am | Product Hunt goes live. Post the founder comment. |
| 12:05am | Verify signup works from a clean browser, in an incognito window, on mobile. |
| 12:15am | Post to Hacker News as **Show HN**. Different framing: lead with the architecture, not the savings. |
| 6:00am | Reply to every comment. Every one. |
| 9:00am | Check pooled key burn rate and the metering gap. |
| Hourly | `/api/admin/metrics` — error rate, P99 overhead, signup count. |

### What to watch, in priority order

```bash
# 1. Are we metering everything? A non-zero gap means unbilled requests.
curl -s https://api.aegis.dev/api/admin/metrics -H "Cookie: $ADMIN" | jq '.metering_gap'

# 2. Is the performance claim still true under real load?
curl -s https://api.aegis.dev/api/admin/metrics -H "Cookie: $ADMIN" | jq '.gateway_overhead_p99_ms'

# 3. Are providers holding up?
curl -s https://api.aegis.dev/api/admin/metrics -H "Cookie: $ADMIN" | jq '.degraded_providers'

# 4. Free-tier burn. This is the only cost that scales with signups.
curl -s https://api.aegis.dev/metrics | grep aegis_requests_total
```

### If the free-tier pool runs dry

Do not let free traffic fail silently. In order of preference:

1. Add more pooled keys — a configuration change and a restart.
2. Lower `AEGIS_FREE_TIER_MONTHLY_REQUESTS` and say so publicly.
3. Return a clear 402 explaining the tier is at capacity and pointing at BYOK.

Never degrade quality silently to save money. It is the exact behaviour the product exists
to protect people from.

---

## Answering the hard questions

They will be asked. Prepare honest answers now rather than improvising at 3am.

**"How is this different from LiteLLM?"**
LiteLLM has far more providers and is free. We are faster, route on quality rather than
just proxying, and prove the savings per request. If you need 140 providers today, use
LiteLLM — say so.

**"What if your routing makes my output worse?"**
The classifier is deliberately biased toward the requested model, complex requests are
never downgraded, and passthrough is available per request and permanently. Point at the
router tests.

**"Why should I trust your savings numbers?"**
Export the CSV and recompute them. The arithmetic is integer micro-cents, so the total
matches exactly rather than approximately.

**"You are a middleman taking a cut."**
Yes. We take a share of savings we actually delivered and nothing when we deliver none.
The alternative is paying full price to the provider.

**"What happens when you shut down?"**
Point the base URL back at your provider. No lock-in, no proprietary format, your keys are
yours. Enterprise runs in your own VPC.

---

## After launch

- Publish a changelog entry the same week.
- Two-week fix cycle on feedback. Ship something visible from the comments within seven
  days — that is what converts a launch spike into retention.
- Write the retrospective into `MEMORY.md`: what broke, what the conversion was, what the
  actual free-tier cost per signup turned out to be.

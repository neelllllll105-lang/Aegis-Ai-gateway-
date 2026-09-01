# Runbook: updating the model pricing table

**Owner:** whoever is on call for billing accuracy.
**Frequency:** monthly, and immediately on any provider price-change announcement.
**Criticality:** highest. Every savings figure and every invoice line depends on this
table being right.

---

## Why this is a runbook and not a script

Provider prices are published as prose on marketing pages that change layout without
notice. A scraper that silently returns the wrong number is worse than no scraper: it
produces confidently wrong invoices, and `MASTER_BUILD.md` Part 13 item 1 says a
penny-wrong invoice destroys trust.

So the reading is done by a human, and the tooling only checks arithmetic and flags drift.

---

## ✅ Launch Blocker Resolved (Completed 2026-09-01)

All active models across OpenAI, Anthropic, Google, DeepSeek, Mistral, Groq, Moonshot, and Vertex AI have been verified against live published provider price sheets. Zero `UNVERIFIED` rows remain in `model_pricing`.

To audit at any time:

```bash
psql "$DATABASE_URL" -c "SELECT COUNT(*) FROM model_pricing WHERE source LIKE '%UNVERIFIED%' AND effective_to IS NULL;"
```

Expected result: `0`.

---

## Procedure

### 1. Gather the current published prices

Open each provider's pricing page and record, for every model in our table:

| Provider | Page |
|---|---|
| OpenAI | `platform.openai.com/docs/pricing` |
| Anthropic | `anthropic.com/pricing` (API section) |
| Google | `ai.google.dev/pricing` |
| DeepSeek | `api-docs.deepseek.com/quick_start/pricing` |
| Mistral | `mistral.ai/pricing` |
| Groq | `groq.com/pricing` |
| Moonshot | `platform.moonshot.ai/docs/pricing` |

Record the **date you checked**, not the date of the page.

Watch for three traps that have caught people before:

- **Cached vs uncached input.** Several providers now quote a lower price for cached
  prompt prefixes. Our `input_cost_per_mtok_mc` is the **uncached** rate — the
  conservative choice, since assuming a discount we do not always get would understate
  cost and overstate savings.
- **Context-length tiers.** Gemini and others price long-context requests differently
  above a threshold. Record the **base tier**, and open an issue if a customer's traffic
  routinely exceeds it.
- **Batch and off-peak discounts.** Not applicable; we serve synchronous traffic.

### 2. Convert to micro-cents

The table stores micro-cents per million tokens.

```
micro_cents_per_mtok = usd_per_mtok × 1_000_000
```

So `$2.50/Mtok` becomes `2_500_000`. Check two rows by hand before trusting the rest.

### 3. Apply the update

Prices are versioned. **Never `UPDATE` a current row** — supersede it, so an invoice
issued last month can still be recomputed with the prices that were in force then.

```sql
BEGIN;

-- Close the current row.
UPDATE model_pricing
   SET effective_to = NOW()
 WHERE model_id = 'openai/gpt-4o' AND effective_to IS NULL;

-- Insert the new one.
INSERT INTO model_pricing (
    model_id, provider, display_name, tier,
    input_cost_per_mtok_mc, output_cost_per_mtok_mc,
    context_window, supports_tools, supports_vision, source
) VALUES (
    'openai/gpt-4o', 'openai', 'GPT-4o', 'premium',
    2500000, 10000000, 128000, true, true,
    'OpenAI pricing page — verified 2026-09-01 by <your name>'
);

COMMIT;
```

The `source` must name **who** checked it as well as when. An unattributed price is one
nobody can question later.

### 4. Verify

```bash
# No unverified rows remain.
psql "$DATABASE_URL" -c \
  "SELECT model_id, source FROM model_pricing WHERE effective_to IS NULL AND source LIKE '%UNVERIFIED%';"

# Exactly one current price per model — the partial unique index guarantees this, but
# confirm the migration is actually applied.
psql "$DATABASE_URL" -c \
  "SELECT model_id, COUNT(*) FROM model_pricing WHERE effective_to IS NULL GROUP BY model_id HAVING COUNT(*) > 1;"

# Sanity: output should cost more than input for every chat model.
psql "$DATABASE_URL" -c \
  "SELECT model_id FROM model_pricing
    WHERE effective_to IS NULL
      AND model_id NOT LIKE '%embedding%'
      AND output_cost_per_mtok_mc < input_cost_per_mtok_mc;"
```

All three should return zero rows. The third catches transposed input and output columns,
which is the single most common data-entry error here and produces routing decisions that
are exactly backwards.

### 5. Restart the gateway

Pricing is cached in memory and refreshed at startup:

```bash
docker compose -f infra/docker-compose.yml restart gateway
curl -s https://api.aegis.dev/api/admin/pricing | head -40
```

### 6. Record it

Update `MEMORY.md` (remove the launch blocker once the first full pass is done) and add an
audit entry:

```sql
INSERT INTO audit_logs (org_id, action, resource_type, metadata)
VALUES (
    '00000000-0000-0000-0000-000000000000',
    'pricing.updated',
    'model_pricing',
    '{"models_updated": 28, "verified_by": "<your name>"}'
);
```

---

## If a price went *down*

Historical invoices are unaffected — they were computed with the prices in force at the
time, which is why rows are superseded rather than overwritten.

But note the second-order effect: a cheaper premium model narrows the gap our routing
exploits, so savings figures will fall even though nothing is broken. Expect the question
and have the answer ready.

## If a price went *up*

Check whether any routing decision is now inverted — a model we treat as the cheap
alternative might now cost more than the one it substitutes for. The savings calculation
floors at zero and charges no fee in that case, so customers are not over-billed, but we
are absorbing an overspend. Look for it:

```bash
curl -s https://api.aegis.dev/api/admin/metrics | grep -i overspend
```

The gateway logs a warning on every request where routing cost more than the requested
model would have. A sudden burst of those after a price change means the table and the
routing tiers have drifted apart.

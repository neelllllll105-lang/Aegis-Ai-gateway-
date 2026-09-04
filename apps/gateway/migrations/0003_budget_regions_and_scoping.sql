-- Regional budgets, and the index the request path needs to load budgets cheaply.
--
-- Two gaps found by the enterprise readiness audit, both of the same shape: a feature that
-- exists in code and in tests but that a customer could never actually turn on.
--
-- 1. `middleware::budget` has enforced a fourth, regional scope since P7.2 -- an org
--    running in several regions has one spend figure but several exposures, and the org
--    total says nothing about one region burning its whole allowance by the 9th. The
--    enforcement, the counters, and the tests all exist. The `budgets` table has no column
--    to express one, so no customer could ever create one. This adds it.
--
-- 2. Budgets are read on the request path now (previously every call site passed None for
--    the org, team, and region limits, so the only ceiling with any effect was the one on
--    the api_keys row). `idx_budgets_org` alone is enough for the org-scoped read, but the
--    partial index below keeps the common case -- an org with a handful of budgets, most
--    of them monthly -- off a sequential scan as the table grows.

ALTER TABLE budgets
    ADD COLUMN region VARCHAR(64);

COMMENT ON COLUMN budgets.region IS
    'Region this budget applies to, lower-cased, e.g. eu-central. NULL means the budget is '
    'not region-scoped. Set alongside team_id/api_key_id being NULL for an org-wide '
    'regional cap.';

-- Region names arrive from two places typed by two different people at two different
-- times: the gateway''s own AEGIS_REGION config, and whatever an admin typed into the
-- dashboard. Normalising on write means the request path can compare them directly rather
-- than lower-casing on every lookup.
ALTER TABLE budgets
    ADD CONSTRAINT budgets_region_is_lowercase
    CHECK (region IS NULL OR region = LOWER(region));

-- A budget row scoped to a team AND a key AND a region would be ambiguous about which
-- counter it caps. One dimension at a time, which is what the enforcement code assumes.
ALTER TABLE budgets
    ADD CONSTRAINT budgets_have_one_scope
    CHECK (
        (CASE WHEN team_id    IS NOT NULL THEN 1 ELSE 0 END) +
        (CASE WHEN api_key_id IS NOT NULL THEN 1 ELSE 0 END) +
        (CASE WHEN region     IS NOT NULL THEN 1 ELSE 0 END) <= 1
    );

CREATE INDEX idx_budgets_org_monthly ON budgets (org_id)
    WHERE period = 'monthly';

-- ---------------------------------------------------------------------------
-- Prompt-cache and long-context pricing.
--
-- Two dimensions the pricing table could not express, both found by the enterprise
-- readiness audit and both producing wrong invoices rather than missing features:
--
-- 1. **Cached tokens.** Every major provider bills a cache read at a fraction of the input
--    rate and, on Anthropic, a cache write at a premium. With no columns for them, the
--    application priced cached tokens at the full input rate -- and because Anthropic
--    reports cache tokens additively while OpenAI folds them into prompt_tokens, the same
--    missing dimension produced an *under*-count on one provider and an *over*-count on
--    the other. Both invisible.
--
-- 2. **Long-context tiers.** Gemini 2.5 Pro bills prompts past 200,000 tokens at
--    $2.50/$15.00 instead of $1.25/$10.00, against a context window of 1,048,576. A flat
--    price under-bills every long-context request on a model sold specifically for long
--    context.
--
-- Defaults reproduce today's behaviour exactly: 10,000 basis points is 100% of the input
-- rate, and a NULL threshold means no second tier. An existing row keeps pricing as it did
-- until someone verifies and sets real values, which is the safe direction -- the defaults
-- can only over-bill relative to the truth, never under-bill.
-- ---------------------------------------------------------------------------

ALTER TABLE model_pricing
    ADD COLUMN cache_read_bp  INTEGER NOT NULL DEFAULT 10000
        CHECK (cache_read_bp BETWEEN 0 AND 20000),
    ADD COLUMN cache_write_bp INTEGER NOT NULL DEFAULT 10000
        CHECK (cache_write_bp BETWEEN 0 AND 20000),
    ADD COLUMN long_context_threshold_tokens   BIGINT
        CHECK (long_context_threshold_tokens IS NULL OR long_context_threshold_tokens > 0),
    ADD COLUMN long_context_input_per_mtok_mc  BIGINT
        CHECK (long_context_input_per_mtok_mc IS NULL OR long_context_input_per_mtok_mc >= 0),
    ADD COLUMN long_context_output_per_mtok_mc BIGINT
        CHECK (long_context_output_per_mtok_mc IS NULL OR long_context_output_per_mtok_mc >= 0);

COMMENT ON COLUMN model_pricing.cache_read_bp IS
    'Cache-read rate in basis points of the input rate. 10000 = full price (no discount), '
    '2500 = OpenAI/Google, 1000 = Anthropic.';
COMMENT ON COLUMN model_pricing.cache_write_bp IS
    'Cache-write rate in basis points of the input rate. 10000 = no separate charge, '
    '12500 = Anthropic five-minute cache write premium.';

-- A long-context tier is all three columns or none of them. A threshold with no rates
-- would silently price long prompts at zero.
ALTER TABLE model_pricing
    ADD CONSTRAINT model_pricing_long_context_is_complete
    CHECK (
        (long_context_threshold_tokens IS NULL
         AND long_context_input_per_mtok_mc IS NULL
         AND long_context_output_per_mtok_mc IS NULL)
     OR (long_context_threshold_tokens IS NOT NULL
         AND long_context_input_per_mtok_mc IS NOT NULL
         AND long_context_output_per_mtok_mc IS NOT NULL)
    );

-- The long-context tier must be more expensive than the base tier, or it is not a tier --
-- it is a bug that quietly discounts the largest requests.
ALTER TABLE model_pricing
    ADD CONSTRAINT model_pricing_long_context_costs_more
    CHECK (
        long_context_input_per_mtok_mc IS NULL
        OR (long_context_input_per_mtok_mc >= input_cost_per_mtok_mc
            AND long_context_output_per_mtok_mc >= output_cost_per_mtok_mc)
    );

-- Cached-token columns on the usage record.
--
-- Without these, an invoice can show that two identical-looking requests to the same
-- model cost different amounts and give the customer no way to see why. Prompt caching is
-- the most common reason for exactly that, so the counts belong on the record next to the
-- tokens they explain.
ALTER TABLE usage_records
    ADD COLUMN cached_input_tokens BIGINT NOT NULL DEFAULT 0
        CHECK (cached_input_tokens >= 0),
    ADD COLUMN cache_write_tokens  BIGINT NOT NULL DEFAULT 0
        CHECK (cache_write_tokens >= 0);

COMMENT ON COLUMN usage_records.cached_input_tokens IS
    'Input tokens served from the provider''s own prompt cache, billed at a discount. '
    'Distinct from a cache_hit, which means Aegis served the whole response without '
    'calling a provider at all.';

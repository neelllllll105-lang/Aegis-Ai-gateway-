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

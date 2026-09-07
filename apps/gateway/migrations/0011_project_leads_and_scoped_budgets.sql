-- Migration 0011: project-lead role, and per-person / token-limited budgets.
--
-- Two independent gaps, bundled because both are "the org/project/person hierarchy is
-- missing a layer it needs to be governable, not just usable."
--
-- 1. `team_memberships` (teams are this schema's "project" — see MASTER_BUILD.md; this
--    migration deliberately does not invent a parallel `projects` table) has always been a
--    bare (team_id, user_id) pair. There has never been a way to say "this person may
--    manage this project's keys and budget" versus "this person is just a member of it" --
--    every team membership has been equally powerless, so the only way to grant that
--    authority has been full org admin, which is far more than a project lead should need.
--
-- 2. `budgets` could only cap an org, a team, an API key, or a region -- never a specific
--    person, and never a token count independent of money. Both were asked for explicitly:
--    "the budget should be based on price and on the tokens spent, be it on a project, be
--    it on a person."

-- ---------------------------------------------------------------------------
-- Project leads.
-- ---------------------------------------------------------------------------
ALTER TABLE team_memberships
    ADD COLUMN IF NOT EXISTS role VARCHAR(20) NOT NULL DEFAULT 'member'
        CHECK (role IN ('lead', 'member'));

COMMENT ON COLUMN team_memberships.role IS
    'lead may manage this team''s keys, budget, and routing defaults without full org '
    'admin. member has read access to the team''s own usage only. Distinct from '
    'organizations.role (owner/admin/member/viewer), which is org-wide.';

-- ---------------------------------------------------------------------------
-- Per-person and token-limited budgets.
-- ---------------------------------------------------------------------------
ALTER TABLE budgets
    ADD COLUMN IF NOT EXISTS user_id UUID REFERENCES users (id) ON DELETE CASCADE;

-- A token ceiling on the same scope as the row's existing money ceiling, enforced
-- independently: a request that is cheap in dollars (a promotional rate, a very cheap
-- model) can still be unbounded in tokens without one. NULL means "money only" -- every
-- budget created before this migration, and every one created after it that does not ask
-- for a token limit.
ALTER TABLE budgets
    ADD COLUMN IF NOT EXISTS limit_tokens BIGINT
        CHECK (limit_tokens IS NULL OR limit_tokens >= 0);

COMMENT ON COLUMN budgets.user_id IS
    'The person this budget caps -- spend summed across every key issued to them. NULL '
    'for a budget that is not scoped to a specific person.';
COMMENT ON COLUMN budgets.limit_tokens IS
    'Additional ceiling in tokens on this row''s scope, enforced independently of '
    'limit_mc. NULL means this budget caps money only.';

-- Re-declare the one-scope-per-row constraint to include the new dimension. Postgres has
-- no ALTER CONSTRAINT, so the old one (team_id/api_key_id/region, added in migration 0003)
-- is dropped and replaced rather than modified in place.
ALTER TABLE budgets DROP CONSTRAINT IF EXISTS budgets_have_one_scope;
ALTER TABLE budgets
    ADD CONSTRAINT budgets_have_one_scope
    CHECK (
        (CASE WHEN team_id    IS NOT NULL THEN 1 ELSE 0 END) +
        (CASE WHEN api_key_id IS NOT NULL THEN 1 ELSE 0 END) +
        (CASE WHEN user_id    IS NOT NULL THEN 1 ELSE 0 END) +
        (CASE WHEN region     IS NOT NULL THEN 1 ELSE 0 END) <= 1
    );

-- "Which budgets cap this person" -- the query the management API's create/list handlers
-- and the request-path budget loader both need, org-scoped like every index in this schema.
CREATE INDEX IF NOT EXISTS idx_budgets_org_user ON budgets (org_id, user_id)
    WHERE user_id IS NOT NULL;

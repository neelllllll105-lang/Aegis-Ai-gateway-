-- Migration 0012: policy-driven routing modes, a default mode per key/project/org, every
-- exceeded budget scope named at once, and a persisted compression breakdown.
--
-- Closes the four gaps left over from IG-1's audit that had no good excuse for staying
-- open: they were each ordinary, low-risk additions to code that already existed, not new
-- subsystems needing their own verification story.

-- ---------------------------------------------------------------------------
-- A default routing mode, resolvable key -> project -> org.
--
-- Before this, the only way to steer traffic onto a mode was the caller sending
-- X-Aegis-Routing-Hint on every request. An org that wants all of its interns' traffic to
-- default to `economy` without trusting every caller to remember the header had no way to
-- say so. Three nullable columns, one per level of the existing hierarchy -- resolution
-- order (most specific wins) is applied in application code, not here.
--
-- The CHECK is repeated three times rather than factored into a domain type because
-- Postgres domains cannot be added to an existing column with ALTER TABLE ... TYPE without
-- a table rewrite, and three copies of a five-value list is not worth that cost.
ALTER TABLE api_keys
    ADD COLUMN IF NOT EXISTS default_routing_mode VARCHAR(20)
        CHECK (default_routing_mode IS NULL
               OR default_routing_mode IN ('passthrough', 'quality', 'balanced', 'economy', 'auto'));

ALTER TABLE teams
    ADD COLUMN IF NOT EXISTS default_routing_mode VARCHAR(20)
        CHECK (default_routing_mode IS NULL
               OR default_routing_mode IN ('passthrough', 'quality', 'balanced', 'economy', 'auto'));

ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS default_routing_mode VARCHAR(20)
        CHECK (default_routing_mode IS NULL
               OR default_routing_mode IN ('passthrough', 'quality', 'balanced', 'economy', 'auto'));

COMMENT ON COLUMN api_keys.default_routing_mode IS
    'Mode this key uses when the caller sends no X-Aegis-Routing-Hint header. NULL falls '
    'through to the team''s default, then the org''s, then auto.';
COMMENT ON COLUMN teams.default_routing_mode IS
    'Mode this project''s keys use by default, when the key itself sets none.';
COMMENT ON COLUMN organizations.default_routing_mode IS
    'Organisation-wide default mode, the last rung before auto.';

-- ---------------------------------------------------------------------------
-- Compression breakdown, persisted rather than only ever computed live.
--
-- The per-technique counts (duplicate messages removed, whitespace chars collapsed, JSON
-- blocks minified, and so on) were only ever visible in the live response and the
-- /api/compression/preview demo endpoint -- nothing wrote them onto the usage record, so
-- "how much did whitespace-collapse save us last month" had no query that could answer it.
ALTER TABLE usage_records
    ADD COLUMN IF NOT EXISTS techniques_fired JSONB;

COMMENT ON COLUMN usage_records.techniques_fired IS
    'Per-technique compression counts for this request (duplicate_system_messages_removed, '
    'whitespace_chars_removed, json_blocks_minified, duplicate_blocks_referenced, '
    'stale_tool_results_trimmed, messages_truncated). NULL when compression did not run '
    '(passthrough mode, zero-retention orgs) or changed nothing.';

-- Migration 0009: per-person attribution.
--
-- Until now the gateway could answer "what did this organisation spend" and "what did this
-- team spend", but never "what did this person spend". `AuthContext::from_key` hardcoded
-- `user_id: None` because an API key belonged to an org and a team and nobody else, and
-- `usage_records` had no column to put a person in even if one had been known.
--
-- Two columns close that:
--
--   api_keys.assigned_to_user_id  — the person this key was issued to, set by whoever
--                                   distributes keys. NULL means a shared key (a project
--                                   key, a service account), which is a legitimate and
--                                   common case, so this is deliberately nullable rather
--                                   than backfilled with a guess.
--
--   usage_records.user_id         — stamped from the key's assignee at request time and
--                                   frozen onto the record, exactly like cost. Resolving
--                                   it later through the key would be wrong: a key can be
--                                   reassigned, and history must not move when it is.
--
-- ON DELETE SET NULL rather than CASCADE: removing a person from an organisation must not
-- delete the keys they used or rewrite what was already spent. The key survives,
-- unassigned, for an administrator to reassign or revoke deliberately.

ALTER TABLE api_keys
    ADD COLUMN IF NOT EXISTS assigned_to_user_id UUID REFERENCES users (id) ON DELETE SET NULL;

-- No foreign key on usage_records: it is partitioned and append-only, and a billing record
-- must remain readable after the user row it referenced is gone. The same reasoning already
-- applies to org_id and api_key_id on this table.
ALTER TABLE usage_records
    ADD COLUMN IF NOT EXISTS user_id UUID;

-- "What has this person spent this month" is the query the whole feature exists to serve,
-- and it is always org-scoped and time-bounded.
CREATE INDEX IF NOT EXISTS idx_usage_user_time
    ON usage_records (org_id, user_id, created_at DESC);

-- "Which keys belong to this person" — used both by the dashboard's per-member view and by
-- the permission check that stops an ordinary member listing everyone else's keys.
CREATE INDEX IF NOT EXISTS idx_api_keys_assignee
    ON api_keys (org_id, assigned_to_user_id);

COMMENT ON COLUMN api_keys.assigned_to_user_id IS
    'The person this key was issued to. NULL for shared project or service keys.';
COMMENT ON COLUMN usage_records.user_id IS
    'Copied from the issuing key''s assignee at request time and never recomputed, so '
    'reassigning a key cannot rewrite spend history.';

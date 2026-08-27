-- Durable cache tier: queries the hot Redis cache has already proven repeat get promoted
-- here, encrypted, so a genuinely popular question survives past the hot tier's 24h TTL
-- without keeping every one-off prompt around indefinitely. See
-- docs/adr/0008-tiered-durable-cache.md for the reasoning.

CREATE TABLE cache_entries (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id             UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    -- The same tenant-scoped SHA-256 fingerprint the hot cache already uses
    -- (cache/fingerprint.rs). org_id is a real column too, not just embedded in the
    -- fingerprint, so a lookup and an org-wide invalidation are both plain indexed
    -- queries rather than a hash-prefix scan.
    fingerprint        TEXT NOT NULL,
    -- AES-256-GCM(per-tenant key, response JSON) as nonce || ciphertext || tag, the same
    -- layout provider_credentials.encrypted_key already uses. The per-tenant key is
    -- derived from the master key via HKDF and never stored — compromising this table
    -- alone reveals nothing.
    encrypted_response BYTEA NOT NULL,
    -- How many times this fingerprint has been served from the hot tier before being
    -- promoted, plus every durable-tier hit after that. Not used for eviction — expiry
    -- alone handles that — but it is the honest answer to "which cached queries are
    -- actually saving money," surfaced nowhere yet but useful for future admin tooling.
    hit_count          INTEGER NOT NULL DEFAULT 2 CHECK (hit_count >= 2),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_hit_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Sliding: refreshed on every hit, so a query that keeps repeating keeps its place,
    -- and one that stops repeating ages out on its own with no separate cleanup logic
    -- needed beyond a periodic DELETE WHERE expires_at < now().
    expires_at         TIMESTAMPTZ NOT NULL,
    UNIQUE (org_id, fingerprint)
);

CREATE INDEX idx_cache_entries_expires_at ON cache_entries (expires_at);
CREATE INDEX idx_cache_entries_org ON cache_entries (org_id);

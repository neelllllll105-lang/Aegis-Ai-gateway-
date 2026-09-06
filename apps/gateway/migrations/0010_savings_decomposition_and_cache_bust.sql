-- Migration 0010: savings decomposition, and prefix cache-bust detection.
--
-- `usage_records` has carried `gross_savings_mc` since the first migration -- the total
-- saved, versus the model the caller requested. That answers "how much did we save" but
-- not "why": a customer's finance team asking whether the saving came from routing to a
-- cheaper model, from context compression, or from caching has never had an answer, and
-- the founder-level ask this migration exists to satisfy was explicit about wanting all
-- three levers visible separately, not just the total.
--
-- Two of the three are priced directly at write time from data the pipeline already has;
-- the third (routing) is stored as whatever of `gross_savings_mc` the other two do not
-- explain. See `crate::metering::savings::SavingsComponents` for why that is the accurate
-- way to do this decomposition without a second, harder-to-verify hypothetical price
-- lookup -- the doc comment there is the source of truth this column comment summarises.
--
-- All three default to zero and are nullable-by-default in effect (a zero on a non-zero
-- `gross_savings_mc` row written before this migration shipped just means the figure was
-- never decomposed, not that nothing was saved) -- existing rows are not, and cannot be,
-- backfilled with a real answer.
ALTER TABLE usage_records
    ADD COLUMN IF NOT EXISTS routing_savings_mc     BIGINT NOT NULL DEFAULT 0
        CHECK (routing_savings_mc >= 0),
    ADD COLUMN IF NOT EXISTS compression_savings_mc BIGINT NOT NULL DEFAULT 0
        CHECK (compression_savings_mc >= 0),
    ADD COLUMN IF NOT EXISTS cache_savings_mc       BIGINT NOT NULL DEFAULT 0
        CHECK (cache_savings_mc >= 0);

-- The parts must never exceed the whole. This is the same "the parts always reconstitute
-- the whole" property `fee_within_savings` already enforces for `aegis_fee_mc`, extended to
-- the decomposition -- a database constraint, not just an application-level test, because
-- this table is written from more than one place over the life of the product.
ALTER TABLE usage_records
    ADD CONSTRAINT savings_components_within_gross CHECK (
        routing_savings_mc + compression_savings_mc + cache_savings_mc <= gross_savings_mc
    );

COMMENT ON COLUMN usage_records.routing_savings_mc IS
    'Share of gross_savings_mc attributed to model substitution. Computed as a residual: '
    'gross_savings_mc minus whatever compression_savings_mc and cache_savings_mc directly '
    'explain -- see crate::metering::savings::SavingsComponents.';
COMMENT ON COLUMN usage_records.compression_savings_mc IS
    'Share of gross_savings_mc attributed to context compression: tokens removed before '
    'the request reached a provider, priced at the served model''s input rate.';
COMMENT ON COLUMN usage_records.cache_savings_mc IS
    'Share of gross_savings_mc attributed to caching: the whole baseline on an Aegis '
    'exact/semantic/durable cache hit, or the provider''s own prompt-cache discount on a '
    'request that did reach a provider.';

-- ---------------------------------------------------------------------------
-- Prefix cache-bust detection (technique #7, detect-only).
--
-- Agent frameworks routinely embed a fresh timestamp, request id, or nonce directly into
-- the system prompt on every turn. Each occurrence changes the prefix's bytes and
-- invalidates the provider's own prompt cache for everything that follows it -- silently
-- undoing the very discount `cache_savings_mc` above exists to measure. This column is the
-- count of such spans found in the system prompt at parse time; see
-- crate::engine::cache_bust. Detect-only: nothing here rewrites a customer's prompt.
-- ---------------------------------------------------------------------------
ALTER TABLE usage_records
    ADD COLUMN IF NOT EXISTS cache_bust_hits INTEGER NOT NULL DEFAULT 0
        CHECK (cache_bust_hits >= 0);

COMMENT ON COLUMN usage_records.cache_bust_hits IS
    'Volatile spans (timestamps, UUIDs, request ids, nonces) found in this request''s '
    'system prompt that would invalidate the provider''s own prefix cache on every turn. '
    'Zero on a cache hit, since no provider was called. Detect-only -- see '
    'crate::engine::cache_bust.';

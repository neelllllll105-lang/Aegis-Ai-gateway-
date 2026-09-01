-- OpenRouter pricing, kept as a reference/cross-check signal only.
--
-- This is NOT model_pricing. Nothing in the request path, the router, or billing reads
-- this table, and nothing should ever be changed to make it do so without a very
-- deliberate decision to revisit that: OpenRouter is a reseller, not the provider itself,
-- and their own docs never confirm there is no markup over the direct provider rate
-- Aegis actually pays. See metering::openrouter_reference's module doc and
-- docs/runbooks/pricing-update.md for the full reasoning.
--
-- One row per OpenRouter model id, upserted on every fetch — a current snapshot, not a
-- history. If drift-over-time analysis is wanted later, that is a natural extension
-- (either a separate append-only table, or an effective_from/effective_to pair like
-- model_pricing already uses), not built here.

CREATE TABLE openrouter_pricing_reference (
    model_id                TEXT PRIMARY KEY,
    display_name            TEXT NOT NULL,
    context_length           INTEGER,
    -- Micro-cents per million tokens, same unit and scale as model_pricing, so a future
    -- comparison against it needs no unit conversion. NULL means OpenRouter did not
    -- report this price at all; a genuine zero (a free model, an uncharged cache tier)
    -- is stored as the integer 0, not NULL — see
    -- metering::openrouter_reference::per_token_usd_to_micro_cents_per_mtok.
    input_per_mtok_mc        BIGINT,
    output_per_mtok_mc       BIGINT,
    cache_read_per_mtok_mc   BIGINT,
    cache_write_per_mtok_mc  BIGINT,
    -- The full, unmodified model object OpenRouter returned. Every field this table does
    -- not extract into a typed column is still here, so a smarter reconciliation query
    -- later never needs a re-fetch to get at something this migration didn't anticipate.
    raw                      JSONB NOT NULL,
    fetched_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_openrouter_pricing_reference_fetched_at
    ON openrouter_pricing_reference (fetched_at);

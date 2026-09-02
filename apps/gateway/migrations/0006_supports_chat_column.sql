-- Migration 0006: Add supports_chat column to model_pricing.
--
-- The Rust PricingRow struct has always had a supports_chat field, annotated with
-- #[sqlx(default)], which means sqlx fills it from bool::default() = false when the
-- column is absent. This caused ALL models loaded from the database to have
-- supports_chat=false, making the chat-capable requirement check in cheaper_alternatives()
-- always fail, so smart routing never produced a cheaper candidate and fell back to
-- passthrough on every request.
--
-- We add the column with DEFAULT TRUE (all chat-completion models) and then flip the
-- embedding models whose model_id contains "embed" to FALSE, matching the heuristic
-- already used in the hardcoded test fixture.

ALTER TABLE model_pricing
    ADD COLUMN IF NOT EXISTS supports_chat BOOLEAN NOT NULL DEFAULT TRUE;

-- Embedding models are not chat models.
UPDATE model_pricing
SET supports_chat = FALSE
WHERE model_id ILIKE '%embed%';

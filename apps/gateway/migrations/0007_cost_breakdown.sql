-- Migration 0007: Add input_cost_mc and output_cost_mc to usage_records.
--
-- Enables discrete financial transparency between prompt processing cost and completion
-- generation cost. Propagates automatically to all existing and future monthly partitions.

ALTER TABLE usage_records
    ADD COLUMN IF NOT EXISTS input_cost_mc BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS output_cost_mc BIGINT NOT NULL DEFAULT 0;

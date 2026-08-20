-- Monthly partition management for usage_records.
--
-- A partitioned table with no matching partition **rejects the insert**. Since
-- usage_records is the billing source of truth, a missing partition is not a degraded
-- experience — it is silent revenue loss and a violation of Principle 2. So:
--
--   * partitions are created ahead of need, not on demand;
--   * the maintenance function is idempotent, so calling it repeatedly is free;
--   * a DEFAULT partition catches anything that still slips through, so a row is
--     misfiled rather than lost, and the reconciliation job can find and move it.

-- Create the partition covering a given month, if it does not already exist.
CREATE OR REPLACE FUNCTION ensure_usage_partition(target DATE)
RETURNS TEXT
LANGUAGE plpgsql
AS $$
DECLARE
    start_of_month DATE := DATE_TRUNC('month', target)::DATE;
    end_of_month   DATE := (DATE_TRUNC('month', target) + INTERVAL '1 month')::DATE;
    partition_name TEXT := FORMAT('usage_records_%s', TO_CHAR(start_of_month, 'YYYYMM'));
BEGIN
    IF EXISTS (SELECT 1 FROM pg_class WHERE relname = partition_name) THEN
        RETURN partition_name;
    END IF;

    EXECUTE FORMAT(
        'CREATE TABLE %I PARTITION OF usage_records FOR VALUES FROM (%L) TO (%L)',
        partition_name, start_of_month, end_of_month
    );

    RETURN partition_name;
END;
$$;

COMMENT ON FUNCTION ensure_usage_partition(DATE) IS
    'Idempotently create the usage_records partition covering the month containing the argument.';

-- Keep the current month and the next two live. Called at startup and hourly by
-- workers::usage_writer, so a month boundary can never arrive unprepared.
CREATE OR REPLACE FUNCTION maintain_usage_partitions()
RETURNS SETOF TEXT
LANGUAGE plpgsql
AS $$
DECLARE
    offset_months INT;
BEGIN
    FOR offset_months IN 0..2 LOOP
        RETURN NEXT ensure_usage_partition(
            (CURRENT_DATE + (offset_months || ' months')::INTERVAL)::DATE
        );
    END LOOP;
END;
$$;

COMMENT ON FUNCTION maintain_usage_partitions() IS
    'Ensure the current and next two monthly usage_records partitions exist.';

-- Last-resort catch-all. A row landing here means partition maintenance failed; the
-- reconciliation worker alerts on a non-empty default partition.
CREATE TABLE IF NOT EXISTS usage_records_default PARTITION OF usage_records DEFAULT;

-- Create the partitions needed right now so the very first request can be recorded.
SELECT maintain_usage_partitions();

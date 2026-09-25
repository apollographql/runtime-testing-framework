-- Supports the per-cluster hourly execution counts shown in the cluster summary, which select
-- executions started within a recent time window.
--
-- Without this, the query falls back to a sequential scan of the whole execution table.

CREATE INDEX IF NOT EXISTS test_execution_started_at_idx
    ON test_execution (started_at DESC);

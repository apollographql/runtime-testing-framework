-- Indexes covering the foreign-key style lookups performed by the orchestrator.
-- Without these, status reads and per-run execution lookups fall back to sequential scans.

-- `db::test_run::test_executions_for_run` filters executions by their parent run.
CREATE INDEX IF NOT EXISTS test_execution_test_run_id_idx
    ON test_execution (test_run_id);

-- Status history and current-status reads filter by parent_id and order by updated_at DESC;
-- the composite index lets Postgres satisfy both the filter and the sort from the index.
CREATE INDEX IF NOT EXISTS test_run_status_parent_id_updated_at_idx
    ON test_run_status (parent_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS test_execution_status_parent_id_updated_at_idx
    ON test_execution_status (parent_id, updated_at DESC);

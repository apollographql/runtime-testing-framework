-- Supports listing historic test runs filtered by name/initiator and ordered/paginated by
-- start time (see `db::test_run::TestRun::query`).

CREATE INDEX IF NOT EXISTS test_run_started_at_idx
    ON test_run (started_at DESC);

CREATE INDEX IF NOT EXISTS test_run_initiated_by_idx
    ON test_run (initiated_by);

CREATE INDEX IF NOT EXISTS test_run_name_idx
    ON test_run (name);

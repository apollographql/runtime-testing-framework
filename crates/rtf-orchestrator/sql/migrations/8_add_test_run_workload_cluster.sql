-- Records which workload cluster a test run's executions ran (or will run) in. NOT NULL with a
-- default so existing rows backfill to "alpha" for free, and every future run always has an
-- explicit cluster rather than an ambiguous absence.
ALTER TABLE test_run
    ADD COLUMN workload_cluster TEXT NOT NULL DEFAULT 'alpha';

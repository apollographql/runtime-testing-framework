-- Both of these columns hold the name of a workload cluster pool rather than of an individual
-- cluster (test_execution.workload_cluster is the one that holds an actual cluster), so rename them
-- to match.
ALTER TABLE test_run
    RENAME COLUMN workload_cluster TO workload_pool;

ALTER TABLE known_test_plan
    RENAME COLUMN pinned_workload_cluster TO pinned_workload_pool;

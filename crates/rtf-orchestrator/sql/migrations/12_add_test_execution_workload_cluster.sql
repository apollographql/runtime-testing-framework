-- Records the workload cluster each execution was assigned to. Nullable so that the previous
-- server version, which does not populate it, can still insert executions if we need to roll back.
ALTER TABLE test_execution
    ADD COLUMN workload_cluster TEXT DEFAULT NULL;

UPDATE test_execution
SET workload_cluster = test_run.workload_cluster
FROM test_run
WHERE test_execution.test_run_id = test_run.id;

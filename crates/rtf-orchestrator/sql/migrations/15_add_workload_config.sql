-- A known test plan's overrides to the workload config of the pool it runs in. This is a JSON object
-- in which every field is optional, a field being absent meaning "use the pool's value". NULL means
-- no overrides at all. It replaces the allow_k8s_write column, which is now one of the fields of
-- this object (the existing values are not migrated, they are set again by hand after deployment).
ALTER TABLE known_test_plan
    ADD COLUMN workload_config_patch JSONB DEFAULT NULL,
    DROP COLUMN allow_k8s_write;

-- The fully resolved workload config (pool values with the known test plan overrides applied) a run
-- executed with. Many runs share the same config, so the UNIQUE constraint on `data` deduplicates
-- in the same way as `variables`: an upsert returns the existing row's id on a repeat.
CREATE TABLE IF NOT EXISTS workload_config (
    id   SERIAL PRIMARY KEY,
    data JSONB NOT NULL UNIQUE
);

-- A run references its resolved config by id. Nullable so that runs created before this existed
-- (which only have test_run.allow_k8s_write) can still be recovered. Loose INT + CHECK rather than
-- a real FOREIGN KEY, matching the convention used by variables_id.
ALTER TABLE test_run
    ADD COLUMN workload_config_id INT CHECK (workload_config_id > 0);

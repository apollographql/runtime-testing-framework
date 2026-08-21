-- Optionally pins a known test plan to a specific workload cluster, overriding whatever cluster
-- would otherwise be chosen for its runs. NULL (the default) means "no pin" — the run falls back
-- to the server's configured default cluster.
ALTER TABLE known_test_plan
    ADD COLUMN pinned_workload_cluster TEXT DEFAULT NULL;

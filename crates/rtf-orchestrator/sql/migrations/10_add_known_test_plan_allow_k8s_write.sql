-- Whether or not a known test plan's scenario should be granted elevated k8s write permissions.
ALTER TABLE known_test_plan
    ADD COLUMN allow_k8s_write BOOLEAN DEFAULT false;

ALTER TABLE test_run
    ADD COLUMN allow_k8s_write BOOLEAN DEFAULT false;

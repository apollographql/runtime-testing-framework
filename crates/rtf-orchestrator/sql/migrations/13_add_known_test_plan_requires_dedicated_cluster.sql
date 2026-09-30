-- Whether runs of a known test plan must have a cluster to themselves. Only valid for plans
-- targeting a pool that supports dedicated clusters.
ALTER TABLE known_test_plan
    ADD COLUMN requires_dedicated_cluster BOOLEAN NOT NULL DEFAULT false;

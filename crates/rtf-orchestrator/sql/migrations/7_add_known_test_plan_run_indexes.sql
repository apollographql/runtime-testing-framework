-- Indexes covering the junction table joining test runs to the test plan they were triggered from.
--
-- `known_test_plan_id` supports gathering one registered test plan's history: both the per-day run
-- status aggregate and the execution duration histogram start by selecting that plan's runs.
--
-- `test_run_id` supports the reverse lookup, which is the direction `db::TestRunFilter` joins in when
-- filtering runs by their known test plan.
--
-- Without these, both directions fall back to a sequential scan of the whole junction table.

CREATE INDEX IF NOT EXISTS known_test_plan_run_known_test_plan_id_idx
    ON known_test_plan_run (known_test_plan_id);

CREATE INDEX IF NOT EXISTS known_test_plan_run_test_run_id_idx
    ON known_test_plan_run (test_run_id);

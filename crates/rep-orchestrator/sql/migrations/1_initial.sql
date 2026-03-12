-- Initial schema for the RTF REP orchestrator service

-- Test Runs represent to a single user request to execute tests written as an RTF test plan.
-- Each run contains one or more Test Executions.
CREATE TABLE IF NOT EXISTS test_run (
    id SERIAL PRIMARY KEY,
    uuid UUID UNIQUE NOT NULL DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

-- Test Executions represent individual environment/scenario pairs run in an ephemeral namespace.
-- Each execution is associated with a single Test Run.
CREATE TABLE IF NOT EXISTS test_execution (
    id SERIAL PRIMARY KEY,
    uuid UUID UNIQUE NOT NULL DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    test_run_id INT NOT NULL,
    exit_code INT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

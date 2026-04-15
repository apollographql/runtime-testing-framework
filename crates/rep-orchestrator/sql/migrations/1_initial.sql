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

-- Status history is tracked over time using the ID of the parent test run and
-- a status ID that is converted to a Rust enum in the server.
CREATE TABLE IF NOT EXISTS test_run_status (
    parent_id INT NOT NULL CHECK (parent_id > 0),
    status INT NOT NULL CHECK (status > 0),
    message TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
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
    completed_at TIMESTAMPTZ,
    has_file_upload BOOLEAN NOT NULL DEFAULT false
);

-- Status history is tracked over time using the ID of the parent test
-- execution and a status ID that is converted to a Rust enum in the server.
CREATE TABLE IF NOT EXISTS test_execution_status (
    parent_id INT NOT NULL CHECK (parent_id > 0),
    status INT NOT NULL CHECK (status > 0),
    message TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

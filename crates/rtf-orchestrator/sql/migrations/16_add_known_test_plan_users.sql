-- Add support for associating users with known test plans for the purpose of
-- updating config and limited triggers.

-- User email addresses that are checked against the IAP authenticated user header
CREATE TABLE IF NOT EXISTS test_plan_user (
    id SERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE
);

-- Association of users to known test plans.
-- `role` is a Rust enum managed by the orchestrator
CREATE TABLE IF NOT EXISTS known_test_plan_user (
    user_id INT NOT NULL CHECK (user_id > 0),
    known_test_plan_id INT NOT NULL CHECK (known_test_plan_id > 0),
    role INT NOT NULL CHECK (role > 0),
    PRIMARY KEY (known_test_plan_id, user_id)
);

-- Flags for restricting triggers to a subset of registered users.
-- The `restrict_triggers` flag applies to regular users and makes use of
-- the `known_test_plan_user` table above. The `restrict_automated_triggers`
-- flag gates whether or not automation users are allowed to trigger test
-- runs and is independent of restricting triggers for regular users.
ALTER TABLE known_test_plan
    ADD COLUMN restrict_triggers BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN restrict_automated_triggers BOOLEAN NOT NULL DEFAULT false;

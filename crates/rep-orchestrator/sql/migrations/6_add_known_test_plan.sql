-- A Test Plan registered with the orchestrator so it can be triggered by UUID or name without the
-- caller needing to provide its GitHub location at the point of triggering.
CREATE TABLE IF NOT EXISTS known_test_plan (
    id SERIAL PRIMARY KEY,
    uuid UUID UNIQUE NOT NULL DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    org TEXT NOT NULL,
    repo TEXT NOT NULL,
    path TEXT NOT NULL,
    UNIQUE (org, repo, path)
);

CREATE TABLE IF NOT EXISTS known_test_plan_run (
    id SERIAL PRIMARY KEY,
    known_test_plan_id INT NOT NULL CHECK (known_test_plan_id > 0),
    test_run_id INT NOT NULL CHECK (test_run_id > 0),
    git_sha TEXT
);

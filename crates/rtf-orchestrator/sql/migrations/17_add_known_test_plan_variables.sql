-- Association of named variables blobs against known test plans.
-- The blobs themselves are stored in the `variables` table.
CREATE TABLE IF NOT EXISTS known_test_plan_variables (
    known_test_plan_id INT NOT NULL CHECK (known_test_plan_id > 0),
    name TEXT NOT NULL,
    variables_id INT NOT NULL CHECK (variables_id > 0),
    PRIMARY KEY (known_test_plan_id, name)
);

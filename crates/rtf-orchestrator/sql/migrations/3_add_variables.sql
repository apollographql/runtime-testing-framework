-- Captured runtime variable overrides (`-v` flags and `--vars` file) for a test run, stored in the
-- flat form the user supplied them. The same override set is expected to recur across many runs, so
-- the UNIQUE constraint on `data` deduplicates: an upsert returns the existing row's id on a repeat.
-- Postgres normalizes jsonb key order on storage, so equal override sets supplied in different key
-- orders collapse to one row (array order is preserved, distinguishing matrix dimensions).
CREATE TABLE IF NOT EXISTS variables (
    id   SERIAL PRIMARY KEY,
    data JSONB NOT NULL UNIQUE
);

-- A run references its captured variables by id. Nullable: NULL exactly when the run had no runtime
-- overrides. Loose INT + CHECK rather than a real FOREIGN KEY, matching the convention used by
-- payload_cache and the *_status tables.
ALTER TABLE test_run
    ADD COLUMN variables_id INT CHECK (variables_id > 0);

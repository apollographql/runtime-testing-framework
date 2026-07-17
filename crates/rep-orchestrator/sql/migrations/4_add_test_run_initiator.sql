-- Records who (or what) triggered a test run, read from the X-Goog-Authenticated-User-Email
-- header IAP forwards. NOT NULL + DEFAULT is a metadata-only change for a constant
-- default, so existing rows need no explicit backfill.
ALTER TABLE test_run
    ADD COLUMN initiated_by TEXT NOT NULL DEFAULT 'unknown';

-- Records who (or what) triggered a test run, read from the X-Goog-Authenticated-User-Email
-- header IAP forwards. Nullable: NULL means no identity could be extracted (or none was
-- supplied), which existing rows and any such future run both fall into with no backfill needed.
ALTER TABLE test_run
    ADD COLUMN initiated_by TEXT DEFAULT NULL;

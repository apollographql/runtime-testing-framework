<!-- diataxis-type: reference -->

# rtf-orchestrator

This page documents how tests in the [`rtf-orchestrator`][0] crate are organized and implemented.
The crate uses two test styles:

- Unit tests follow the [unit test style][1]
- HTTP API integration tests follow the [HTTP API integration test style][2]

See [Run rtf-orchestrator tests][3] for a step-by-step guide to running each category locally.

## Categories

| Category          | Location                                        | Requires                    |
| ----------------- | ----------------------------------------------- | --------------------------- |
| Unit tests        | `src/` (`#[cfg(test)] mod tests` blocks)        | Nothing                     |
| DB tests          | `src/` (gated by `db_tests` feature)            | PostgreSQL via `make db-up` |
| Integration tests | `tests/suite.rs` (gated by `k8s_tests` feature) | Full tilt stack             |

Running `cargo test` from the workspace root skips all stack-dependent tests. They must be run
explicitly via `make`.

## Unit test organization

Unit tests follow the Module → Function → Test class → Test case hierarchy from the
[unit test style][1]:

```rust
// Without simple_test_case
module::path::tests::function_test_case

// With simple_test_case
module::path::tests::function_test_class::test_case
```

## Integration test organization

Integration tests follow the Endpoint → Scenario naming described in the
[HTTP API integration test style][2]:

```rust
endpoint_scenario
```

For example: `trigger_valid_test_plan_returns_200`.

## Feature flag annotation

Stack-dependent tests must always carry this annotation to prevent the standard test run from
failing without a stack:

```rust
#[cfg_attr(not(feature = "db_tests"), ignore)]    // for DB tests
#[cfg_attr(not(feature = "k8s_tests"), ignore)]   // for integration tests
```

Do not remove these annotations from existing tests.

## Testing infrastructure

Integration tests use the following tools:

- **`TestHelper`** — Defined in `tests/common/mod.rs`. Wraps `reqwest::Client` and provides
  convenience methods for calling server endpoints
- **[`tokio`][4]** — `#[tokio::test]` for async test functions
- **[`reqwest`][5]** — HTTP client used inside `TestHelper`
- **[`assert_fs`][6]** — Temporary filesystem utilities; uses `CARGO_TARGET_TMPDIR` rather than
  `/tmp` to avoid macOS symlink issues with Docker volume mounts
- **[`simple_test_case`][7]** — Parameterized testing with `#[test_case]` for multiple input
  variations

### TestHelper API

```rust
pub struct TestHelper {
    client: Client,
}
```

| Method                   | Purpose                                                              |
| ------------------------ | -------------------------------------------------------------------- |
| `prepare_rep_payload`    | Prepares a `TriggerPayload` from a test plan directory               |
| `json_get` / `json_post` | Typed helpers that deserialize JSON responses into the expected type |
| `get` / `post`           | Raw helpers returning `Response` for status-code assertions          |

Shared logic between tests should be added as further methods on `TestHelper`.

## Make targets

| Target                                                            | Description                                         |
| ----------------------------------------------------------------- | --------------------------------------------------- |
| `make db-up` / `make db-down`                                     | Start/stop the lightweight DB-only test stack       |
| `make db-tests`                                                   | Run DB-gated tests against the running DB stack     |
| `make cluster-setup && make cluster-up` / `make cluster-teardown` | Start/stop the full stack                           |
| `make integration-tests`                                          | Run `tests/suite.rs` against the running full stack |
| `make ci-tests`                                                   | Full CI flow: stack up → wait → test → stack down   |

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-orchestrator
[1]: ./unit-tests.md
[2]: ./http-tests.md
[3]: ../../howto/run-rtf-orchestrator-tests.md
[4]: https://docs.rs/tokio/latest/tokio/attr.test.html
[5]: https://docs.rs/reqwest/latest/reqwest/
[6]: https://docs.rs/assert_fs/latest/assert_fs/
[7]: https://docs.rs/simple_test_case/latest/simple_test_case/

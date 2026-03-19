<!-- diataxis-type: reference -->

# rep-orchestrator

This page documents how tests in the [`rep-orchestrator`][0] crate are organized and implemented.

The crate has two categories of test:

- **Unit tests** — live alongside source code in `src/`. Some require a running Postgres instance
  and are gated behind the `db_tests` feature flag.
- **Integration tests** — live in `tests/suite.rs`. Require a fully running Docker Compose stack
  (orchestrator server + database).

Running `cargo test` from the workspace root will skip all DB-dependent tests. They must be run
explicitly via `make`.

## Organization

### Unit tests

Unit tests in `rep-orchestrator` follow the same hierarchy used in [`rtf-integrations`][1]:

1. **Module** - A logical grouping of functionality. Since this maps directly to the Rust module
   structure, there are likely to be sub-modules in the path. For example, database operations for
   test executions could be nested under `db::test_execution`. The primary concern is the logical
   organization of the Rust code for scopes and privacy.
1. **Function** - The function or logic under test. This most likely maps to a function or trait
   method. If this is not fully specified in the module path, it should be specified as the first
   prefix in the test case name.
1. **Test class** - A logical grouping of test cases. This is defined when using
   [`simple_test_case`][2] to create multiple parameterized tests.
1. **Test case** - The specific test case. This should be uniquely and meaningfully named. Examples
   of test case naming can be seen [here][3].

Following the hierarchy above leads to the following generic test case paths:

```rust
// When simple_test_case is not used
module::path::tests::function_test_case

// When simple_test_case is used
module::path::tests::function_test_class::test_case
```

### Integration tests

Integration tests in `tests/suite.rs` test the HTTP API as a whole. They do not follow the module
hierarchy used by unit tests since they are not organized by source module. Instead, they are named
using the following hierarchy:

1. **Endpoint** - The HTTP endpoint under test. Examples include `trigger`
   (`POST /test-run/trigger`) and `run_status` (`GET /test-run/{id}/status`).
1. **Scenario** - The specific behaviour being verified. Examples include
   `returns_200_for_valid_plan` or `returns_404_for_unknown_run`.

This leads to the following generic test case paths:

```rust
endpoint_scenario
```

#### Example - triggering a test run

This example demonstrates testing the trigger endpoint with a valid test plan:

1. **Endpoint** is `trigger`. This is the endpoint under test.
1. **Scenario** is `valid_rep_test_plan_returns_200`. This describes the input and the expected
   outcome.

This leads to the following full test path:

```rust
test trigger_valid_rep_test_plan_returns_200
```

#### Example - querying run status for an unknown run

1. **Endpoint** is `run_status`. This is the endpoint under test.
1. **Scenario** is `returns_404_for_unknown_run`. This describes the input and the expected outcome.

This leads to the following full test path:

```rust
test run_status_returns_404_for_unknown_run
```

## Implementation

### Testing infrastructure

The crate leverages the following key testing tools:

- **[`simple_test_case`][2]** - Provides `#[test_case]` and `#[dir_cases]` attributes for
  parameterized testing across both unit and integration tests
- **[`tokio`][4]** - Provides the `#[tokio::test]` attribute for async test functions, used
  extensively in integration tests
- **[`reqwest`][5]** - HTTP client used by `TestHelper` to make requests against the running server
  during integration tests
- **[`assert_fs`][6]** - Temporary filesystem utilities used by `TestHelper` to manage test plan
  directories during integration tests
- **[`anyhow`][7]** - Ergonomic error handling in tests with the `-> anyhow::Result<()>` return type

### Unit tests

Unit tests live inside `#[cfg(test)] mod tests` blocks within their source file and test business
logic directly without requiring HTTP or database infrastructure where possible.

To run all unit tests (DB tests skipped):

```bash
cargo test -p rep-orchestrator --lib
```

### DB tests

Tests that require a live database connection are annotated with:

```rust
#[cfg_attr(not(feature = "db_tests"), ignore)]
```

This means they are skipped by `cargo test` (and reported as ignored in the output) unless
`--features db_tests` is passed. Do not remove this annotation from existing tests — it prevents the
standard workspace test run from failing when no database is available.

To run DB tests, first start the test database stack:

```bash
cd crates/rep-orchestrator
make test-db-up
```

In a second terminal session, run the tests:

```bash
make db-tests
```

To stop the database when done:

```bash
make test-db-down
```

### Integration tests

Integration tests require the full Docker Compose stack (orchestrator server + database). They test
the HTTP API end-to-end and are defined in `tests/suite.rs`.

To run the integration tests manually:

```bash
cd crates/rep-orchestrator
make up
```

This will show you the server logs for everything in the stack, useful for debugging failed tests.
In a second terminal window run the tests:

```bash
make integration-tests    # run tests in tests/suite.rs
```

To stop the stack when done run:

```bash
make down
```

To run all tests as CI does (spin up, wait, test, tear down):

```bash
cd crates/rep-orchestrator
make ci-tests
```

#### Testing infrastructure

Integration tests use a `TestHelper` struct defined in `tests/common/mod.rs`. Any shared logic
between tests should be added as further methods to this helper. It wraps a `reqwest::Client` and
provides convenience methods for calling server endpoints:

```rust
pub struct TestHelper {
    client: Client,
}
```

The helper handles:

- **`trigger_run`** - prepares a `RepPayload` from a test plan directory and posts it to
  `/test-run/trigger`
- **`json_get`** / **`json_post`** - typed request/response helpers that deserialize JSON responses
  directly into the expected type.
- **`get`** / **`post`** - request/response helpers that return a `Response` so you can do simple
  high level assertions on things like the status code.
- **Temp directory placement** - uses `CARGO_TARGET_TMPDIR` rather than `/tmp` to avoid macOS
  symlink issues with Docker volume mounts

#### Parameterized integration tests

Tests that cover multiple input variations use `#[test_case]` from [`simple_test_case`][2]:

```rust
#[test_case(&[su(Provisioning, None), su(Running, None), su(Successful, None)]; "successful")]
#[test_case(&[su(Provisioning, None), su(Running, None), su(Failed, Some(1))]; "failed")]
#[test_case(&[su(Provisioning, None), su(Unrunnable, None)]; "unrunnable")]
#[tokio::test]
async fn update_test_execution_status_valid_sequence_works(payloads: &[SetStatusPayload]) {
    // ...
}
```

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rep-orchestrator
[1]: ./rtf-integrations.md
[2]: https://docs.rs/simple_test_case/latest/simple_test_case/
[3]: ./index.md#test-case-naming
[4]: https://docs.rs/tokio/latest/tokio/attr.test.html
[5]: https://docs.rs/reqwest/latest/reqwest/
[6]: https://docs.rs/assert_fs/latest/assert_fs/
[7]: https://docs.rs/anyhow/latest/anyhow/

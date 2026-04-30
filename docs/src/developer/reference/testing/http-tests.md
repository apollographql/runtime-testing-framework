<!-- diataxis-type: reference -->

# HTTP API integration tests

HTTP API integration tests verify HTTP servers end-to-end. They require a running stack (on either
Docker Compose or Kubernetes) and are distinct from unit tests that test business logic in
isolation.

## Test scope

Integration tests are expensive to run — starting a full stack adds significant overhead. Keep them
focused: verify that the stack wires together correctly and that the HTTP API behaves as expected
for the happy path and key error boundaries.

Don't use integration tests to cover every error scenario exhaustively. Detailed error-case coverage
belongs in unit tests, where it's fast and deterministic. Integration tests should give just enough
confidence that the end-to-end plumbing works.

## Feature flags

Stack-dependent tests are gated behind feature flags so that `cargo test` passes without any
infrastructure:

| Flag        | What it enables                                                                |
| ----------- | ------------------------------------------------------------------------------ |
| `db_tests`  | Unit tests that require a live PostgreSQL database                             |
| `k8s_tests` | Full integration tests requiring the Docker Compose stack (implies `db_tests`) |

Tests gated by these flags are annotated as follows:

```rust
#[cfg_attr(not(feature = "db_tests"), ignore)]
fn some_db_test() { ... }

#[cfg_attr(not(feature = "k8s_tests"), ignore)]
fn some_k8s_test() { ... }
```

Do not remove these annotations — they prevent the standard workspace test run from failing when no
stack is available.

## Organization

Integration tests test the HTTP API as a whole and are not organized by source module. Instead, they
are named by endpoint and scenario:

1. **Endpoint** — The HTTP endpoint under test. Examples: `trigger` (`POST /test-run/trigger`),
   `run_status` (`GET /test-run/{id}/status`).
1. **Scenario** — The specific behaviour being verified. Examples:
   `valid_rep_test_plan_returns_200`, `returns_404_for_unknown_run`.

Following the hierarchy above leads to the following generic test case path:

```rust
endpoint_scenario
```

For example: `trigger_valid_rep_test_plan_returns_200`.

## Testing infrastructure

Integration tests use the following tools:

- **[`tokio`][0]** — `#[tokio::test]` for async test functions
- **[`serial_test`][1]** — Forces tests to run serially; required because tests share a single
  running stack and would interfere with each other if run in parallel
- **[`reqwest`][2]** — HTTP client for making requests to the server under test
- **[`assert_fs`][3]** — Temporary filesystem utilities
- **[`simple_test_case`][4]** — Parameterized testing with `#[test_case]` for multiple input
  variations

Define shared test setup and request helpers in a `tests/common/` module. See the relevant
crate-specific page for implementation details.

[0]: https://docs.rs/tokio/latest/tokio/attr.test.html
[1]: https://docs.rs/serial_test/latest/serial_test/
[2]: https://docs.rs/reqwest/latest/reqwest/
[3]: https://docs.rs/assert_fs/latest/assert_fs/
[4]: https://docs.rs/simple_test_case/latest/simple_test_case/

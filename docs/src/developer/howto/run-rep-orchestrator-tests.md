<!-- diataxis-type: howto -->

# How to run rep-orchestrator tests

The `rep-orchestrator` crate partitions its tests into three categories, each requiring a different
level of infrastructure. This guide explains how to run each category.

> **Prerequisites**
>
> - `cargo` (via `rustup`)
> - `make`
> - Docker with Compose support
> - [`tilt`][0]
> - [`kind`][1]

## Unit tests

Unit tests require no external infrastructure and run via the standard workspace test command:

```bash
cargo test -p rep-orchestrator --lib
```

Stack-dependent tests are skipped automatically and reported as ignored in the output.

## DB tests

DB tests require a running PostgreSQL instance.

Start the lightweight database stack:

```bash
cd crates/rep-orchestrator
make db-up
```

In a second terminal, run the DB tests:

```bash
make db-tests
```

Stop the stack when done:

```bash
make db-down
```

The DB tests can also be run against the tilt stack (documented in the section below).

## Integration tests

Integration tests exercise the full HTTP API and require the complete tilt stack (orchestrator
server + database).

Set up the cluster:

```bash
cd crates/rep-orchestrator
make cluster-setup
```

Then start the tilt stack (and view the status of the resources via the link provided in the
output):

```bash
make cluster-up
```

In a second terminal, run the integration tests:

```bash
make integration-tests
```

Stop the stack when done:

```bash
make cluster-teardown
```

## Running all tests as CI does

To replicate the full CI flow — start stack, wait for readiness, run all tests, tear down:

```bash
cd crates/rep-orchestrator
make ci-tests
```

## Feature flags reference

| Flag        | What it gates                                                   |
| ----------- | --------------------------------------------------------------- |
| `db_tests`  | Unit tests requiring a live PostgreSQL database                 |
| `k8s_tests` | Full integration tests in `tests/suite.rs` (implies `db_tests`) |

To test a feature flag directly, use the `make` targets as these also set required environment
variables that are necessary for the tests to work:

```bash
make db-tests
make integration-tests
```

See the [HTTP API integration test reference][2] for details on test organization and the
`TestHelper` infrastructure.

[0]: https://docs.tilt.dev/index.html
[1]: https://kind.sigs.k8s.io/
[2]: ../reference/testing/http-tests.md

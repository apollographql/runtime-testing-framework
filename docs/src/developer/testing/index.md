# Testing

RTF testing follows this [Testing Strategy][0]. How this strategy is followed and specific
approaches will be documented here.

Each crate has its own page within the docs explaining test structure and how tests are implemented.
Testing guidance applicable across all crates is documented on this page.

## Test coverage reports

This section documents how to run a full test coverage report for rtf.

> **Note**: There are no test coverage targets for rtf. The test coverage report showing a high
> percentage of coverage does not mean that rtf is fully tested. The reports are used to identify
> clear gaps in testing coverage where we would have expected to have at least one test case.

The [`cargo-llvm-cov`][1] crate is used to generate test coverage reports for rtf. If using
[`mise`][2] then this will already be installed locally.

To run a new report, first, clear previous test coverage data

```bash
cargo llvm-cov clean --workspace
```

Next, test all the unignored tests

```bash
cargo llvm-cov --no-report
```

Next, test all the ignored tests. This will add to the previous coverage, not replace it.

> **Note**: Valid credentials will need to be supplied for these tests to complete successfully.

```bash
GITHUB_KEY="$GITHUB_KEY" cargo llvm-cov --test github --no-report -- --ignored
APOLLO_KEY="$APOLLO_KEY" APOLLO_SUDO="true" cargo llvm-cov --test starstuff --no-report -- --ignored
APOLLO_KEY="$APOLLO_KEY" APOLLO_SUDO="true" cargo llvm-cov --test imgood_observability_test --no-report -- --ignored
```

Finally, generate the report. It should open in your browser.

```bash
cargo llvm-cov report --open
```

The report should be manually inspected to ensure all branches that should have at least one test
case are covered. Two points to consider

1. 100% coverage for any given area should not be treated as "this is fully tested". It just means
   the code was executed at least once during testing. It is likely that calling code once during
   tests is not sufficient to test all scenarios.
1. In reverse, there may be good reasons why certain areas of the code are not executed during
   tests. Where there is a justifiable reason, it is valid to not cover every single line of code
   during tests.

[0]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/1621688363/RTF+Testing+Strategy
[1]: https://github.com/taiki-e/cargo-llvm-cov
[2]: https://mise.jdx.dev/getting-started.html

<!-- diataxis-type: reference -->

# Testing

As developers of a testing framework, we believe in the importance of comprehensive,
well-structured, automated tests. High-quality testing enables safe, rapid iteration and continuous
delivery. This document serves as our implementation guide for delivering well-tested software.

Each crate has its own page within the docs explaining test structure and how tests are implemented.
Testing guidance applicable across all crates is documented on this page.

## Organizing Tests

Each crate's page describes a hierarchy that should be followed for all tests in that crate.
Examples are included for additional guidance.

The key aim of the hierarchies is to ensure that all tests related to the same level in the
hierarchy are named consistently, making it easy to find related tests. The hierarchies will be
followed using a combination of module structure and test case naming. For module structure, the
primary concern is ensuring that code has the right scope and privacy level. Test structure is a
secondary concern to module structure.

## Test Case Naming

Test cases should be uniquely named within their hierarchy. The test cases should be given
meaningful names that describe what is being tested.

Examples of good test case names:

- `all_fields_specified_and_valid`
- `optional_fields_not_defined`
- `required_field_missing`

Examples of bad test case names:

- `works` - does not specify what works
- `fails` - does not specify what causes the failure
- `test_case` - does not identify what is happening in this test
- `optional_field_specified1` & `optional_field_specified2` - incrementing test cases by integer
  does not differentiate the test cases

## Test Coverage Reports

This section documents how to run a full test coverage report for RTF.

> **Note**: There are no test coverage targets for RTF. A test coverage report showing a high
> percentage of coverage does not mean that RTF is fully tested. The reports are used to identify
> clear gaps in testing coverage where we would have expected to have at least one test case.

The [`cargo-llvm-cov`][0] crate is used to generate test coverage reports for RTF. If you are using
[`mise`][1], then this will already be installed locally.

To run a new report, first clear the previous test coverage data:

```bash
cargo llvm-cov clean --workspace
```

Next, test all the unignored tests:

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

Finally, generate the report. It should open in your browser:

```bash
cargo llvm-cov report --open
```

The report should be manually inspected to ensure that all branches that should have at least one
test case are covered. Two points to consider:

1. 100% coverage for any given area should not be treated as "this is fully tested." It just means
   the code was executed at least once during testing. It is likely that calling code once during
   tests is not sufficient to test all scenarios.
1. Conversely, there may be good reasons why certain areas of the code are not executed during
   tests. Where there is a justifiable reason, it is valid to not cover every single line of code
   during tests.

[0]: https://github.com/taiki-e/cargo-llvm-cov
[1]: https://mise.jdx.dev/getting-started.html

<!-- diataxis-type: reference -->

# Testing

RTF maintains comprehensive, well-structured, automated tests to enable safe, rapid iteration and
continuous delivery. This document covers testing conventions across the codebase.

Testing guidance is split into two areas:

- **Styles of test** — patterns that apply across multiple crates, documenting the tools and
  conventions for each broad category of test
- **Crate-specific pages** — pages for each crate that describe any additional conventions on top of
  the relevant style

## Styles of test

- [Unit tests](unit-tests.md) — tests inside `#[cfg(test)] mod tests` blocks
- [CLI integration tests](cli-tests.md) — testing compiled binaries via `assert_cmd`
- [HTTP API integration tests](http-tests.md) — full-stack tests against a running server
- [Proc macro tests](proc-macro-tests.md) — compile-time testing with `trybuild`

## Crate-specific pages

- [rtf-config](crates/rtf-config.md)
- [rtf-core](crates/rtf-core.md)
- [rtf-integrations](crates/rtf-integrations.md)
- [rtf-derive](crates/rtf-derive.md)
- [rtf-cli](crates/rtf-cli.md)
- [rep-orchestrator](crates/rep-orchestrator.md)
- [rtf-orchestrator-cli](crates/rtf-orchestrator-cli.md)

## Organizing tests

Each style page and crate-specific page describes a naming hierarchy for its test category. The key
aim is consistent naming at each level of the hierarchy, making it easy to find related tests. The
hierarchies use a combination of module structure and test case naming. For module structure, the
primary concern is ensuring code has the right scope and privacy level. Test structure is a
secondary concern to module structure.

## Test case naming

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

## Test coverage reports

RTF uses [`cargo-llvm-cov`][0] for coverage reporting. There are no coverage targets — reports are
used to identify unexpected gaps. See [How to run a test coverage report][1] for the full procedure.

[0]: https://github.com/taiki-e/cargo-llvm-cov
[1]: ../../howto/run-coverage-reports.md

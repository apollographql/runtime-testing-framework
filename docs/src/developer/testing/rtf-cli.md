# rtf-cli

This page documents how tests in the [`rtf-cli`][0] crate are organized and implemented.

## Organization

For the [`rtf-cli`][0] crate, tests are defined in a `tests` directory at the root of the crate.
This allows them to test the execution of the actual CLI as a fully compiled binary. Tests will not
be named based on the module structure for the [`rtf-cli`][0], so tests must be named by creating
modules within the test directory itself.

Comprehensive test coverage for the config and core RTF functionality should be provided by the
other crates. CLI testing should focus on ensuring the commands execute as expected and present the
right information to the end user.

Tests in the [`rtf-cli`][0] crate are organized using the following hierarchy:

1. **Environment dependency** - Some tests in this crate require API tokens to be provided to run
   tests successfully. By default, any such tests should be ignored. To run the tests, environment
   variable(s) containing the token(s) must be provided. To make it easier to run all tests with the
   same dependencies, tests with the same token requirements are stored in the same file. The first
   part of the test path therefore contains the environment dependency identifier. Examples include
   `github`, which stores all the tests that require a GitHub token.
1. **Command** - The command supplied to the CLI. For example, the `run` and `template` commands.
   Where no command is supplied, this is `no_command`. The command is part of the module path.
1. **Flag(s)** - Flags passed to the command that cause significant branching in functionality. For
   example, `check` could be used to describe tests covering the `template --check` case. Where it
   is necessary to identify the flags, these should be specified as the prefix to the test case
   name.
1. **Test class** - A logical grouping of test cases, for example, all valid `template` commands
   using the `--check` flag. This is only defined when using [`simple_test_case`][1] to create
   multiple parameterized tests.
1. **Test case** - The specific test case. This should be uniquely and meaningfully named. Examples
   of test case naming can be seen [here][2].

Following the hierarchy above leads to the following generic test case paths:

```rust
// When simple_test_case is not used
env_dependency::command::tests::flags_test_case

// When simple_test_case is used
env_dependency::command::tests::flags_test_class::test_case
```

### Example - Template and Check

This example demonstrates how to test that a local test plan can be templated and passes a static
check. Working through the hierarchy:

1. **Environment dependency** is omitted. This test requires no tokens to successfully run.
1. **Command** is `template`. This is the command being passed to the CLI.
1. **Flag(s)** is `check`. The `--check` flag is used in this test and adds significant additional
   logic.
1. **Test class** is omitted since [`simple_test_case`][1] is not used.
1. **Test case** is `completes`. A failing test case is `missing_value`.

This leads to the following full test path:

```rust
test template::check_completes
test template::check_missing_value
```

To achieve the structure above, the tests are defined in `tests/template.rs` and organized as
follows:

```rust
mod tests {
  #[test]
  fn check_completes() {
    ...
  }

  #[test]
  fn check_missing_value() {
    ...
  }
}
```

### Example - Run from GitHub

This example demonstrates how to test that the run command can execute a test plan stored in GitHub.
Working through the hierarchy:

1. **Environment dependency** is `github`. This test will require a `GITHUB_TOKEN` to successfully
   run.
1. **Command** is `run`. This is the command being passed to the CLI.
1. **Flag(s)** could be `github`; however, this duplicates the identifier in the environment
   dependency, so it is omitted. The `--github` flag branches the `run` command significantly, so it
   has to be included in the test case path somewhere (just not twice!).
1. **Test class** is omitted since [`simple_test_case`][1] is not used.
1. **Test case** is `completes_no_ref`. An additional test case is `completes_with_ref`.

This leads to the following full test path:

```rust
test github::run::completes_with_no_ref
test github::run::completes_with_ref
```

To achieve the structure above, the tests are defined in `tests/run/github.rs` and organized as
follows:

```rust
mod tests {
  #[test]
  #[ignore = "requires a valid GitHub API Token"]
  fn success_no_ref() {
    ...
  }

  #[test]
  #[ignore = "requires a valid GitHub API Token"]
  fn success_with_ref() {
    ...
  }
}
```

> **Note**: The tests are ignored by default as they require a GitHub token to run successfully.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-cli
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/
[2]: ./index.md#test-case-naming

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
env_dependency::command::flags_test_case

// When simple_test_case is used
env_dependency::command::flags_test_class::test_case
```

### Example - Template and Check

This example demonstrates how to test that a local test plan can be templated and passes a static
check. Working through the hierarchy:

1. **Environment dependency** is omitted. This test requires no tokens to successfully run.
1. **Command** is `template`. This is the command being passed to the CLI.
1. **Flag(s)** is `check`. The `--check` flag is used in this test and adds significant additional
   logic.
1. **Test class** is omitted since [`simple_test_case`][1] is not used.
1. **Test case** is `completes_basic`. A failing test case is `completes_with_cli_values`.

This leads to the following full test path:

```rust
test template::check_completes_basic
test template::check_completes_with_cli_values
```

To achieve the structure above, the tests are defined in `tests/template.rs` and organized as
follows:

```rust
#[test]
fn check_completes_basic() {
  ...
}

#[test]
fn check_completes_with_cli_values() {
  ...
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
1. **Test case** is `github_flag_completes`. An additional test case is
   `github_flag_produces_expected_output`.

This leads to the following full test path:

```rust
test github::run::github_flag_completes
test github::run::github_flag_produces_expected_output
```

To achieve the structure above, the tests are defined in `tests/github/run.rs` and organized as
follows:

```rust
#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_completes() {
  ...
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_produces_expected_output() {
  ...
}
```

> **Note**: The tests are ignored by default as they require a GitHub token to run successfully.

## Implementation

The [`rtf-cli`][0] crate uses specialized testing tools and patterns focused on testing CLI
behavior, command execution, and user-facing functionality.

### Testing Infrastructure

The crate leverages the following key testing tools:

- **[`assert_cmd`][3]** - Provides utilities for testing command-line applications, including
  spawning commands and asserting on their output and exit status
- **[`assert_fs`][4]** - Offers temporary filesystem testing utilities for file operations and
  directory management
- **[`predicates`][5]** - Provides composable assertion predicates for more expressive test
  assertions on command outputs
- **[`indoc`][6]** - Allows clean multi-line string literals in tests, particularly useful for
  expected error messages
- **[`simple_test_case`][1]** - Enables parameterized testing with `#[test_case]` attributes for
  testing multiple input variations

### Command Testing Patterns

The CLI tests focus on three main areas of functionality:

1. **Command execution and argument parsing** - Testing that commands run correctly with various
   flag combinations
2. **Output validation** - Ensuring the CLI produces correct stdout/stderr content
3. **Exit status verification** - Checking that commands succeed or fail with appropriate exit codes

#### Basic Command Testing

Tests verify that commands are executable and respond appropriately to basic invocations:

```rust
#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").assert();
    
    res.stderr(contains("Usage: rtf template"));
}
```

#### Parameterized Testing for Multiple Scenarios

Complex scenarios use parameterized testing to cover multiple input variations efficiently:

```rust
#[test_case("command-from-spec"; "command from spec")]
#[test_case("matrix-values"; "matrix values")]
#[test_case("resolved-values"; "resolved values")]
#[test]
fn check_completes_basic(test_plan_dir: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("template")
        .arg(format!("resources/valid/{test_plan_dir}/test-plan.yaml"))
        .arg("--check")
        .assert();
        
    res.success().stdout(contains("name:"));
}
```

### Temporary Directory Management

The crate implements a custom `CmdWithTmpDir` utility that combines command execution with temporary
directory management:

```rust
pub struct CmdWithTmpDir {
    cmd: Command,
    tmp: TempDir,
}
```

This ensures that test files and directories are properly cleaned up after test execution while
providing convenient access to both the command and the temporary filesystem state.

### Error Testing Strategy

CLI error testing focuses on user-facing error messages and exit codes:

1. **Input validation errors** - Testing malformed YAML, missing files, and invalid configurations
2. **Templating errors** - Ensuring template resolution failures are properly reported
3. **Static analysis failures** - Testing the `--check` flag's validation logic
4. **Runtime execution errors** - Verifying that script execution failures are handled gracefully

Error tests are heavily parameterized to cover multiple failure scenarios:

```rust
#[test_case(
    "missing-values.yaml",
    "Missing template values definitions";
    "missing values"
)]
#[test_case(
    "unknown-values.yaml", 
    "Unknown templating value";
    "unknown values"
)]
#[test]
fn templating_fails(file: &str, err_contains: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg(format!("resources/invalid/templating/{file}"))
        .assert();
        
    res.stderr(contains(format!("Templating failed\n{err_contains}")));
}
```

### Integration Testing Approach

Since the [`rtf-cli`][0] crate tests are integration tests, they focus on end-to-end functionality
rather than unit testing individual functions. This includes:

- **Full command execution** - Running the compiled binary with real arguments
- **File system interactions** - Testing with actual YAML files and directory structures
- **Environment isolation** - Using `env_clear()` to ensure consistent test conditions
- **Output capture and validation** - Asserting on complete stdout/stderr content

### Test Resource Management

Test data is organized in a structured resource hierarchy:

- `resources/valid/` - Contains complete, valid test plans for success scenarios
- `resources/invalid/` - Organized by failure category (load-and-resolve, templating, checks, run)

Each test plan directory is self-contained with all necessary files, enabling realistic integration
testing scenarios.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-cli
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/
[2]: ./index.md#test-case-naming
[3]: https://docs.rs/assert_cmd/latest/assert_cmd/
[4]: https://docs.rs/assert_fs/latest/assert_fs/
[5]: https://docs.rs/predicates/latest/predicates/
[6]: https://docs.rs/indoc/latest/indoc/

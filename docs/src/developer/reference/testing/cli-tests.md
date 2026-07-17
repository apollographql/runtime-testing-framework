<!-- diataxis-type: reference -->

# CLI integration tests

CLI tests verify the compiled binary's behavior end-to-end: argument parsing, output formatting,
exit codes, and filesystem interactions. They live in a `tests/` directory at the crate root and
compile the binary rather than calling functions directly.

## Organization

CLI tests are not organized by source module. Modules within the `tests/` directory create the
naming hierarchy:

1. **Environment dependency** — Tests that require API tokens are grouped by their credential
   requirement. Files named after the command they test have no external dependency. Examples:
   `github` (requires `GITHUB_TOKEN`), `graphos` (requires `APOLLO_KEY`).
1. **Command** — The CLI subcommand under test. Examples: `template`, `run`, `remote`. Where no
   command is supplied, use `no_command`.
1. **Flag(s)** — Flags that cause significant branching in functionality, added as a prefix to the
   test case name. Omit if the flag is already implied by the environment dependency.
1. **Test class** — A grouping of parameterized cases defined with [`simple_test_case`][0].
1. **Test case** — The specific scenario, uniquely and meaningfully named. See
   [test case naming][1].

Following the hierarchy above leads to the following generic test case paths:

```rust
// Without simple_test_case
env_dependency::command::flags_test_case

// With simple_test_case
env_dependency::command::flags_test_class::test_case
```

## Testing infrastructure

CLI tests use the following tools:

- **[`assert_cmd`][2]** — Spawns the compiled binary and asserts on stdout, stderr, and exit status
- **[`assert_fs`][3]** — Temporary filesystem utilities for managing test directories
- **[`predicates`][4]** — Composable assertion predicates for output matching
- **[`indoc`][5]** — Clean multi-line expected output strings
- **[`simple_test_case`][0]** — Parameterized testing across multiple input variations
- **`cargo_bin_cmd!`** — Macro that builds a command for the target binary

## Ignoring credential-dependent tests

Tests that require API tokens must be ignored by default. The `#[ignore]` attribute takes a reason
string explaining which credential is needed:

```rust
#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_completes() {
    // ...
}
```

This keeps `cargo test` fast and dependency-free. To run these tests, supply the credential as an
environment variable and pass `-- --ignored` or `-- --include-ignored` to cargo.

## Temporary directory management

`CmdWithTmpDir` combines a command with a temporary directory, ensuring filesystem state is cleaned
up after each test:

```rust
pub struct CmdWithTmpDir {
    cmd: Command,
    tmp: TempDir,
}
```

Use this when a test must write output files and the test needs to inspect them.

## Test resources

Test data lives under `resources/` at the crate root, organized by resource type and validity:

```text
resources/
└── <resource_type>/
    ├── valid/    — complete, valid test plans for success scenarios
    └── invalid/  — organized by failure category (load-and-resolve, templating, checks, run)
```

Each test plan directory is self-contained with all necessary files.

[0]: https://docs.rs/simple_test_case/latest/simple_test_case/
[1]: ./index.md#test-case-naming
[2]: https://docs.rs/assert_cmd/latest/assert_cmd/
[3]: https://docs.rs/assert_fs/latest/assert_fs/
[4]: https://docs.rs/predicates/latest/predicates/
[5]: https://docs.rs/indoc/latest/indoc/

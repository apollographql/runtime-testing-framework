<!-- diataxis-type: reference -->

# rtf-cli

This page documents how tests in the [`rtf-cli`][0] crate are organized and implemented. The crate
follows the [CLI integration test style][1]. This page describes the additional conventions that
apply within `rtf-cli`.

## Organization

Tests in `rtf-cli` use the environment dependency → command → flags → test class → test case
hierarchy described in the [CLI integration test style][1].

The key convention specific to this crate: when a flag identifier duplicates the environment
dependency identifier (e.g. a `--github` flag inside the `github` test module), omit the flag from
the test case name. The environment dependency already encodes it.

```rust
// Environment dependency is "github", command is "run"
// Flag "--github" is omitted — already implied by the module
test github::run::github_flag_completes        // good
test github::run::github_github_flag_completes // bad — duplicated identifier
```

## Ignored tests

Tests with external API dependencies are `#[ignore]` by default. The reason string must name the
specific credential required:

```rust
#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_completes() { ... }
```

Tests are grouped by their credential requirement into separate files (e.g. `tests/github/`,
`tests/graphos/`), making it easy to run only the tests relevant to a given credential.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-cli
[1]: ./cli-tests.md

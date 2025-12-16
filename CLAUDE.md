🧪

# RTF is the Apollo Runtime Testing Framework - a CLI tool for executing test plans against GraphOS services.

## Build & Test Commands

```bash
# Install from source
cargo install --path crates/rtf-cli

# Run all tests
cargo test

# Run a single test
cargo test test_name

# Run tests in a specific crate
cargo test -p rtf-config

# Run all PR checks locally
mise run pr-all

# Individual PR checks
mise run pr-test       # cargo test
mise run pr-format     # cargo pr-format (rustfmt --check)
mise run pr-clippy     # cargo pr-clippy (clippy with -D warnings)
mise run pr-rustdoc    # check doc links
mise run pr-spell-check # typos
mise run pr-markdown   # dprint check

# Fix formatting
cargo fmt
mise run format-markdown  # markdown files
mise run fix-spelling     # auto-fix typos
```

### Crate Structure

- **rtf-cli**: Command-line interface, integration tests in `tests/` directory
- **rtf-config**: Config file parsing (TestPlan, Environment, Scenario YAML files), providers,
  templating, validation
- **rtf-derive**: Proc macros for config traits
- **rtf-docgen**: Documentation generation utilities
- **rtf-integrations**: Core functionality - GraphOS and GitHub API calls, wrapped for CLI/server
  use

### Config Resolution Flow

1. Load TestPlan YAML
2. Load Scenario/Environment YAML, merge any overrides from TestPlan
3. Template Environment setup section → run static checks
4. Execute setup → use output to template Scenario and Environment teardown
5. Run Scenario → Run teardown

### Key Traits (rtf-config)

- `Template`: locate/resolve templatable fields in config structs
- `Check`: static analysis checks before execution
- `AsUtf8FileContent`: file providers returning single string content
- `ResolveAndWrite`: providers writing arbitrary output

### Provider IO Pattern

All IO in providers must go through the `ResolutionContext` trait passed to methods. This enables
mocking in tests and CLI control over execution.

### Error Handling

Use `ErrorBuilder` from `rtf-config::error` to collect and report multiple errors in batch
operations (templating, validation, static analysis).

## Testing Conventions

Test naming follows hierarchies documented in `docs/src/developer/testing/`. Key patterns:

- **rtf-cli**: `env_dependency::command::flags_test_case`
- **rtf-integrations**: `module::path::tests::function_test_case`

CLI tests are integration tests using `assert_cmd`, `assert_fs`, and `predicates`. Tests requiring
API tokens (GitHub, Apollo) are `#[ignore]` by default.

## Forbidden behaviours

- You MUST NOT delete existing tests without explicit confirmation
  - This applies even if you think the test in question is now obsolete or has compile errors
- You MUST NOT break public APIs without explicit confirmation

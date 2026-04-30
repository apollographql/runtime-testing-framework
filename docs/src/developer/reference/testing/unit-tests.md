<!-- diataxis-type: reference -->

# Unit tests

Unit tests in RTF are defined inside `#[cfg(test)] mod tests` blocks within source files. This keeps
tests close to the code they test and ensures test helper code remains private to the module.

Crate-specific pages describe any additional conventions that apply within that crate.

## Organization

Unit tests follow this naming hierarchy:

1. **Module** — A logical grouping of functionality. Maps directly to the Rust module structure;
   sub-modules may appear in the path (e.g. `providers::file`). The primary concern is the Rust
   code's scope and privacy; test structure is secondary.
1. **Function** — The function or logic under test. Maps to a function or trait method. If not fully
   specified in the module path, it becomes the first prefix in the test case name.
1. **Test class** — A logical grouping of test cases. Defined when using [`simple_test_case`][0] to
   create multiple parameterized tests. Named after the test function when using [`dir_cases`][1].
1. **Test case** — The specific test case. Should be uniquely and meaningfully named. See
   [test case naming][2].

Following the hierarchy above leads to the following generic test case paths:

```rust
// Without simple_test_case
module::path::tests::function_test_case

// With simple_test_case
module::path::tests::function_test_class::test_case
```

## Testing infrastructure

Unit tests commonly use the following tools:

- **[`simple_test_case`][0]** — Parameterized testing with `#[test_case]` for multiple input
  variations sharing the same test logic
- **[`indoc`][3]** — Clean multi-line string literals for inline test data (YAML, JSON, etc.)
- **[`anyhow`][4]** — Ergonomic error handling via `-> anyhow::Result<()>` return type and
  `.context()` for test failure messages

## Parameterized tests

Use `#[test_case]` when multiple inputs should exercise the same logic:

```rust
use simple_test_case::test_case;

#[test_case("{{ valid }}"; "standard")]
#[test_case("{{ with_underscore }}"; "with underscore")]
#[test]
fn field_parse_valid(raw: &str) {
    // Same assertion logic for each case
}
```

The test class is named after the function (`field_parse_valid`). Each test case gets a unique label
in the semicolon position (`"standard"`, `"with underscore"`).

## Test data

**Inline:** Use `indoc!` for small, focused YAML or text:

```rust
let yaml = indoc! {"
    command:
      script: echo hello
"};
```

**Resource files:** Place larger configs and any non-UTF-8 content in a `resources/` directory at
the crate root. Reference them with `include_str!` or by path via `assert_fs`.

[0]: https://docs.rs/simple_test_case/latest/simple_test_case/
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/attr.dir_cases.html
[2]: ./index.md#test-case-naming
[3]: https://docs.rs/indoc/latest/indoc/
[4]: https://docs.rs/anyhow/latest/anyhow/

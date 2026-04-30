<!-- diataxis-type: reference -->

# rtf-integrations

This page documents how tests in the [`rtf-integrations`][0] crate are organized and implemented.
The crate follows the [unit test style][1]. This page describes the additional conventions that
apply within `rtf-integrations`.

## Organization

Tests follow the standard unit test hierarchy. The test class level uses the [`dir_cases`][2] macro
naming convention: when `#[dir_cases]` is used, the test function name becomes the test class (e.g.
`try_parse_ok`, `try_parse_err`), and test case names are derived from the test data file names.

Following the hierarchy leads to the following generic test case paths:

```rust
// With dir_cases
module::path::tests::function_test_class::file_name

// Without simple_test_case
module::path::tests::function_test_case
```

## Trait-based testing with PlatformQuery

The crate's primary testing pattern leverages the `PlatformQuery` trait to test GraphQL response
parsing without requiring HTTP mocking:

```rust
pub trait PlatformQuery: GraphQLQuery {
    type Output;
    type Error;

    fn try_parse(
        data: Self::ResponseData,
        variables: Self::Variables,
    ) -> Result<Self::Output, Self::Error>;
}
```

Tests deserialize JSON directly into the GraphQL response type and call `try_parse` in isolation:

```rust
fn try_parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
    let raw: <OfflineLicense as GraphQLQuery>::ResponseData = serde_json::from_str(contents)?;
    let result = OfflineLicense::try_parse(raw, variables)?;
    assert_eq!(result, "expected-value");
    Ok(())
}
```

## Directory-driven parameterized tests

The [`dir_cases`][2] macro generates test cases from files in a directory:

```rust
#[dir_cases("crates/rtf-integrations/resources/test_data/offline_license/valid")]
#[test]
fn try_parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
    // Test logic runs once per file in the directory
}
```

Each file becomes a separate test case. `path` provides context for error messages; `contents` is
the file text. Adding new test cases is as simple as adding a file to the directory.

## Test data organization

Test data lives in `resources/test_data/`, organized by operation and validity:

```
resources/test_data/
└── <operation>/
    ├── valid/    — complete GraphQL response JSON
    └── invalid/  — response JSON wrapped with an expected_error field
```

**Valid test case** — the raw GraphQL response:

```json
{
  "graph": {
    "account": {
      "offlineLicense": { "jwt": "test-jwt-token" }
    }
  }
}
```

**Invalid test case** — wrapped with `expected_error` so tests can assert the correct error variant
is returned:

```json
{
  "expected_error": "UnknownSupergraph",
  "data": { "graph": null }
}
```

Each error variant in the crate's error types should have at least one corresponding file in the
`invalid/` directory.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-integrations
[1]: ./unit-tests.md
[2]: https://docs.rs/simple_test_case/latest/simple_test_case/attr.dir_cases.html

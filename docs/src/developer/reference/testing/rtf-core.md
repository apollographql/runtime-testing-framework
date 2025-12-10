<!-- diataxis-type: reference -->

# rtf-core

This page documents how tests in the [`rtf-core`][0] crate are organized and implemented.

## Organization

Tests in the [`rtf-core`][0] crate are organized using the following hierarchy:

1. **Module** - A logical grouping of core RTF functionality. Examples include GraphOS (`graphos`)
   and GitHub (`github`) API calls. Since this maps directly to the Rust module structure, there are
   likely to be sub-modules in the path. For example, supergraph details could be nested under the
   path `graphos::supergraph::details`. The primary concern is the logical organization of the Rust
   code for scopes and privacy.
1. **Function** - The function or logic under test. This most likely maps to a function or trait.
   For example, `try_parse` for parsing GraphQL response data. It is unlikely that each of these
   will have a specific file in the Rust code. If this is not fully specified in the module path,
   then it should be specified as the first prefix in the test case name.
1. **Test class** - A logical grouping of test cases, for example, all valid test cases for parsing
   an offline license response. This is defined when using [`simple_test_case`][1] to create
   multiple parameterized tests. When using the [`dir_cases`][3] macro, test classes are named based
   on the test function (e.g., `try_parse_ok` or `try_parse_err`).
1. **Test case** - The specific test case. The test case should be uniquely and meaningfully named.
   When using [`dir_cases`][3], test case names are derived from the test data file names. Examples
   of test case naming can be seen [here][2].

Following the hierarchy above leads to the following generic test case paths:

```rust
// When dir_cases is used
module::path::tests::function_test_class::file_name

// When simple_test_case is not used
module::path::tests::function_test_case
```

### Example - Parsing an Offline License Response

This example demonstrates how to test that an offline license response can be parsed from GraphOS.
Working through the hierarchy:

1. **Module** is `graphos::supergraph::operations::license`. This is where the `OfflineLicense`
   query and its `PlatformQuery` implementation are defined.
1. **Function** is `try_parse`. This is the method that parses the GraphQL response data. The test
   functions are named `try_parse_ok` for success cases and `try_parse_err` for failure cases.
1. **Test class** is `try_parse_ok` or `try_parse_err`. The [`dir_cases`][3] macro is used, so the
   test function name becomes the test class.
1. **Test case** is derived from the file name. For example, `normal` for the `valid/normal.json`
   test data file.

This leads to the following full test paths:

```rust
test graphos::supergraph::operations::license::tests::try_parse_ok::normal
test graphos::supergraph::operations::license::tests::try_parse_err::no_graph
test graphos::supergraph::operations::license::tests::try_parse_err::no_account
test graphos::supergraph::operations::license::tests::try_parse_err::no_offline_license
```

To achieve the structure above, the tests are defined in
`src/graphos/supergraph/operations/license.rs` and organized as follows:

```rust
#[cfg(test)]
mod tests {
    use simple_test_case::dir_cases;

    #[dir_cases("crates/rtf-core/resources/test_data/offline_license/valid")]
    #[test]
    fn try_parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
        // Parse and validate successful response
    }

    #[dir_cases("crates/rtf-core/resources/test_data/offline_license/invalid")]
    #[test]
    fn try_parse_err(path: &str, contents: &str) -> anyhow::Result<()> {
        // Parse and validate error response
    }
}
```

### Example - URL Rewriting in Supergraph Details

This example demonstrates testing a utility function without [`dir_cases`][3]. Working through the
hierarchy:

1. **Module** is `graphos::supergraph::details`. This is where the `rewrite_subgraph_urls` function
   is defined.
1. **Function** is `rewrite_subgraph_urls`. This becomes the first prefix in the test case name.
1. **Test class** is omitted since [`dir_cases`][3] is not used.
1. **Test case** describes the specific scenario, for example `all_urls_updated` or
   `invalid_sdl_returns_none`.

This leads to the following full test paths:

```rust
test graphos::supergraph::details::tests::rewrite_subgraph_urls_all_urls_updated
test graphos::supergraph::details::tests::rewrite_subgraph_urls_invalid_sdl_returns_none
test graphos::supergraph::details::tests::rewrite_subgraph_urls_missing_join_graph_returns_none
```

To achieve the structure above, the tests are defined in `src/graphos/supergraph/details.rs` and
organized as follows:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn rewrite_subgraph_urls_all_urls_updated() {
        // Test happy path
    }

    #[test]
    fn rewrite_subgraph_urls_invalid_sdl_returns_none() {
        // Test error handling
    }
}
```

## Implementation

The [`rtf-core`][0] crate uses a trait-based testing approach that focuses on testing business logic
directly, without requiring HTTP mocking or complex test infrastructure.

### Testing Infrastructure

The crate leverages the following key testing tools:

- **[`simple_test_case`][1]** - Provides the [`dir_cases`][3] macro for directory-driven
  parameterized testing, automatically generating test cases from files in a directory
- **[`anyhow`][4]** - Enables ergonomic error handling in tests with the `-> anyhow::Result<()>`
  return type and contextual error messages via `.context()`
- **[`serde_json`][5]** - Used to deserialize test data from JSON files into GraphQL response types

### Trait-Based Testing Pattern

The crate's primary testing pattern leverages the `PlatformQuery` trait to test GraphQL response
parsing logic without requiring HTTP mocking:

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

Tests call `try_parse` directly with deserialized JSON data, validating the parsing logic in
isolation:

```rust
#[test]
fn try_parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
    // Deserialize JSON directly into the GraphQL response type
    let raw: <OfflineLicense as GraphQLQuery>::ResponseData =
        serde_json::from_str(contents)?;

    // Test the parsing logic directly
    let jwt = OfflineLicense::try_parse(raw, variables)?;

    assert_eq!(jwt, "expected-value");
    Ok(())
}
```

This approach avoids the complexity of HTTP mocking while ensuring comprehensive coverage of the
response parsing logic.

### Directory-Driven Parameterized Tests

The [`dir_cases`][3] macro automatically generates test cases from files in a directory:

```rust
#[dir_cases("crates/rtf-core/resources/test_data/offline_license/valid")]
#[test]
fn try_parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
    // Test logic runs once per file in the directory
}
```

Each file in the specified directory becomes a separate test case, with:

- `path` - The file path (useful for error context)
- `contents` - The file contents as a string

This pattern makes it easy to add new test cases by simply adding files to the test data directory.

### Test Data Organization

Test data is organized in structured directories under `resources/test_data/`:

```
resources/test_data/
├── offline_license/
│   ├── valid/
│   │   └── normal.json
│   └── invalid/
│       ├── no_graph.json
│       ├── no_account.json
│       └── no_offline_license.json
├── supergraph_details/
│   ├── valid/
│   └── invalid/
└── queries/
    └── *.graphql
```

**Valid test cases** contain complete GraphQL response data as JSON:

```json
{
  "graph": {
    "account": {
      "offlineLicense": {
        "jwt": "test-jwt-token"
      }
    }
  }
}
```

**Invalid test cases** wrap the response data with an `expected_error` field:

```json
{
  "expected_error": "UnknownSupergraph",
  "data": {
    "graph": null
  }
}
```

This structure ensures that error tests validate both that an error occurs and that it's the
_correct_ error.

### Schema-Based Validation Tests

Tests involving GraphQL schema manipulation use [`apollo-compiler`][6] for parsing and validation:

```rust
#[test]
fn fix_aliases_adds_aliases_for_duplicate_fields() {
    let schema = Schema::parse_and_validate(SCHEMA, "supergraph.graphql").unwrap();
    let mut doc = ExecutableDocument::parse(&schema, query, "test").unwrap();

    fix_aliases(&mut doc);

    let res = doc.validate(&schema);
    assert!(res.is_ok());
}
```

Schema test data is typically embedded using `include_str!`:

```rust
const SCHEMA: &str = include_str!("../../../../resources/engine-prod-schema.graphql");
```

### Error Testing Strategy

Error testing ensures all error variants are covered and properly handled:

1. **Exhaustive variant coverage** - Each error variant has at least one corresponding test file in
   the `invalid/` directory
2. **Error matching** - Tests validate that the specific expected error variant is returned:

```rust
#[dir_cases("crates/rtf-core/resources/test_data/offline_license/invalid")]
#[test]
fn try_parse_err(path: &str, contents: &str) -> anyhow::Result<()> {
    let ErrCase { expected_error, data } = serde_json::from_str(contents)?;
    let raw = serde_json::from_value(data)?;

    let result = OfflineLicense::try_parse(raw, variables);

    match (expected_error.as_str(), result) {
        ("UnknownSupergraph", Err(FetchErrorCause::UnknownSupergraph)) => (),
        ("UnknownOrganisation", Err(FetchErrorCause::UnknownOrganisation)) => (),
        (expected, actual) => panic!("expected {expected}, got {actual:?}"),
    }

    Ok(())
}
```

### Unit Tests for Utility Functions

Not all tests use [`dir_cases`][3]. Utility functions with deterministic behavior use standard unit
tests:

```rust
#[test]
fn rewrite_subgraph_urls_all_urls_updated() {
    let sdl = include_str!("../../../resources/test_data/simple-supergraph.graphql");
    let url_map = HashMap::from([
        ("products".to_string(), "http://localhost:4001".to_string()),
        ("reviews".to_string(), "http://localhost:4002".to_string()),
    ]);

    let result = rewrite_subgraph_urls(sdl, &url_map);

    assert!(result.is_some());
    let new_sdl = result.unwrap();
    assert!(new_sdl.contains("http://localhost:4001"));
}
```

These tests are named with the `function_scenario` pattern and are grouped together in the `tests`
module.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-core
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/
[2]: ./index.md#test-case-naming
[3]: https://docs.rs/simple_test_case/latest/simple_test_case/attr.dir_cases.html
[4]: https://docs.rs/anyhow/latest/anyhow/
[5]: https://docs.rs/serde_json/latest/serde_json/
[6]: https://docs.rs/apollo-compiler/latest/apollo_compiler/

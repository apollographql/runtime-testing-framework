# rtf-config

This page documents how tests in the [`rtf-config`][0] crate are organized and implemented.

## Organization

Tests in the [`rtf-config`][0] crate are organized using the following hierarchy:

1. **Module** - A logical grouping of config data structures. Examples include providers and config
   files. Since this maps directly to the Rust module structure, there are likely to be sub-modules
   in the path. For example, file providers could be nested under the path `providers::file`. The
   primary concern is the logical organization of the Rust code for scopes and privacy.
1. **Data structures** - Any specific snippet or reusable YAML config; these most likely map to a
   specific type. Examples include specific file providers like `RelativePath` and `GithubFile`. It
   is unlikely that each of these will have a specific file in the Rust code. If these are not fully
   specified in the module path, then they should be specified as the first prefix in the test case
   name.
1. **Functionality** - Any specific functionality implemented for the data structure. This could
   include trait implementations and other functions implemented for the data structure. Examples
   include parsing, template, validate, and resolve for file providers. This should be the second
   prefix in the test case name.
1. **Test class** - A logical grouping of test cases, for example, all valid test cases for parsing
   a file provider. This is only defined when using [`simple_test_case`][1] to create multiple
   parameterized tests.
1. **Test case** - The specific test case. This should be uniquely and meaningfully named. Examples
   of test case naming can be seen [here][2].

Following the hierarchy above leads to the following generic test case paths:

```rust
// When simple_test_case is not used
module::path::tests::data_structure_functionality_test_case

// When simple_test_case is used
module::path::tests::data_structure_functionality_test_class::test_case
```

### Example - Resolving a File Provider

This example demonstrates how to test that the file provider, `RelativePath`, can be resolved into
file content. Working through the hierarchy:

1. **Module** is `provider::file`. This is where the `RelativePath` type is defined.
1. **Data structure** is `relative_path` since this is the specific file provider being tested. This
   becomes the first prefix in the test case name since the module does not uniquely identify the
   data structure.
1. **Functionality** is `resolve`. This becomes the second prefix in the test case name.
1. **Test class** is omitted since [`simple_test_case`][1] is not used.
1. **Test case** is `into_text`. For illustrative purposes, a failing test case, `file_not_text`, is
   included as well.

The full test paths are:

```rust
test provider::file::tests::relative_path_resolve_into_text
test provider::file::tests::relative_path_resolve_file_not_text
```

To achieve the structure above, the tests are defined in `src/providers/file/mod.rs` and organized
as follows:

```rust
mod tests {
  #[test]
  fn relative_path_resolve_success() {
    ...
  }

  #[test]
  fn relative_path_resolve_file_not_text() {
    ...
  }
}
```

### Example - Malformed Template Strings

This example demonstrates how to test that the template string `"{{ template_value }}"` does not
work when similar, but not acceptable, patterns are specified. Working through the hierarchy:

1. **Module** is `templating`. This is where the `Field` type is defined.
1. **Data structure** is `field` since the `Field` type is being tested. This becomes the first
   prefix in the test case name since the module does not uniquely identify the data structure.
1. **Functionality** is `parse` since the template string is evaluated during the parsing of a
   `Field`. This becomes the second prefix in the test case name.
1. **Test class** is `malformed_template_string`. [`simple_test_case`][1] is used since there are
   variations in strings that are tested using the same logic.
1. **Test cases** are a series of uniquely named tests. For example, `no_space_after_value_name` and
   `single_curly_braces`.

The full test paths are:

```rust
test templating::tests::field_parse_malformed_template_string::no_space_after_value_name
test templating::tests::field_parse_malformed_template_string::single_curly_braces
```

To achieve the structure above, the tests are defined in `src/templating.rs` and organized as
follows:

```rust
mod tests {
  use simple_test_case::test_case;
  #[test_case("arg1"; "no_space_after_value_name")]
  #[test_case("arg2"; "single_curly_braces")]
  #[test]
  fn field_parse_malformed_template_string(arg: &str) {
    ...
  }
}
```

## Implementation

The [`rtf-config`][0] crate uses several testing strategies and tools to ensure comprehensive
coverage of configuration parsing, validation, and resolution.

### Testing Infrastructure

The crate leverages the following key testing tools:

- **[`simple_test_case`][1]** - Enables parameterized testing with `#[test_case]` attributes for
  testing multiple input variations with the same test logic
- **`assert_fs`** - Provides temporary filesystem testing utilities for file operations
- **`predicates`** - Offers composable assertion predicates for more expressive test assertions
- **`indoc`** - Allows clean multi-line string literals in tests, particularly useful for YAML
  configurations

### Mock System

The crate implements a flexible mock system through `MockContext<T>` that allows dependency
injection during testing:

```rust
// For HTTP client testing
let mock_ctx = MockContext::with_http_client(&[
    ("https://example.com/api", "200", "response body")
]);

// For GitHub client testing  
let mock_ctx = MockContext::with_github_client("file content");
```

This approach enables testing without external dependencies while maintaining the same interfaces
used in production code.

For mocking interactions with GraphOS, a different approach is used. Rather than mocking the GraphOS
API directly, the tests create mock `SupergraphDetails` and test the content transformation logic:

```rust
#[test]
fn supergraph_resolve_success() {
    // Create mock supergraph details with known test data
    let details = Arc::new(SupergraphDetails {
        graph_id: "test-graph".to_string(),
        variant: "test-variant".to_string(),
        supergraph_sdl: "schema { query: Query }".to_string(),
        subgraphs: vec![/* test subgraphs */],
    });

    let supergraph = GraphosSupergraph {
        graph_ref: Field::Resolved("graph@variant".to_string()),
        with_subgraph_overrides: None,
    };

    // Test the content transformation directly
    let result = supergraph.content_from_details(details);
    assert_eq!(result, "schema { query: Query }");
}
```

This pattern allows testing the core business logic while avoiding the complexity of mocking
external APIs. The `with_supergraph_details` method in the production context handles the API
interaction separately.

### Parameterized Testing Patterns

Tests extensively use `#[test_case]` to cover multiple scenarios efficiently:

```rust
#[test_case("foo"; "ascii")]
#[test_case("BAR"; "upper case")]
#[test_case("世界"; "unicode")]
#[test]
fn field_parse_valid_identifiers(raw: &str) {
    // Test logic handles all cases
}
```

Related test cases are grouped into test classes when using parameterized testing, following the
naming hierarchy described in the [Organization](#organization) section.

### Configuration Testing Strategy

The crate employs a systematic approach to testing configuration handling:

1. **Parsing Tests** - Verify YAML deserialization works correctly for valid inputs and fails
   appropriately for invalid ones
2. **Template Resolution Tests** - Ensure the `{{ value }}` templating system works across all
   supported data types. In most cases specific tests are not required for the `Template` trait
   since this is derived with a proc macro. Whenever there is a custom implementation of this trait
   then tests are defined.
3. **Validation Tests** - Check that configuration validation catches common errors and edge cases
4. **Resolve Tests** - Test that providers and configuration resolve and execute correctly.

### Test Data Management

Test data is organized in two main ways:

- **Inline test data** using `indoc!` for small, focused examples
- **Resource files** in `resources/` for larger, realistic configurations and any non UTF-8 files.

### Error Testing

The crate places strong emphasis on testing error conditions:

- **Malformed input testing** - Invalid YAML, incorrect template syntax, missing required fields
- **Boundary condition testing** - Empty values, special characters, unicode handling
- **Error message validation** - Ensuring error messages are helpful for users

Error tests follow the same organizational patterns as success tests, often using parameterized
testing to cover multiple error scenarios efficiently.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/
[2]: ./index.md#test-case-naming

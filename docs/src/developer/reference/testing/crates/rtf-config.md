<!-- diataxis-type: reference -->

# rtf-config

This page documents how tests in the [`rtf-config`][0] crate are organized and implemented. The
crate follows the [unit test style][1]. This page describes the additional conventions that apply
within `rtf-config`.

## Organization

Tests in `rtf-config` extend the standard unit test hierarchy with an extra level between Module and
Function to account for the data-centric structure of the crate:

1. **Module** — A logical grouping of config data structures (e.g. `providers::file`).
1. **Data structure** — The specific type under test (e.g. `RelativePath`, `GithubFile`). If not
   fully specified in the module path, this becomes the first prefix in the test case name.
1. **Functionality** — The trait or function being tested (e.g. `parse`, `template`, `resolve`).
   This becomes the second prefix in the test case name.
1. **Test class** — A logical grouping of parameterized cases created with [`simple_test_case`][2].
1. **Test case** — The specific scenario, uniquely and meaningfully named. See
   [test case naming][3].

Following the hierarchy above leads to the following generic test case paths:

```rust
// Without simple_test_case
module::path::tests::data_structure_functionality_test_case

// With simple_test_case
module::path::tests::data_structure_functionality_test_class::test_case
```

## Mock system

The crate implements a `MockContext<T>` that allows dependency injection for HTTP and GitHub clients
during testing:

```rust
// For HTTP client testing
let mock_ctx = MockContext::with_http_client(&[
    ("https://example.com/api", "200", "response body")
]);

// For GitHub client testing
let mock_ctx = MockContext::with_github_client("file content");
```

This enables testing without external dependencies while using the same interfaces as production
code.

For GraphOS interactions, the crate tests against mock `SupergraphDetails` rather than mocking the
GraphOS API directly:

```rust
#[test]
fn supergraph_resolve_success() {
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

    let result = supergraph.content_from_details(details);
    assert_eq!(result, "schema { query: Query }");
}
```

This tests the core content transformation logic while keeping API interaction complexity in the
production context method (`with_supergraph_details`).

## Configuration testing strategy

Config handling is tested in four phases:

1. **Parsing** — YAML deserialization works correctly for valid inputs and fails for invalid ones
1. **Template resolution** — The `{{ variable }}` templating system works across all supported
   types. Derived implementations of `Template` generally don't need separate tests; only custom
   implementations require them.
1. **Validation** — Config validation catches common errors and edge cases
1. **Resolve** — Providers and configuration resolve and execute correctly

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config
[1]: ./unit-tests.md
[2]: https://docs.rs/simple_test_case/latest/simple_test_case/
[3]: ./index.md#test-case-naming

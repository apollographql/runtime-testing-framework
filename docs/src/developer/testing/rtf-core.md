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
   For example, `generate_canned_ops` in `graphos`. It is unlikely that each of these will have a
   specific file in the Rust code. If this is not fully specified in the module path, then it should
   be specified as the first prefix in the test case name.
1. **Test class** - A logical grouping of test cases, for example, all valid test cases for fetching
   an offline license from GraphOS. This is only defined when using [`simple_test_case`][1] to
   create multiple parameterized tests.
1. **Test case** - The specific test case. This should be uniquely and meaningfully named. Examples
   of test case naming can be seen [here][2].

Following the hierarchy above leads to the following generic test case paths:

```rust
// When simple_test_case is not used
module::path::tests::function_test_case

// When simple_test_case is used
module::path::tests::function_test_class::test_case
```

### Example - Fetching an Offline License from GraphOS

This example demonstrates how to test that an offline license can be fetched from GraphOS. Working
through the hierarchy:

1. **Module** is `graphos::supergraph::operations::license`. This is where the `offline_license`
   function is defined.
1. **Function** is `offline_license`. This becomes the first prefix in the test case name since the
   module does not uniquely identify the data structure.
1. **Test class** is omitted since [`simple_test_case`][1] is not used.
1. **Test case** is `retrieved`.

This leads to the following full test path:

```rust
test graphos::supergraph::operations::license::tests::offline_license_retrieved
```

To achieve the structure above, the tests are defined in
`src/graphos/supergraph/operations/license.rs` and organized as follows:

```rust
mod tests {
  #[test]
  fn offline_license_retrieved() {
    ...
  }
}
```

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-core
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/
[2]: ./index.md#test-case-naming

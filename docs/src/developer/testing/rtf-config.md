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

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config
[1]: https://docs.rs/simple_test_case/latest/simple_test_case/
[2]: ./index.md#test-case-naming

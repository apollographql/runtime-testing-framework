<!-- diataxis-type: reference -->

# rtf-derive

This page documents how tests in the [`rtf-derive`][0] crate are organized and implemented.

This crate defines a [proc macro][1] for deriving the `Template` trait defined in the
[`rtf-config`][2] crate. The considerations for testing a proc macro are different from testing a
standard Rust crate.

## Organization

For the [`rtf-derive`][0] crate, tests are defined in a `tests` directory at the root of the crate.
This is because we need to test how the derive macro is applied to a variety of Rust containers. We
also need to test the conditions under which the macro will fail to compile. Tests will not be named
based on the module structure for the [`rtf-derive`][0], so tests must be named by creating modules
within the test directory itself.

Tests in the [`rtf-derive`][0] crate are organized into the following modules:

1. **`compile`** - The `compile` tests check the conditions under which the derive macro will fail
   to compile into valid Rust.
1. **`template`** - The `template` tests check the implementation of the `Template` trait works as
   expected when the derive macro is applied to a Rust container.

The structure of the tests within each of these modules is discussed below.

### Compile tests

There are two types of compile test: `pass` and `fail`. These are organized into separate
directories within the `compile` subdirectory of `tests`.

The `pass` tests work by importing the `*.rs` files into the `compile` module. These are not
executed by `cargo test` but will fail if `cargo check` is run and they do not result in valid Rust.
If any of the code in these modules fails to compile then the test should be considered to have
failed.

The `fail` tests use the [`trybuild`][3] crate. This is specifically to test the conditions under
which we expect the derive macro to fail to compile valid Rust. The `*.rs` files in the `fail`
directory should be used as examples of Rust containers that should not have the `Template` derive
macro applied to them.

> **Note**: The `trybuild` crate automatically generates the expected error messages when you run
> the tests with no error messages defined. There is a chance that updates to the Rust version used
> to run tests changes the format of the error messages produced and causes tests to fail. When this
> happens, delete the `.stderr` files associated with the failing test files and rerun `cargo test`.
> This should generate a `wip` directory at the root of the repository with the new `.stderr` files.
> Copy these into the `fail` directory and re-run `cargo test`. Your tests should now pass.

### Template tests

The `template` tests follow a typical Rust unit testing approach. The module defines various valid
Rust containers the derive macro can be applied to. It then checks, for each of those containers,
that the `Template` trait methods work as expected. The [`simple_test_case`][4] crate is used to
parameterize these containers into a single test function.

The tests are organized using the following hierarchy:

1. **Trait method** - The method of the `Template` trait under test. In cases where the method can
   error, this will also identify the error conditions.
1. **Data structure** - The Rust container under test. This is defined as part of the
   [`simple_test_case`][4] test case name.
1. **Test case** - Additional information required to uniquely identify the test. This is defined as
   part of the [`simple_test_case`][4] test case name.

Following the hierarchy above leads to the following generic test case paths:

```rust
// When simple_test_case is used
template::tests::trait_method::data_structure_test_case
```

## Implementation

The [`rtf-derive`][0] crate employs specialized testing strategies tailored to procedural macro
testing requirements.

### Testing infrastructure

The crate leverages the following key testing tools:

- **[`trybuild`][3]** - Enables testing of procedural macro compilation failures with precise error
  message validation
- **[`simple_test_case`][4]** - Provides parameterized testing capabilities for testing the derive
  macro across multiple Rust container types
- **Compile-time testing** - Uses the Rust compiler itself as a testing tool to validate successful
  macro expansion

### Procedural macro testing strategy

Testing procedural macros requires a different approach from standard unit testing:

1. **Compilation Success Testing** - Verify that the macro successfully generates valid Rust code
   for supported container types
2. **Compilation Failure Testing** - Ensure the macro fails appropriately with helpful error
   messages for unsupported scenarios
3. **Generated Code Testing** - Validate that the generated `Template` trait implementations behave
   correctly at runtime

### Error testing with trybuild

The `fail` tests use `trybuild` to capture and validate compilation error messages:

```rust
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/fail/*.rs");
}
```

This approach ensures that:

- Error messages remain consistent across Rust versions
- Users receive helpful diagnostic information when the macro cannot be applied
- The macro fails early with clear explanations rather than generating invalid code

### Parameterized container testing

The `template` tests extensively use `#[test_case]` to verify the derive macro works across
different Rust container types:

```rust
#[test_case("Struct"; "basic_struct")]
#[test_case("TupleStruct"; "tuple_struct")]
#[test_case("UnitStruct"; "unit_struct")]
#[test]
fn template_method_works(container_type: &str) {
    // Test the generated Template implementation
}
```

This pattern allows comprehensive testing of the macro's behavior across the full range of supported
Rust syntax.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-derive
[1]: https://doc.rust-lang.org/reference/procedural-macros.html
[2]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-config
[3]: https://docs.rs/trybuild/latest/trybuild/
[4]: https://docs.rs/simple_test_case/latest/simple_test_case/
[5]: ./index.md#test-case-naming

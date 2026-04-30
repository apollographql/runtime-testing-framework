<!-- diataxis-type: reference -->

# Proc macro tests

Proc macro testing requires a different approach from standard unit testing because the code under
test runs at compile time.

## Test structure

Tests live in a `tests/` directory at the crate root, split into two modules:

```
tests/
├── compile.rs          # trybuild pass/fail compilation tests
├── compile/
│   ├── pass/           # .rs files that must compile successfully
│   └── fail/           # .rs files that must fail to compile, with .stderr files
└── template.rs         # runtime tests for the generated trait implementations
```

## Compile tests

There are two types of compile test:

**Pass tests** — `.rs` files imported into the `compile` module. They are not executed but will
cause a test failure if they do not produce valid Rust under `cargo check`. These verify that the
macro generates correct code for supported input types.

**Fail tests** — `.rs` files that should not compile. The [`trybuild`][0] crate captures the
compiler's error output and compares it against a `.stderr` file:

```rust
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/fail/*.rs");
}
```

This ensures users receive helpful, stable error messages when the macro is applied incorrectly.

### Updating .stderr files after a Rust version change

Rust may change compiler error message formatting between versions, causing fail tests to break. See
[How to update trybuild .stderr files][1] for the procedure.

## Template tests

The `template` module tests that the generated `Template` trait implementations work correctly at
runtime. [`simple_test_case`][2] is used to run the same assertions against multiple Rust container
types (structs, tuple structs, enums, etc.):

```rust
#[test_case("Struct"; "basic_struct")]
#[test_case("TupleStruct"; "tuple_struct")]
#[test]
fn try_template_method_works(container_type: &str) {
    // ...
}
```

Tests are organized using the following hierarchy:

1. **Trait method** — The `Template` method under test, including whether the error path is being
   tested.
1. **Data structure** — The Rust container type, defined as the test case name.
1. **Test case** — Additional context to uniquely identify the test.

Following the hierarchy above leads to the following generic test case path:

```rust
template::tests::trait_method::data_structure_test_case
```

[0]: https://docs.rs/trybuild/latest/trybuild/
[1]: ../../howto/update-trybuild-stderr-files.md
[2]: https://docs.rs/simple_test_case/latest/simple_test_case/

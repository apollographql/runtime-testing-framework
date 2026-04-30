<!-- diataxis-type: howto -->

# How to update trybuild .stderr files for rtf-derive

Rust may change compiler error message formatting between versions. When this causes [`trybuild`][0]
fail tests to break in the `rtf-derive` crate, update the `.stderr` files to match the new output.

> **Prerequisites**
>
> - The current Rust toolchain installed via `rustup`

Delete the `.stderr` files for the failing tests:

```bash
rm crates/rtf-derive/tests/compile/fail/*.stderr
```

Run `cargo test`. `trybuild` writes new `.stderr` files to a `wip/` directory at the repository
root:

```bash
cargo test -p rtf-derive
```

Copy the generated files into `tests/compile/fail/`:

```bash
cp wip/*.stderr crates/rtf-derive/tests/compile/fail/
```

Confirm the tests pass:

```bash
cargo test -p rtf-derive
```

[0]: https://docs.rs/trybuild/latest/trybuild/

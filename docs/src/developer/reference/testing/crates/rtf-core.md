<!-- diataxis-type: reference -->

# rtf-core

The [`rtf-core`][0] crate follows the standard [unit test style][1]. Tests live alongside source
code in `#[cfg(test)] mod tests` blocks.

The crate's tests focus on execution logic and diff/comparison output. Test data is typically
generated inline or loaded via `include_str!` from resource files.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/crates/rtf-core
[1]: ./unit-tests.md

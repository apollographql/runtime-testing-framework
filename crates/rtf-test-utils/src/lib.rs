//! Config file parsing and validation for the Apollo Runtime Testing Framework.
#![warn(
    clippy::complexity,
    clippy::correctness,
    clippy::style,
    future_incompatible,
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    rustdoc::all
)]
#![deny(clippy::undocumented_unsafe_blocks)]
use serde_json::ser::{PrettyFormatter, Serializer};

/// The default pretty string method for serde_json prints to a string with a 2 space indent
/// The textar files have 4 space indents. This function provides a method to print the pretty
/// json string with a customisable space indent so that assert_eq! works when comparing with
/// json stored in the txtar files.
pub fn to_pretty_json_with_indent<T: ?Sized + serde::Serialize>(
    value: &T,
    indent: usize,
) -> String {
    let spaces = vec![b' '; indent];
    let formatter = PrettyFormatter::with_indent(&spaces);
    let mut buf = Vec::new();
    let mut ser = Serializer::with_formatter(&mut buf, formatter);
    value.serialize(&mut ser).unwrap();

    String::from_utf8(buf).unwrap()
}

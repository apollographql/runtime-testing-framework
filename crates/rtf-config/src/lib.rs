//! Config file parsing and validation for the Apollo Runtime Testing Framework.
#![warn(
    clippy::complexity,
    clippy::correctness,
    clippy::style,
    future_incompatible,
    missing_debug_implementations,
    // missing_docs,
    rust_2018_idioms,
    rustdoc::all
)]
#![deny(clippy::undocumented_unsafe_blocks)]

/// Re-exports the current crate as `rtf_config`, allowing other modules within the crate
/// to refer to it using the `rtf_config` name. This is useful for the Template proc macro
/// in rtf_derive and allows us to refer to the trait using the ::rtf_config:: path.
#[allow(unused_extern_crates)]
extern crate self as rtf_config;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod checks;
pub mod context;
pub mod error;
pub mod formats;
#[cfg(test)]
mod mock_context;
pub mod providers;
pub mod templating;

pub use providers::file::Source;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ValueDefinition {
    /// The templating name for this value
    pub name: String,
    /// A brief description of how this value is used
    pub description: String,
    /// An optional default to use if this value is not provided in the parent test plan
    #[serde(default)]
    pub default: Option<templating::Scalar>,
}

/// A function for merging yaml overrides with the base config. It is expected
/// behaviour that lists will be a combination of the base and override lists
/// for the same key. This function performs no deduplication.
pub(crate) fn merge_yaml(overrides: serde_yaml::Value, base: &mut serde_yaml::Value) {
    use serde_yaml::Value;

    match (overrides, base) {
        // If both values are mappings we add all keys from src into dst.
        (Value::Mapping(override_map), Value::Mapping(base_map)) => {
            for (key, override_val) in override_map.into_iter() {
                // If a key is present in both maps then we recursively merge the values,
                // otherwise we just insert the src key into dst directly.
                match base_map.get_mut(&key) {
                    Some(base_val) => merge_yaml(override_val, base_val),
                    None => _ = base_map.insert(key, override_val),
                };
            }
        }

        // If both values are sequences we append overrides to base
        (Value::Sequence(override_seq), Value::Sequence(base_seq)) => {
            base_seq.extend_from_slice(&override_seq)
        }

        // Otherwise we replace base with overrides
        (overrides, base) => *base = overrides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use simple_test_case::test_case;

    #[test_case(
        "foo: [1]",
        "foo: [2]",
        indoc!(r#"
        foo:
        - 1
        - 2"#);
        "arrays concatenate"
    )]
    #[test_case(
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#),
        indoc!(r#"
        foo:
          baz: 42"#),
        indoc!(r#"
        foo:
          bar: 1
          baz: 42"#);
        "maps override individual keys"
    )]
    #[test_case(
        "foo: 1",
        "foo: 2",
        "foo: 2";
        "scalar scalar overwrites"
    )]
    #[test_case(
        "foo: 1",
        "foo: [2]",
        indoc!(r#"
        foo:
        - 2"#);
        "scalar array overwrites"
    )]
    #[test_case(
        "foo: 1",
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#),
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#);
        "scalar map overwrites"
    )]
    #[test_case(
        "foo: [1]",
        "foo: 2",
        "foo: 2";
        "array scalar overwrites"
    )]
    #[test_case(
        "foo: [1]",
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#),
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#);
        "array map overwrites"
    )]
    #[test_case(
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#),
        "foo: 2",
        "foo: 2";
        "map scalar overwrites"
    )]
    #[test_case(
        indoc!(r#"
        foo:
          bar: 1
          baz: 2"#),
        "foo: [2]",
        indoc!(r#"
        foo:
        - 2"#);
        "map array overwrites"
    )]
    #[test]
    fn merge_yaml_returns_expected_structure(base: &str, overrides: &str, expected: &str) {
        let mut base: serde_yaml::Value = serde_yaml::from_str(base).unwrap();
        let overrides: serde_yaml::Value = serde_yaml::from_str(overrides).unwrap();

        merge_yaml(overrides, &mut base);

        let merged = serde_yaml::to_string(&base).unwrap().trim().to_string();

        assert_eq!(merged, expected);
    }
}

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
use serde::{Deserialize, Serialize};

pub mod checks;
pub mod context;
pub mod error;
pub mod formats;
pub mod providers;
pub mod templating;
#[cfg(test)]
mod txtar_context;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ValueDefinition {
    pub name: String,
    pub description: String,
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

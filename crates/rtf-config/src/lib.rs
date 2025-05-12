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
use serde::Deserialize;

mod formats;
mod providers;

pub use formats::{EnvironmentConfig, RawEnvironmentConfig};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ValueSchema {
    pub name: String,
    pub description: String,
    pub schema: Option<serde_json::Value>,
}

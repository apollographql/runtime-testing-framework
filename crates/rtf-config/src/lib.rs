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

pub mod formats;
pub mod providers;
pub mod templating;
pub mod validation;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ValueDefinition {
    pub name: String,
    pub description: String,
}

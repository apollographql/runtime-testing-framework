//! The core functionality of the Apollo Runtime Testing Framework.
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

pub mod platform_query;
pub mod supergraph;

/// The maximum number of queries to run in parallel querying the platform API.
pub const N_PARALLEL_FETCH: usize = 20;

//! Declarations for named GraphOS environments that a test plan can resolve graph_refs against.
//!
//! A single test plan may need to pull data from more than one GraphOS instance in one run — for
//! example, customer graphs like `Expedia@prod` live in production GraphOS while Apollo's own
//! `engine@prod` lives in staging GraphOS.  To support that, a plan can declare one or more
//! named environments at the top level:
//!
//! ```yaml
//! graphos_environments:
//!   apollo_staging:
//!     url: https://graphql-staging.api.apollographql.com/api/graphql
//!     api_key_env_var: APOLLO_KEY_STAGING
//!     sudo: false
//! ```
//!
//! Any GraphOS file provider can then opt into a declared environment via its `graphos_env` field.
//! Providers that don't specify one fall through to the implicit `default` environment, which is
//! synthesized from `APOLLO_KEY` (and optionally `APOLLO_SUDO`) and points at prod.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A declared GraphOS environment that a test plan can resolve graph_refs against.
///
/// Environments are purely data — no hardcoded enum of well-known environments in the engine.
/// New Apollo environments (dev0, dev1, ...) never require an engine change; they're just more
/// entries in a test plan's `graphos_environments` block.
///
/// These fields are not templatable — the URL, env var name, and sudo flag are fixed per
/// environment and don't vary across matrix variants.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct GraphosEnvironment {
    /// The GraphOS API endpoint (e.g. `https://graphql.api.apollographql.com/api/graphql`).
    pub url: String,
    /// The env var name the rtf process reads to obtain the API key for this environment.
    pub api_key_env_var: String,
    /// Whether API requests to this environment should include the `apollo-sudo: true` header.
    ///
    /// Defaults to false if unset.
    #[serde(default)]
    pub sudo: bool,
}

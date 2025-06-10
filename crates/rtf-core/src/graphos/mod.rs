//! Logic for working with Apollo GraphOS
use apollo_compiler::validation::DiagnosticList;
use std::{fmt, io};

pub mod platform_query;
pub mod supergraph;

/// Error variants that we can encounter when interacting with Apollo graphOS.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error variants that we can encounter when making graphQL requests to the platform API.
    #[error(transparent)]
    Fetch(#[from] supergraph::FetchError),

    /// A graphQL error was encountered while attempting to pull supergraph details
    #[error(transparent)]
    Graphql(#[from] platform_query::Error),

    /// IO errors
    #[error(transparent)]
    Io(#[from] io::Error),

    /// GraphQL parsing errors
    #[error("error parsing GraphQL document")]
    InvalidDocument {
        /// Errors encountered while parsing or validating a GraphQL document
        errors: DiagnosticList,
        /// Context for the document that the errors occurred in
        context: String,
    },

    /// JSON parsing errors
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Alias for a [Result][std::result::Result] where the error variant is an [Error].
pub type Result<T> = std::result::Result<T, Error>;

/// An API client backed by [reqwest::Client] that can make requests to the Apollo platform API.
pub struct PlatformClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) url: String,
    pub(crate) api_key: String,
    pub(crate) sudo: bool,
}

// Custom Debug impl to prevent us dumping the api key
impl fmt::Debug for PlatformClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformClient")
            .field("url", &self.url)
            .finish()
    }
}

//! Logic for working with Apollo GraphOS
use std::fmt;

pub mod platform_query;
pub mod supergraph;

/// An API client backed by [reqwest::Client] that can make requests to the Apollo platform API.
pub struct PlatformClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) url: String,
    pub(crate) api_key: String,
}

// Custom Debug impl to prevent us dumping the api key
impl fmt::Debug for PlatformClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformClient")
            .field("url", &self.url)
            .finish()
    }
}

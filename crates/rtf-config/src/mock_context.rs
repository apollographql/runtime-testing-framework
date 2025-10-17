#[cfg(test)]
use rtf_core::{github, graphos::platform_query};

/// Used to implement [ResolutionContext] in tests where no client is needed.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct NullClient;

#[cfg(test)]
impl platform_query::Client for NullClient {
    async fn post_operation(
        &self,
        _body: &impl serde::Serialize,
    ) -> Result<serde_json::Value, platform_query::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

#[cfg(test)]
impl github::Client for NullClient {
    async fn raw_file_content(
        &self,
        _org: &str,
        _repo: &str,
        _path: &str,
        _git_ref: Option<impl AsRef<str>>,
    ) -> Result<bytes::Bytes, github::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

#[cfg(test)]
impl rtf_core::HttpClient for NullClient {
    async fn get(&self, _url: &str) -> Result<rtf_core::HttpResponse, reqwest::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

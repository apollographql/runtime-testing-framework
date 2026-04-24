use crate::{
    context::{PathKind, ResolutionContext},
    formats::Sources,
    providers::{
        self,
        file::{SourceDir, StableSource},
    },
    run::Provider,
    templating::CustomProviderDefinitions,
};
use bytes::Bytes;
use reqwest::StatusCode;
use rtf_integrations::{
    HttpClient, HttpResponse, github, graphos::platform_query,
    graphos::supergraph::SupergraphDetails,
};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

pub(crate) struct MockContext<C: HttpClient + Clone> {
    http: C,
    github: Option<MockGithubClient>,
    output_path: PathBuf,
    source: Option<SourceDir>,
}

impl<C: HttpClient + Clone> MockContext<C> {
    /// Set the source dir used for source-relative path operations (used in tests that need IO from a specific source).
    pub(crate) fn with_source(mut self, source: SourceDir) -> Self {
        self.source = Some(source);
        self
    }
}

impl MockContext<MockHttpClient> {
    pub(crate) fn with_http_client(responses: &[(&str, &str, &str)]) -> Self {
        MockContext {
            http: MockHttpClient::with_responses(responses),
            github: None,
            output_path: PathBuf::new(),
            source: None,
        }
    }
}

impl MockContext<NullClient> {
    pub(crate) fn with_github_client(responses: &[(&str, &str)]) -> Self {
        MockContext {
            http: NullClient,
            github: Some(MockGithubClient {
                responses: responses
                    .iter()
                    .map(|(url, content)| (url.to_string(), content.to_string()))
                    .collect(),
            }),
            output_path: PathBuf::new(),
            source: None,
        }
    }
}

impl<C: HttpClient + Clone + Send + Sync + 'static> ResolutionContext for MockContext<C> {
    type HttpClient = C;
    type GithubClient = MockGithubClient;
    type PlatformClient = NullClient;

    fn http_client(&self) -> &Self::HttpClient {
        &self.http
    }

    fn github_client(&self) -> Option<&MockGithubClient> {
        self.github.as_ref()
    }

    fn set_output_path(&mut self, path: impl Into<PathBuf>) {
        self.output_path = path.into();
    }

    fn output_path(&self) -> &Path {
        &self.output_path
    }

    fn canonicalize_path(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        path.as_ref().canonicalize()
    }

    fn path_kind(&self, path: impl AsRef<Path>) -> PathKind {
        let p = path.as_ref();
        if !p.exists() {
            PathKind::Missing
        } else if p.is_file() {
            PathKind::File
        } else {
            match p.read_dir() {
                Ok(mut rd) => {
                    if rd.next().is_some() {
                        PathKind::OccupiedDir
                    } else {
                        PathKind::EmptyDir
                    }
                }
                Err(_) => PathKind::Missing,
            }
        }
    }

    fn source_dir_for(&self, _src: &StableSource) -> &SourceDir {
        self.source
            .as_ref()
            .expect("source not configured in MockContext - call .with_source()")
    }

    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        fs::read_to_string(path)
    }

    async fn read_file_content(
        &self,
        _src: &StableSource,
        relative_path: &str,
    ) -> providers::Result<String> {
        let source = self
            .source
            .as_ref()
            .expect("MockContext::read_file_content requires a source set via with_source");
        source.try_get_file_content(relative_path, self).await
    }

    fn store_provider_output_path(&mut self, _provider: Provider<'_>, _path: PathBuf) {}

    fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        fs::write(path, contents)
    }

    fn run_command_blocking<'a>(
        &self,
        _prog: &str,
        _args: impl IntoIterator<Item = &'a str>,
        _env_vars: &HashMap<String, String>,
    ) -> io::Result<()> {
        unimplemented!(
            "If you are hitting this we have not needed to mock this yet which is why it is not implemented"
        )
    }

    fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn remove_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::remove_dir_all(path)
    }

    fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    async fn with_supergraph_details<T: Send>(
        &self,
        _env_name: impl Into<String> + Send,
        _graph_id: impl Into<String> + Send,
        _variant: impl Into<String> + Send,
        _f: impl FnOnce(&Arc<SupergraphDetails>) -> providers::Result<T> + Send,
    ) -> providers::Result<T> {
        panic!(
            "This should not be called in tests, we test the methods that transform the response from this method instead"
        )
    }

    fn set_sources(&mut self, _sources: Sources) {
        unimplemented!(
            "If you are hitting this we have not needed to mock this yet which is why it is not implemented"
        )
    }

    fn custom_provider_definitions(&self) -> Arc<CustomProviderDefinitions> {
        unimplemented!(
            "If you are hitting this we have not needed to mock this yet which is why it is not implemented"
        )
    }
}

#[derive(Clone)]
pub(crate) struct MockHttpClient {
    responses: HashMap<String, HttpResponse>,
}

impl MockHttpClient {
    fn with_responses(responses: &[(&str, &str, &str)]) -> Self {
        let responses: HashMap<String, HttpResponse> = responses
            .iter()
            .map(|(url, status_code, body)| {
                (
                    url.to_string(),
                    HttpResponse {
                        status: StatusCode::from_str(status_code)
                            .expect("invalid HTTP status code"),
                        body: body.to_string().into(),
                    },
                )
            })
            .collect();
        MockHttpClient { responses }
    }
}

impl HttpClient for MockHttpClient {
    async fn get(&self, url: &str) -> Result<HttpResponse, reqwest::Error> {
        match self.responses.get(url) {
            Some(response) => Ok(response.clone()),
            None => panic!("unexpected URL: {url}"),
        }
    }
}

pub(crate) struct MockGithubClient {
    responses: HashMap<String, String>,
}

impl github::Client for MockGithubClient {
    async fn raw_file_content<G: AsRef<str> + Send>(
        &self,
        _org: &str,
        _repo: &str,
        _path: &str,
        _git_ref: Option<G>,
    ) -> Result<Bytes, github::Error> {
        unimplemented!(
            "If you are hitting this we have not needed to mock this yet which is why it is not implemented"
        )
    }

    async fn string_file_content<G: AsRef<str> + Send>(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<G>,
    ) -> Result<String, github::Error> {
        let git_ref = match git_ref {
            Some(s) => format!("?ref={}", s.as_ref()),
            None => String::new(),
        };

        let key = format!("{org}/{repo}/{path}{git_ref}");

        Ok(self
            .responses
            .get(&key)
            .unwrap_or_else(|| panic!("unknown key: {key}"))
            .to_owned())
    }
}

/// Used to implement [ResolutionContext] in tests where no client is needed.

#[derive(Debug, Clone, Copy)]
pub(crate) struct NullClient;

impl HttpClient for NullClient {
    async fn get(&self, _url: &str) -> Result<rtf_integrations::HttpResponse, reqwest::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

impl platform_query::Client for NullClient {
    async fn post_operation(
        &self,
        _body: &(impl serde::Serialize + Sync),
    ) -> Result<serde_json::Value, platform_query::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

impl github::Client for NullClient {
    async fn raw_file_content<G: AsRef<str> + Send>(
        &self,
        _org: &str,
        _repo: &str,
        _path: &str,
        _git_ref: Option<G>,
    ) -> Result<bytes::Bytes, github::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

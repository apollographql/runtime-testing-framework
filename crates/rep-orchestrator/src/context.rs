use rtf_config::{
    SourceDir, StableSource,
    context::{Context, PathKind, ResolutionContext},
    formats::{CustomProviderDefinition, Sources},
    providers,
    run::Provider,
    templating::CustomProviderDefinitions,
};
use rtf_integrations::{
    ReqwestClient, github,
    graphos::{self, supergraph::SupergraphDetails},
};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::rep_test_plan::{SourceKeyedArrayMap, SourceKeyedMap};

/// A [ResolutionContext] that reads relative files from a map rather than the filesystem.
#[derive(Debug, Clone)]
pub struct RepContext {
    inner: Context,
    relative_files: SourceKeyedMap<String>,
    custom_providers: Arc<CustomProviderDefinitions>,
}

impl RepContext {
    pub fn try_new(
        inner: Context,
        relative_files: SourceKeyedArrayMap<String>,
        custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            inner,
            relative_files: relative_files.into_map(),
            custom_providers: Arc::new(custom_providers.try_into_custom_provider_definitions()?),
        })
    }

    pub fn try_new_from_env_vars(
        env_vars: HashMap<String, String>,
        relative_files: SourceKeyedArrayMap<String>,
        custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
    ) -> anyhow::Result<Self> {
        Self::try_new(
            Context::new_from_env_vars(env_vars),
            relative_files,
            custom_providers,
        )
    }
}

#[allow(async_fn_in_trait)]
impl ResolutionContext for RepContext {
    type PlatformClient = graphos::PlatformClient;
    type GithubClient = github::GithubClient;
    type HttpClient = ReqwestClient;

    fn platform_client(&self) -> Option<&Self::PlatformClient> {
        self.inner.platform_client()
    }

    fn github_client(&self) -> Option<&Self::GithubClient> {
        self.inner.github_client()
    }

    fn http_client(&self) -> &Self::HttpClient {
        self.inner.http_client()
    }

    fn set_output_path(&mut self, path: impl Into<PathBuf>) {
        self.inner.set_output_path(path);
    }

    fn output_path(&self) -> &Path {
        self.inner.output_path()
    }

    fn store_provider_output_path(&mut self, provider: Provider<'_>, path: PathBuf) {
        self.inner.store_provider_output_path(provider, path);
    }

    fn known_provider_output_path(&self, provider: Provider<'_>) -> Option<PathBuf> {
        self.inner.known_provider_output_path(provider)
    }

    async fn with_supergraph_details<T>(
        &self,
        graph_id: impl Into<String>,
        variant: impl Into<String>,
        f: impl FnOnce(&Arc<SupergraphDetails>) -> providers::Result<T>,
    ) -> providers::Result<T> {
        self.inner
            .with_supergraph_details(graph_id, variant, f)
            .await
    }

    // Reading files looks up in the map we were constructed with rather than touching the
    // filesystem

    async fn read_file_content(
        &self,
        src: &StableSource,
        relative_path: &str,
    ) -> providers::Result<String> {
        self.relative_files
            .get(src.clone(), relative_path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, relative_path).into())
    }

    fn custom_provider_definitions(&self) -> Arc<CustomProviderDefinitions> {
        self.custom_providers.clone()
    }

    // All other file system related methods panic as we can't / shouldn't run them in a server
    // context

    fn read_path_to_string(&self, _path: impl AsRef<Path>) -> io::Result<String> {
        panic!("attempt to read path to string")
    }

    fn set_sources(&mut self, _sources: Sources) {
        panic!("attempt to set sources")
    }

    fn source_dir_for(&self, _src: &StableSource) -> &SourceDir {
        panic!("attempt to get source dir for stable source")
    }

    fn canonicalize_path(&self, _relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        panic!("attempt to canonicalize path")
    }

    fn path_kind(&self, _path: impl AsRef<Path>) -> PathKind {
        panic!("attempt to check path kind")
    }

    fn write(&self, _path: impl AsRef<Path>, _contents: impl AsRef<[u8]>) -> io::Result<()> {
        panic!("attempt to write file content")
    }

    fn run_command_blocking<'a>(
        &self,
        _prog: &str,
        _args: impl IntoIterator<Item = &'a str>,
        _env_vars: &HashMap<String, String>,
    ) -> io::Result<()> {
        panic!("attempt to run command blocking");
    }

    fn store_run_metadata(&mut self, key: &'static str, value: impl Into<String>) {
        self.inner.store_run_metadata(key, value);
    }

    fn run_metadata(&self, key: &str) -> Option<&str> {
        self.inner.run_metadata(key)
    }

    fn remove_file(&self, _path: impl AsRef<Path>) -> io::Result<()> {
        panic!("attempt to remove file")
    }

    fn remove_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
        panic!("attempt to remove dir")
    }

    fn create_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
        panic!("attempt to create dir")
    }
}

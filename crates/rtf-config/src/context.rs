use crate::{
    formats::Sources,
    providers::{
        self,
        file::{SourceDir, StableSource},
    },
    run::Provider,
    templating::CustomProviderDefinitions,
};
use rtf_integrations::{
    APOLLO_KEY_ENV_VAR, APOLLO_SUDO_ENV_VAR, GITHUB_TOKEN_ENV_VAR, GRAPH_OS_STAGING_ENV_VAR,
    HttpClient, ReqwestClient, github,
    graphos::{platform_query, supergraph::SupergraphDetails},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    fs::{self, File},
    io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, RwLock},
};
use tokio::sync::Mutex;
use tracing::error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    File,
    EmptyDir,
    OccupiedDir,
}

/// Types that implement ResolutionContext may be used to perform IO while checking and templating
/// providers.
///
/// This trait is used to allow us to inject mock IO implementations in tests so we do not need to
/// make use of temp directories or mock servers. For the most part the methods provided by this
/// trait are wrappers around [std::fs] and API clients for making requests to third party APIs.
///
/// For the canonical real implementations that should be mocked see [Context].
#[allow(async_fn_in_trait)]
pub trait ResolutionContext {
    type PlatformClient: platform_query::Client;
    type GithubClient: github::Client;
    type HttpClient: HttpClient;

    /// Provide a [Client][platform_query::Client] for making requests to the Apollo platform API.
    ///
    /// If it is not possible for this current context to make requests to the platform API then
    /// this method should return [None].
    fn platform_client(&self) -> Option<&Self::PlatformClient> {
        None
    }

    /// Provide a [Client][github::Client] for making requests to the GitHub REST API.
    ///
    /// If it is not possible for this current context to make requests to the GitHub API then
    /// this method should return [None].
    fn github_client(&self) -> Option<&Self::GithubClient> {
        None
    }

    fn http_client(&self) -> &Self::HttpClient;

    fn set_output_path(&mut self, path: impl Into<PathBuf>);

    /// The top level output path being used.
    fn output_path(&self) -> &Path;

    /// Record the path that the given provider's output was written to.
    fn store_provider_output_path(&mut self, provider: Provider<'_>, path: PathBuf);

    /// Query the output path of a given provider.
    #[allow(unused_variables)]
    fn known_provider_output_path(&self, provider: Provider<'_>) -> Option<PathBuf> {
        None
    }

    /// Make use of potentially cached supergraph details to derive related data.
    /// The default implementation of this method will simply pull the required [SupergraphDetails]
    /// and pass them to the mapping function provided. Custom implementations can be used to
    /// implement caching and other custom behaviour.
    ///
    /// # Panics
    /// The default implementation will panic if [ResolutionContext::platform_client] is [None].
    async fn with_supergraph_details<T>(
        &self,
        graph_id: impl Into<String>,
        variant: impl Into<String>,
        f: impl FnOnce(&Arc<SupergraphDetails>) -> providers::Result<T>,
    ) -> providers::Result<T> {
        let client = self.platform_client().expect("no platform client");
        let details = SupergraphDetails::fetch(graph_id, variant, client).await?;

        f(&Arc::new(details))
    }

    /// Returns the canonical, absolute form of the path with all intermediate
    /// components normalized and symbolic links resolved.
    fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf>;

    fn dir_containing(&self, path: impl AsRef<Path>) -> PathBuf {
        match path.as_ref().parent() {
            Some(p) => p.to_path_buf(),
            None => PathBuf::new(),
        }
    }

    /// Categorise the provided path.
    ///
    /// If you cannot access the metadata of the file, e.g. because of a permission error or broken
    /// symbolic links, this will return [PathKind::Missing].
    fn path_kind(&self, path: impl AsRef<Path>) -> PathKind;

    /// Reads the entire contents of a file into a string.
    ///
    /// # Errors
    ///
    /// This function will return an error if `path` does not already exist. Other errors may also
    /// be returned according to [OpenOptions::open][0].
    ///
    /// If the contents of the file are not valid UTF-8, then an error will also be returned.
    ///
    /// While reading from the file, this function handles [io::ErrorKind::Interrupted] with
    /// automatic retries. See [io::Read] documentation for details.
    ///
    /// [0]: std::fs::OpenOptions::open
    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String>;

    /// Writes a slice as the entire contents of a file.
    ///
    /// This function will create a file if it does not exist, and will entirely replace its
    /// contents if it does.
    ///
    /// Depending on the platform, this function may fail if the full directory path does not
    /// exist.
    fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> io::Result<()>;

    /// Spawn the specified program as a subprocess with the provided arguments.
    /// This method will block until the process completes and return the stdout of the process as
    /// a utf-8 string.
    fn run_command_blocking<'a>(
        &self,
        prog: &str,
        args: impl IntoIterator<Item = &'a str>,
        env_vars: &HashMap<String, String>,
    ) -> io::Result<()>;

    /// Changes the permissions of the specified file
    fn make_executable(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let file = File::open(path.as_ref())?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o777);

        file.set_permissions(permissions)
    }

    /// Store a key-value pair of run metadata for cross-phase communication.
    fn store_run_metadata(&mut self, _key: &'static str, _value: impl Into<String>) {}

    /// Retrieve a previously stored run metadata value by key.
    fn run_metadata(&self, _key: &str) -> Option<&str> {
        None
    }

    fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()>;

    fn remove_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()>;

    /// Recursively create a directory and all of its parent components if they
    /// are missing.
    ///
    /// If this function returns an error, some of the parent components might have
    /// been created already.
    ///
    /// If the empty path is passed to this function, it always succeeds without
    /// creating any directories.
    fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()>;

    /// Store the resolved [Sources] for the current test plan.
    ///
    /// Callers of `try_load_and_resolve_from_*` are responsible for calling this
    /// with `test_plan.sources.clone()` before passing `ctx` to an execution phase.
    fn set_sources(&mut self, _sources: Sources) {
        unimplemented!("set_sources not implemented for this context")
    }

    /// Set the [SourceDir] for variables coming from the CLI (`--var k=v`).
    fn set_cli_source(&mut self, _source: SourceDir) {
        unimplemented!("set_cli_source not implemented for this context")
    }

    /// Set the [SourceDir] for variables coming from a `--vars` file.
    fn set_variables_file_source(&mut self, _source: SourceDir) {
        unimplemented!("set_variables_file_source not implemented for this context")
    }

    /// Resolve a [SourceDir] from a [StableSource] logical name.
    fn source_dir_for(&self, _src: &StableSource) -> &SourceDir {
        unimplemented!("source_dir_for not implemented for this context")
    }

    /// Return the [CustomProviderDefinitions] for the current test plan.
    fn custom_provider_definitions(&self) -> Arc<CustomProviderDefinitions> {
        unimplemented!("custom_provider_definitions not implemented for this context")
    }

    /// Read the contents of a file relative to the given [StableSource].
    async fn read_file_content(
        &self,
        src: &StableSource,
        relative_path: impl AsRef<Path>,
    ) -> providers::Result<String>
    where
        Self: Sized,
    {
        self.source_dir_for(src)
            .try_get_file_content(relative_path, self)
            .await
    }
}

/// A [ResolutionContext] that will perform real IO.
#[derive(Default, Debug)]
pub struct Context {
    client: ReqwestClient,
    supergraph_details: Mutex<HashMap<String, Arc<SupergraphDetails>>>,
    fp_output_paths: HashMap<String, PathBuf>,
    output_path: PathBuf,
    capture_output: bool,
    captured_stdout: RwLock<Vec<u8>>,
    captured_stderr: RwLock<Vec<u8>>,
    run_metadata: HashMap<&'static str, String>,
    sources: Sources,
    cli_source: SourceDir,
    variables_file_source: Option<SourceDir>,
}

impl Context {
    /// Construct a new `Context` with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a new `Context` with environment variables.
    pub fn new_from_env_vars(mut env_vars: HashMap<String, String>) -> Self {
        let mut ctx = Self::new();

        if let Some(api_key) = env_vars.remove(APOLLO_KEY_ENV_VAR) {
            let staging = matches!(
                env_vars.remove(GRAPH_OS_STAGING_ENV_VAR).as_deref(),
                Some("true" | "1")
            );
            let sudo = matches!(
                env_vars.remove(APOLLO_SUDO_ENV_VAR).as_deref(),
                Some("true" | "1")
            );

            ctx.with_platform_config(api_key, staging, sudo);
        }

        if let Some(api_token) = env_vars.remove(GITHUB_TOKEN_ENV_VAR) {
            ctx.with_github_config(api_token);
        }

        ctx
    }

    /// Provide configuration for making requests to the Apollo platform API.
    pub fn with_platform_config(
        &mut self,
        api_key: impl Into<String>,
        staging: bool,
        sudo: bool,
    ) -> &mut Self {
        self.client.with_platform_config(api_key, staging, sudo);
        self
    }

    /// Provide configuration for making requests to the GitHub REST API.
    pub fn with_github_config(&mut self, api_token: impl Into<String>) -> &mut Self {
        self.client.with_github_config(api_token);
        self
    }

    /// Enable output capture mode. When enabled, `run_command_blocking` will
    /// suppress stdout/stderr and store the output in internal buffers.
    pub fn enable_output_capture(&mut self) {
        self.capture_output = true;
    }

    /// Clear the captured stdout and stderr buffers.
    pub fn clear_captured_output(&self) {
        self.captured_stdout.write().unwrap().clear();
        self.captured_stderr.write().unwrap().clear();
    }

    /// Returns captured stdout as a string, using lossy UTF-8 conversion.
    pub fn captured_stdout(&self) -> String {
        let guard = self.captured_stdout.read().unwrap();
        String::from_utf8_lossy(&guard).into_owned()
    }

    /// Returns captured stderr as a string, using lossy UTF-8 conversion.
    pub fn captured_stderr(&self) -> String {
        let guard = self.captured_stderr.read().unwrap();
        String::from_utf8_lossy(&guard).into_owned()
    }
}

impl ResolutionContext for Context {
    type PlatformClient = rtf_integrations::graphos::PlatformClient;
    type GithubClient = github::GithubClient;
    type HttpClient = ReqwestClient;

    fn platform_client(&self) -> Option<&Self::PlatformClient> {
        self.client.platform_client()
    }

    fn github_client(&self) -> Option<&Self::GithubClient> {
        self.client.github_client()
    }

    fn http_client(&self) -> &Self::HttpClient {
        &self.client
    }

    fn set_output_path(&mut self, path: impl Into<PathBuf>) {
        self.output_path = path.into();
    }

    fn output_path(&self) -> &Path {
        &self.output_path
    }

    fn store_provider_output_path(&mut self, provider: Provider<'_>, path: PathBuf) {
        let key = match serde_yaml::to_string(&provider) {
            Ok(s) => s,
            Err(error) => {
                error!(?path, %error, "unable to generate file provider cache key");
                return;
            }
        };

        self.fp_output_paths.insert(key, path);
    }

    fn known_provider_output_path(&self, provider: Provider<'_>) -> Option<PathBuf> {
        let key = serde_yaml::to_string(&provider).ok()?;

        self.fp_output_paths.get(&key).cloned()
    }

    async fn with_supergraph_details<T>(
        &self,
        graph_id: impl Into<String>,
        variant: impl Into<String>,
        f: impl FnOnce(&Arc<SupergraphDetails>) -> providers::Result<T>,
    ) -> providers::Result<T> {
        let graph_id = graph_id.into();
        let variant = variant.into();

        let mut guard = self.supergraph_details.lock().await;
        let client = self.client.platform_client().expect("no platform client");
        let graph_ref = format!("{graph_id}@{variant}");

        if let Entry::Vacant(e) = guard.entry(graph_ref.clone()) {
            let details = Arc::new(SupergraphDetails::fetch(graph_id, variant, client).await?);
            let res = f(&details);
            e.insert(details);

            return res;
        }

        let details = guard.get(&graph_ref).expect("details should be cached");

        f(details)
    }

    fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        relative_path.as_ref().canonicalize()
    }

    fn path_kind(&self, path: impl AsRef<Path>) -> PathKind {
        let p = path.as_ref();

        if !p.exists() {
            return PathKind::Missing;
        } else if p.is_file() {
            return PathKind::File;
        }

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

    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        fs::write(path, contents)
    }

    fn run_command_blocking<'a>(
        &self,
        prog: &str,
        args: impl IntoIterator<Item = &'a str>,
        env_vars: &HashMap<String, String>,
    ) -> io::Result<()> {
        if self.capture_output {
            let output = Command::new(prog).args(args).envs(env_vars).output()?;

            self.captured_stdout.write().unwrap().extend(&output.stdout);
            self.captured_stderr.write().unwrap().extend(&output.stderr);

            if !output.status.success() {
                return Err(io::Error::other(format!(
                    "{prog:?} failed to terminate successfully"
                )));
            }
        } else {
            let status = Command::new(prog)
                .args(args)
                .envs(env_vars)
                .spawn()?
                .wait()?;

            if !status.success() {
                return Err(io::Error::other(format!(
                    "{prog:?} failed to terminate successfully"
                )));
            }
        }

        Ok(())
    }

    fn store_run_metadata(&mut self, key: &'static str, value: impl Into<String>) {
        self.run_metadata.insert(key, value.into());
    }

    fn run_metadata(&self, key: &str) -> Option<&str> {
        self.run_metadata.get(key).map(|s| s.as_str())
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

    fn set_sources(&mut self, sources: Sources) {
        self.sources = sources;
    }

    fn source_dir_for(&self, src: &StableSource) -> &SourceDir {
        match src {
            StableSource::TestPlan => self.sources.test_plan(),
            StableSource::Environment => self.sources.environment(),
            StableSource::Scenario => self.sources.scenario(),
            StableSource::CustomProvider(n) => self
                .sources
                .custom_provider_source(n)
                .expect("custom provider source not found"),
            StableSource::Cli => &self.cli_source,
            StableSource::VariablesFile => self
                .variables_file_source
                .as_ref()
                .expect("VariablesFile source not set"),
        }
    }

    fn custom_provider_definitions(&self) -> Arc<CustomProviderDefinitions> {
        self.sources.custom_providers()
    }

    fn set_cli_source(&mut self, source: SourceDir) {
        self.cli_source = source;
    }

    fn set_variables_file_source(&mut self, source: SourceDir) {
        self.variables_file_source = Some(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        StableSource,
        providers::file::{FileProvider, RelativeFile},
        templating::Field,
    };
    use simple_test_case::test_case;

    #[test_case(None, None, true; "no sources")]
    #[test_case(Some(StableSource::TestPlan), Some(StableSource::TestPlan), true; "same source")]
    #[test_case(Some(StableSource::TestPlan), None, false; "cache with source new without")]
    #[test_case(None, Some(StableSource::TestPlan), false; "cache without source new with")]
    #[test_case(Some(StableSource::TestPlan), Some(StableSource::Environment), false; "different sources")]
    #[test]
    fn context_provider_caching_respects_relative_path_sources(
        source_1: Option<StableSource>,
        source_2: Option<StableSource>,
        path_is_known: bool,
    ) {
        let provider_1 = FileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("scripts/run.sh".to_string()),
            src: source_1,
        });

        let provider_2 = FileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("scripts/run.sh".to_string()),
            src: source_2,
        });

        let mut ctx = Context::new();

        ctx.store_provider_output_path(
            Provider::File { fp: &provider_1 },
            PathBuf::from("/some/path"),
        );
        let maybe_path = ctx.known_provider_output_path(Provider::File { fp: &provider_2 });

        assert_eq!(maybe_path.is_some(), path_is_known);
    }
}

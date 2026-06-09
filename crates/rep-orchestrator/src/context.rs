use crate::{
    config::Config,
    resolver::{self, ResolverError},
};
use rep_orchestrator_shared::{payload::SourceKeyedArrayMap, test_plan::RepTestPlan};
use rtf_config::{
    SourceDir, StableSource,
    checks::{self, Check},
    context::{Context, PathKind, ResolutionContext},
    formats::{CustomProviderDefinition, FileProviderServices, Sources},
    inlining::InlinedProvider,
    providers,
    run::Provider,
    templating::{CustomProviderDefinitions, Template, TemplateContext},
};
use rtf_integrations::{
    ReqwestClient, github,
    graphos::{self, supergraph::SupergraphDetails},
};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::take,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

/// A [ResolutionContext] that reads relative files from a map rather than the filesystem.
#[derive(Debug, Clone)]
pub struct RepContext {
    inner: Context,
    relative_files: SourceKeyedArrayMap<String>,
    custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
    inline_cache: Arc<Mutex<HashMap<u64, InlinedProvider>>>,
}

impl RepContext {
    pub fn new(
        cfg: &Config,
        relative_files: SourceKeyedArrayMap<String>,
        custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
    ) -> Self {
        let mut inner = Context::new();
        // apollo_sudo always true; graphos_staging always false for REP
        inner.with_platform_config(&cfg.apollo_key, false, true);
        inner.with_github_app_config(cfg.github_app_id, cfg.github_app_private_key_pem.clone());

        Self {
            inner,
            relative_files,
            custom_providers,
            inline_cache: Default::default(),
        }
    }

    pub fn inline_cache(&self) -> Arc<Mutex<HashMap<u64, InlinedProvider>>> {
        self.inline_cache.clone()
    }

    /// Eagerly resolve and check all docker-compose providers within the given test plan to see if
    /// they contain any user defined explicit volume mounts: if they do, this test plan will fail
    /// to produce a working environment and we'll end up with a dead namespace per invalid
    /// execution.
    ///
    /// This is used as part of the trigger endpoint to prevent such test plans entering the main
    /// event loop.
    pub async fn validate_environment_file_provider_usage(
        &self,
        test_plan: &RepTestPlan,
    ) -> crate::Result<()> {
        let mut inline_cache = HashMap::new();
        let mut invalid = HashSet::new();

        let variants = test_plan
            .try_iter_matrix_variants()
            .map_err(resolver::ResolverError::MatrixExpansion)?;

        for (_, variant) in variants {
            let fps = self
                .inline_and_check_variant(variant, &mut inline_cache)
                .await?;

            if !fps.explicit_mount.is_empty() {
                invalid.extend(fps.explicit_mount);
            }
        }

        if invalid.is_empty() {
            Ok(())
        } else {
            let mut services: Vec<String> = invalid.into_iter().collect();
            services.sort_unstable();

            Err(crate::Error::InvalidFileProviderUsage { services })
        }
    }

    async fn inline_and_check_variant(
        &self,
        mut variant: RepTestPlan,
        inline_cache: &mut HashMap<u64, InlinedProvider>,
    ) -> resolver::Result<FileProviderServices> {
        let variables = take(&mut variant.variables);
        let template_ctx = TemplateContext::new(
            variables,
            HashMap::new(),
            self.custom_provider_definitions(),
        );

        variant
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(ResolverError::VariantTemplating)?;
        variant.try_check(&mut Vec::new(), self)?;

        variant
            .environment
            .execution
            .inline_compose_files(self, inline_cache)
            .await?;

        Ok(FileProviderServices::from_inline(
            &variant.environment.execution,
        ))
    }
}

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

    async fn with_supergraph_details<T: Send>(
        &self,
        graph_id: impl Into<String> + Send,
        variant: impl Into<String> + Send,
        f: impl FnOnce(&Arc<SupergraphDetails>) -> providers::Result<T> + Send,
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
        let mut defs = CustomProviderDefinitions::default();
        for sk in &self.custom_providers.keys {
            let def = self.custom_providers.data[sk.index].clone();
            match &sk.src {
                StableSource::TestPlan => {
                    defs.test_plan.insert(sk.k.clone(), def);
                }
                StableSource::Scenario => {
                    defs.scenario.insert(sk.k.clone(), def);
                }
                StableSource::Environment => {
                    defs.environment.insert(sk.k.clone(), def);
                }
                _ => {}
            }
        }
        Arc::new(defs)
    }

    fn check_path_kind(
        &self,
        stable_src: &StableSource,
        str_path: &str,
        err_path: &[String],
    ) -> checks::Result<Option<PathKind>> {
        match self.relative_files.get(stable_src.clone(), str_path) {
            Some(_) => Ok(Some(PathKind::File)),
            None => {
                if self.relative_files.has_path_prefix(stable_src, str_path) {
                    Ok(Some(PathKind::OccupiedDir))
                } else {
                    Err(checks::Errors::new(
                        checks::ErrorKind::FileNotFound,
                        format!("unknown file path: {stable_src:?} {str_path:?}"),
                        err_path,
                    ))
                }
            }
        }
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

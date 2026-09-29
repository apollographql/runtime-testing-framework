use crate::{
    config::Config,
    resolver::{self, ResolverError},
};
use rtf_config::{
    SourceDir, StableSource,
    checks::{self, Check},
    context::{Context, PathKind, ResolutionContext},
    formats::{CustomProviderDefinition, FileProviderServices, Sources},
    inlining::InlinedProvider,
    providers,
    run::Provider,
    templating::{CustomProviderDefinitions, Scalar, Template, TemplateContext},
};
use rtf_integrations::{
    ReqwestClient, github,
    graphos::{self, supergraph::SupergraphDetails},
};
use rtf_orchestrator_shared::{payload::SourceKeyedArrayMap, test_plan::OrchestratorTestPlan};
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
pub struct OrchestratorContext {
    inner: Context,
    relative_files: SourceKeyedArrayMap<String>,
    custom_providers: Arc<CustomProviderDefinitions>,
    variable_sources: HashMap<String, StableSource>,
    inline_cache: Arc<Mutex<HashMap<u64, InlinedProvider>>>,
}

impl OrchestratorContext {
    pub fn new_from_inlined_files(
        cfg: &Config,
        relative_files: SourceKeyedArrayMap<String>,
        custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
        variable_sources: HashMap<String, StableSource>,
    ) -> Self {
        let mut ctx = Self::new(cfg);
        ctx.relative_files = relative_files;
        ctx.custom_providers = Arc::new(custom_providers.into_custom_provider_definitions());
        ctx.variable_sources = variable_sources;

        ctx
    }

    pub fn new(cfg: &Config) -> Self {
        Self {
            inner: cfg.server_context(),
            relative_files: SourceKeyedArrayMap::empty(),
            custom_providers: Arc::new(CustomProviderDefinitions::default()),
            variable_sources: HashMap::new(),
            inline_cache: Default::default(),
        }
    }

    pub fn inline_cache(&self) -> Arc<Mutex<HashMap<u64, InlinedProvider>>> {
        self.inline_cache.clone()
    }

    pub fn variable_sources(&self) -> HashMap<String, StableSource> {
        self.variable_sources.clone()
    }

    pub fn new_template_context(&self, variables: HashMap<String, Scalar>) -> TemplateContext {
        TemplateContext::new(
            variables,
            self.variable_sources(),
            self.custom_provider_definitions(),
        )
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
        test_plan: &OrchestratorTestPlan,
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
        mut variant: OrchestratorTestPlan,
        inline_cache: &mut HashMap<u64, InlinedProvider>,
    ) -> resolver::Result<FileProviderServices> {
        let variables = take(&mut variant.variables);
        let template_ctx = self.new_template_context(variables);

        variant
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(ResolverError::VariantTemplating)?;
        variant.try_check(&mut Vec::new(), self)?;

        variant
            .environment
            .execution
            .inline_manifest_files(self, inline_cache)
            .await?;

        Ok(variant
            .environment
            .execution
            .file_provider_services_from_inline())
    }
}

impl ResolutionContext for OrchestratorContext {
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
    // filesystem. In the case of local files that were auto-promoted into GitHub files we
    // defer to the inner Context to handle the read so we are able to support RelativeDir.

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

    fn set_sources(&mut self, sources: Sources) {
        self.inner.set_sources(sources);
    }

    fn source_dir_for(&self, src: &StableSource) -> &SourceDir {
        self.inner.source_dir_for(src)
    }

    // All other file system related methods panic as we can't / shouldn't run them in a server
    // context

    fn read_path_to_string(&self, _path: impl AsRef<Path>) -> io::Result<String> {
        panic!("attempt to read path to string")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn new_from_inlined_files_stores_variable_sources() {
        let cfg = Config::for_test();
        let variable_sources =
            HashMap::from([("setup_subject".to_string(), StableSource::VariablesFile)]);

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            SourceKeyedArrayMap::empty(),
            SourceKeyedArrayMap::empty(),
            variable_sources.clone(),
        );

        assert_eq!(ctx.variable_sources(), variable_sources);
    }

    #[test]
    fn new_has_no_variable_sources() {
        let ctx = OrchestratorContext::new(&Config::for_test());

        assert!(ctx.variable_sources().is_empty());
    }

    #[test]
    fn template_context_resolves_variables_against_their_stored_source() {
        let cfg = Config::for_test();
        let variable_sources =
            HashMap::from([("setup_subject".to_string(), StableSource::VariablesFile)]);
        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            SourceKeyedArrayMap::empty(),
            SourceKeyedArrayMap::empty(),
            variable_sources,
        );

        let variables = HashMap::from([
            ("setup_subject".to_string(), Scalar::from("fish")),
            ("untracked_variable".to_string(), Scalar::from("chips")),
        ]);
        let template_ctx = ctx.new_template_context(variables);

        assert_eq!(
            template_ctx.get_with_source("setup_subject"),
            Some((&StableSource::VariablesFile, &Scalar::from("fish")))
        );
        assert_eq!(
            template_ctx.get_with_source("untracked_variable"),
            Some((&StableSource::TestPlan, &Scalar::from("chips")))
        );
    }
}

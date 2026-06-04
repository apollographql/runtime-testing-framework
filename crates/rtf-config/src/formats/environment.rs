//! Parsing of the environment provisioner config file format
use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray, duplicate_keys},
    context::{PathKind, ResolutionContext},
    enum_impl_check,
    formats::{CustomProviderDeclaration, Result},
    inlining::{self, InlineMode, InlinedProvider},
    providers::{
        self,
        command::CommandSection,
        file::{NamedFileProvider, StableSource, compose::NamedComposeFileProvider},
    },
    run::{
        DOCKER_COMPOSE_NETWORK, Execute, OUTDIR, OUTPUT_PATH, PROVIDER_DIR, Provider,
        RunEnvironment, RunProviders,
    },
    templating::{self, Field, FileType, Scalar, Template, TemplateContext},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, read_dir},
    path::{Path, PathBuf},
    pin::Pin,
};

const FILE_PROVIDERS_LABEL: &str = "rtf.io/file-providers";
const PROVIDERS_CONTAINER_PATH: &str = "/providers";

/// # Environment Config
///
/// Configuration for preparing and cleaning up the test environment as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct EnvironmentConfig<R: RunEnvironment> {
    /// The name of this environment configuration
    pub name: String,
    /// A brief description of how this environment setup works
    pub description: String,
    /// Definitions for the required variables for templating this environment
    #[serde(default, alias = "values")]
    // This alias is for backwards compatibility with the original name
    pub variable_definitions: Vec<VariableDefinition>,
    /// Custom provider declarations to load for this environment
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderDeclaration>,
    #[serde(flatten)]
    pub execution: R,
}

impl EnvironmentConfig<EnvironmentExecution> {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    /// Create an empty [EnvironmentConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> EnvironmentConfig<EnvironmentExecution> {
        EnvironmentConfig {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Vec::new(),
            custom_providers: Default::default(),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection::empty(),
                teardown: CommandSection::empty(),
            }),
        }
    }
}

impl<R: RunEnvironment> EnvironmentConfig<R> {
    pub async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.execution.execute_setup(name, out_dir, ctx).await
    }

    pub async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.execution.execute_teardown(name, out_dir, ctx).await
    }
}

impl<R: RunEnvironment> EnvironmentConfig<R> {
    pub async fn inline(
        &mut self,
        mode: &InlineMode,
        ctx: &impl ResolutionContext,
        cache: &mut HashMap<u64, InlinedProvider>,
    ) -> inlining::Result<()> {
        self.execution.inline(mode, ctx, cache).await
    }
}

impl<R: RunEnvironment> Template for EnvironmentConfig<R> {
    fn required_variables(&self) -> Vec<String> {
        self.execution.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut allowed_variables = allowed_variables.clone();
        allowed_variables.extend(self.variable_definitions.iter().map(|vd| &vd.name));

        let ctx = ctx.for_config_file(
            file_source,
            Some(FileType::Environment),
            self.variable_definitions.iter(),
        );

        self.execution
            .validate_context(path, &allowed_variables, file_source, &ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let ctx = ctx.for_config_file(
            source,
            Some(FileType::Environment),
            self.variable_definitions.iter(),
        );

        // We call try_template here instead of try_template_nested to avoid appending
        // an unnecessary entry to the path
        self.execution.try_template(path, source, &ctx)
    }
}

impl<R: RunEnvironment> Check for EnvironmentConfig<R> {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        // Check that hard coded variables are unique
        let all_variables = self.variable_definitions.iter();
        let duplicates = duplicate_keys(all_variables, |v| &v.name);
        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateVariableNames,
                duplicates.join("\n"),
                path,
            );
        }

        // Check that each command is valid in isolation
        // We call try_check here instead of try_check_nested to avoid appending
        // an unnecessary entry to the path
        errs.append(self.execution.try_check(path, ctx));

        errs.into_result(())
    }
}

impl<R: RunEnvironment> CheckArrayDuplicates for EnvironmentConfig<R> {
    const BASE_PATH: &str = "environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        let mut arrays = vec![(
            "variables",
            DedupArray::VariableDef(&mut self.variable_definitions),
        )];
        arrays.extend(self.execution.deduplicated_arrays());

        arrays
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(
    untagged,
    expecting = "expected docker-compose environment (with compose_files) or script environment (with setup/teardown)"
)]
#[allow(clippy::large_enum_variant)] // We only ever allocate one of these, not multiples, so the difference in variant size should not be an issue
pub enum EnvironmentExecution {
    DockerCompose(DockerComposeEnvironment),
    Script(ScriptEnvironment),
}

enum_impl_check!(EnvironmentExecution => Script, DockerCompose);

impl RunEnvironment for EnvironmentExecution {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        match self {
            EnvironmentExecution::DockerCompose(inner) => {
                inner.execute_setup(name, out_dir, ctx).await
            }
            EnvironmentExecution::Script(inner) => inner.execute_setup(name, out_dir, ctx).await,
        }
    }

    async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        match self {
            EnvironmentExecution::DockerCompose(inner) => {
                inner.execute_teardown(name, out_dir, ctx).await
            }
            EnvironmentExecution::Script(inner) => inner.execute_teardown(name, out_dir, ctx).await,
        }
    }
}

impl RunProviders for EnvironmentExecution {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        match self {
            EnvironmentExecution::DockerCompose(inner) => inner.named_providers(),
            EnvironmentExecution::Script(inner) => inner.named_providers(),
        }
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        match self {
            EnvironmentExecution::DockerCompose(inner) => inner.inline(mode, ctx, cache),
            EnvironmentExecution::Script(inner) => inner.inline(mode, ctx, cache),
        }
    }
}

impl CheckArrayDuplicates for EnvironmentExecution {
    const BASE_PATH: &str = "environment_execution";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        match self {
            EnvironmentExecution::DockerCompose(inner) => inner.deduplicated_arrays(),
            EnvironmentExecution::Script(inner) => inner.deduplicated_arrays(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct ScriptEnvironment {
    pub setup: CommandSection,
    pub teardown: CommandSection,
}

impl RunEnvironment for ScriptEnvironment {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.setup
            .run_providers_and_execute_for_output(name, out_dir, ctx)
            .await
    }

    async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.teardown
            .run_providers_and_execute_for_output(name, out_dir, ctx)
            .await
    }
}

impl Check for ScriptEnvironment {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(self.setup.try_check_nested(path, "setup", ctx));
        errs.append(self.teardown.try_check_nested(path, "teardown", ctx));

        errs.into_result(())
    }
}

impl RunProviders for ScriptEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        let mut providers = self.setup.named_providers();
        providers.extend(self.teardown.named_providers());

        providers
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            errs.append(self.setup.inline(mode, ctx, cache).await);
            errs.append(self.teardown.inline(mode, ctx, cache).await);

            errs.into_result(())
        })
    }
}

impl CheckArrayDuplicates for ScriptEnvironment {
    const BASE_PATH: &str = "script_environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![
            (
                "setup.file_providers",
                DedupArray::Nfp(&mut self.setup.file_providers),
            ),
            (
                "teardown.file_providers",
                DedupArray::Nfp(&mut self.teardown.file_providers),
            ),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct DockerComposeEnvironment {
    /// The name of the docker compose project. Defaults to the environment name if not set.
    #[serde(default)]
    #[template(skip)]
    pub project_name: Option<String>,
    /// A list of all the docker compose files to start for this environment
    pub compose_files: Vec<NamedComposeFileProvider>,
    /// A list of all other files the docker compose environment depends on
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
    // Environment variables to set
    #[serde(default)]
    pub env_vars: HashMap<String, Field<Scalar>>,
}

impl DockerComposeEnvironment {
    fn project_name(&self) -> String {
        let raw = self.project_name.as_deref().unwrap_or("rtf_environment");
        slugify_compose_project_name(raw)
    }

    pub async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        RunEnvironment::execute_setup(self, name, out_dir, ctx).await
    }

    pub async fn execute_teardown(
        &self,
        _name: &str,
        _out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let project_name = self.project_name();
        let (cmd, args) = self.teardown_as_command_and_args(&project_name)?;

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &HashMap::new())
            .map_err(|e| providers::Error::CommandFailed {
                name: "docker compose down".to_string(),
                err: e.to_string(),
            })?;

        Ok("{}".to_string())
    }

    fn setup_as_command_and_args(
        &self,
        project_name: &str,
        compose_overrides: &[PathBuf],
        ctx: &impl ResolutionContext,
    ) -> providers::Result<(&'static str, Vec<String>)> {
        let mut args = vec![
            "compose".to_string(),
            "-p".to_string(),
            project_name.to_string(),
        ];

        for file in self.compose_file_paths(ctx)? {
            args.push("-f".to_string());
            args.push(file.to_string_lossy().to_string());
        }

        for file in compose_overrides {
            args.push("-f".to_string());
            args.push(file.to_string_lossy().to_string());
        }

        // Add up command with flags: detached mode, wait for health checks
        args.extend(["up".to_string(), "-d".to_string(), "--wait".to_string()]);

        Ok(("docker", args))
    }

    fn teardown_as_command_and_args(
        &self,
        project_name: &str,
    ) -> providers::Result<(&'static str, Vec<String>)> {
        let mut args = vec![
            "compose".to_string(),
            "-p".to_string(),
            project_name.to_string(),
        ];

        args.push("down".to_string());

        Ok(("docker", args))
    }

    pub fn all_env_vars(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars: HashMap<String, String> = self
            .env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().to_string()))
            .collect();

        for nfp in self.file_providers.iter() {
            let path = ctx
                .known_provider_output_path(Provider::File { fp: &nfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: nfp.name.clone(),
                })?;
            vars.insert(nfp.env_var.clone(), path.to_string_lossy().to_string());
        }

        vars.insert(OUTDIR.to_string(), out_dir.display().to_string());
        vars.insert(OUTPUT_PATH.to_string(), output_path.display().to_string());

        Ok(vars)
    }

    /// Return the names of docker compose services that carry the `rtf.io/file-providers: true`
    /// label across all resolved compose files.
    fn find_labeled_services(
        &self,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<String>> {
        let mut labeled = Vec::new();
        for path in self.compose_file_paths(ctx)? {
            if let Ok(content) = ctx.read_path_to_string(&path) {
                labeled.extend(labeled_services_in_compose(&content));
            }
        }
        labeled.sort();
        labeled.dedup();
        Ok(labeled)
    }

    /// Collect all compose file paths from the resolved providers.
    ///
    /// Handles both single files and directories of compose files. When a provider
    /// outputs a directory, all .yaml/.yml files within it are collected and sorted.
    pub fn compose_file_paths(
        &self,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        for ncfp in self.compose_files.iter() {
            let path = ctx
                .known_provider_output_path(Provider::ComposeFile { fp: &ncfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: ncfp.name.clone(),
                })?;

            match ctx.path_kind(&path) {
                PathKind::File => {
                    paths.push(path);
                }
                PathKind::OccupiedDir => {
                    paths.extend(collect_compose_files(&path)?);
                }
                _ => {
                    return Err(providers::Error::ProviderOutputNotFileOrDir {
                        path_kind: ctx.path_kind(path),
                    });
                }
            }
        }

        Ok(paths)
    }
}

impl RunEnvironment for DockerComposeEnvironment {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let output_path = out_dir.join(OUTPUT_PATH);
        let providers_dir = out_dir.join(PROVIDER_DIR);
        let env_providers_dir = providers_dir.join(format!("{name}_providers"));

        self.run_providers(&env_providers_dir, ctx).await?;

        let labeled_services = self.find_labeled_services(ctx)?;
        let mut compose_overrides = Vec::new();

        if !labeled_services.is_empty() {
            let override_content = generate_local_file_providers_overlay(
                &labeled_services,
                &env_providers_dir,
                &self.file_providers,
            );
            let override_path = out_dir.join("file-providers-override.yaml");
            ctx.write(&override_path, override_content)?;
            compose_overrides.push(override_path);
        }

        let project_name = self.project_name();
        let mut env_vars = self.all_env_vars(out_dir, &output_path, ctx)?;

        // When labeled services are present, the overlay bind-mounts the resolved providers
        // at PROVIDERS_CONTAINER_PATH inside the container. Remap file provider env vars from
        // their host paths (returned by all_env_vars) to the container paths so that ${VAR}
        // references in compose files expand to the correct in-container location.
        if !labeled_services.is_empty() {
            for nfp in self.file_providers.iter() {
                env_vars.insert(
                    nfp.env_var.clone(),
                    format!("{}/{}", PROVIDERS_CONTAINER_PATH, nfp.name),
                );
            }
        }

        let (cmd, args) = self.setup_as_command_and_args(&project_name, &compose_overrides, ctx)?;

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &env_vars)
            .map_err(|e| providers::Error::CommandFailed {
                name: "docker compose up".to_string(),
                err: e.to_string(),
            })?;

        ctx.store_run_metadata(DOCKER_COMPOSE_NETWORK, format!("{project_name}_default"));

        Ok("{}".to_string())
    }

    async fn execute_teardown(
        &self,
        _name: &str,
        _out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let project_name = self.project_name();
        let (cmd, args) = self.teardown_as_command_and_args(&project_name)?;

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &HashMap::new())
            .map_err(|e| providers::Error::CommandFailed {
                name: "docker compose down".to_string(),
                err: e.to_string(),
            })?;

        Ok("{}".to_string())
    }
}

impl CheckArrayDuplicates for DockerComposeEnvironment {
    const BASE_PATH: &str = "docker_compose_environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![
            ("compose_files", DedupArray::Ncfp(&mut self.compose_files)),
            ("file_providers", DedupArray::Nfp(&mut self.file_providers)),
        ]
    }
}

/// Slugify a string into a valid docker compose project name.
///
/// Project names must contain only lowercase letters, decimal digits, dashes, and underscores,
/// and must begin with a lowercase letter or decimal digit.
fn slugify_compose_project_name(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '-' | '_' => c,
            _ => '-',
        })
        .collect();

    // Strip leading characters that aren't a lowercase letter or digit
    while slug.starts_with('-') || slug.starts_with('_') {
        slug.remove(0);
    }

    slug
}

/// Return service names in a compose YAML that carry `rtf.io/file-providers: true`.
fn labeled_services_in_compose(yaml_content: &str) -> Vec<String> {
    let Ok(value) = serde_yaml::from_str::<Value>(yaml_content) else {
        return vec![];
    };

    let Some(services) = value.get("services").and_then(|s| s.as_mapping()) else {
        return vec![];
    };

    services
        .iter()
        .filter_map(|(name, service)| {
            let label = service
                .get("labels")
                .and_then(|l| l.get(FILE_PROVIDERS_LABEL))?;

            let is_set = label.as_bool() == Some(true) || label.as_str() == Some("true");

            if is_set {
                name.as_str().map(str::to_string)
            } else {
                None
            }
        })
        .collect()
}

/// Build a compose override that bind-mounts the resolved providers directory at
/// `/providers` inside each labeled service and remaps file-provider env vars to
/// their container-side paths.
fn generate_local_file_providers_overlay(
    labeled_services: &[String],
    providers_host_path: &Path,
    file_providers: &[NamedFileProvider],
) -> String {
    let mut services_map = Mapping::new();

    for service in labeled_services {
        let mut service_map = Mapping::new();

        let volume = format!(
            "{}:{}",
            providers_host_path.display(),
            PROVIDERS_CONTAINER_PATH
        );
        service_map.insert(
            Value::String("volumes".into()),
            Value::Sequence(vec![Value::String(volume)]),
        );

        if !file_providers.is_empty() {
            let env_entries: Vec<Value> = file_providers
                .iter()
                .map(|nfp| {
                    Value::String(format!(
                        "{}={}/{}",
                        nfp.env_var, PROVIDERS_CONTAINER_PATH, nfp.name
                    ))
                })
                .collect();

            service_map.insert(
                Value::String("environment".into()),
                Value::Sequence(env_entries),
            );
        }

        services_map.insert(Value::String(service.clone()), Value::Mapping(service_map));
    }

    let mut root = Mapping::new();
    root.insert(
        Value::String("services".into()),
        Value::Mapping(services_map),
    );

    serde_yaml::to_string(&Value::Mapping(root)).unwrap_or_default()
}

/// Collect all YAML compose files from a directory.
fn collect_compose_files(dir: &Path) -> providers::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in read_dir(dir)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            files.push(path);
        }
    }

    files.sort();

    Ok(files)
}

impl Check for DockerComposeEnvironment {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        for ncfp in self.compose_files.iter() {
            errs.append(ncfp.try_check_nested(path, "compose_files", ctx));
        }
        for nfp in self.file_providers.iter() {
            errs.append(nfp.try_check_nested(path, "file_providers", ctx));
        }

        errs.into_result(())
    }
}

#[allow(async_fn_in_trait)]
impl RunProviders for DockerComposeEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        let mut providers = self.compose_files.named_providers();
        providers.extend(self.file_providers.named_providers());

        providers
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            errs.append(self.compose_files.inline(mode, ctx, cache).await);
            errs.append(self.file_providers.inline(mode, ctx, cache).await);

            errs.into_result(())
        })
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use crate::{
        context::Context,
        formats::tests::{
            named_file_providers_with_fields, templatable_file_providers, variable_definitions,
        },
        providers::file::{InlineFile, compose::ComposeFileProvider},
        run::Provider,
        templating::Field,
    };
    use assert_fs::{TempDir, prelude::*};
    use std::path::PathBuf;

    /// Create an EnvironmentConfig for testing Template trait methods (has_pending_fields, required_variables)
    pub(crate) fn environment_with_fields(
        setup_fields: &[Field<String>],
        teardown_fields: &[Field<String>],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig<EnvironmentExecution> {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection {
                    file_providers: named_file_providers_with_fields(setup_fields),
                    ..CommandSection::empty()
                },
                teardown: CommandSection {
                    file_providers: named_file_providers_with_fields(teardown_fields),
                    ..CommandSection::empty()
                },
            }),
            ..EnvironmentConfig::empty()
        }
    }

    /// Create a test EnvironmentConfig for template tests
    pub(crate) fn templatable_environment(
        variable_names: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig<EnvironmentExecution> {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            variable_definitions: variable_definitions(variable_names),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection {
                    file_providers: templatable_file_providers(setup_fields),
                    ..CommandSection::empty()
                },
                teardown: CommandSection {
                    file_providers: templatable_file_providers(teardown_fields),
                    ..CommandSection::empty()
                },
            }),
            ..EnvironmentConfig::empty()
        }
    }

    /// Create a named compose file with inline content unique to the name.
    /// Using the name in the content ensures each compose file has a unique provider
    /// identity (since providers are keyed by their serialized content).
    pub(crate) fn named_compose_file(name: &str) -> NamedComposeFileProvider {
        NamedComposeFileProvider {
            name: name.to_string(),
            provider: ComposeFileProvider::Inline(InlineFile {
                content: format!("# {name}\nservices: {{}}"),
            }),
        }
    }

    /// Create a simple DockerComposeEnvironment for command-building tests
    pub(crate) fn docker_compose_env(
        project_name: Option<&str>,
        compose_file_names: &[&str],
    ) -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: project_name.map(String::from),
            compose_files: compose_file_names
                .iter()
                .map(|name| named_compose_file(name))
                .collect(),
            file_providers: Vec::new(),
            env_vars: HashMap::new(),
        }
    }

    /// Create temp files and store their paths in context, returning the paths for use in assertions.
    /// The returned TempDir must be kept alive for the duration of the test to prevent cleanup.
    pub(crate) fn register_compose_paths(
        ctx: &mut Context,
        env: &DockerComposeEnvironment,
    ) -> (TempDir, Vec<String>) {
        let temp_dir = TempDir::new().unwrap();
        let paths = env
            .compose_files
            .iter()
            .map(|ncfp| {
                let file = temp_dir.child(&ncfp.name);
                file.write_str(&format!("# {}\nservices: {{}}", ncfp.name))
                    .unwrap();
                let path = file.path().to_string_lossy().to_string();
                ctx.store_provider_output_path(
                    Provider::ComposeFile { fp: &ncfp.provider },
                    PathBuf::from(&path),
                );
                path
            })
            .collect();
        (temp_dir, paths)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        checks::ErrorKind,
        context::Context,
        formats::{
            Sources,
            environment::test_helpers::{
                docker_compose_env, environment_with_fields, register_compose_paths,
                templatable_environment,
            },
            tests::{
                assert_check_errors, assert_template_errors, expected_error_details, p, r,
                templatable_file_providers, template_context, variable_definitions,
            },
        },
        providers::{
            self,
            command::{
                CommandProvider, CommandSection, CommandSpec,
                test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            },
            file::{
                DirFile, FileProvider, InlineDir, InlineFile, RawSource, RelativeFile,
                RequiredFile, SourceDir, compose::ComposeFileProvider,
            },
            test_helpers::create_temp_dir_with_file,
        },
        templating::Field,
    };
    use assert_fs::{
        fixture::PathChild,
        prelude::{FileWriteStr, PathCreateDir},
    };
    use indoc::{formatdoc, indoc};
    use simple_test_case::test_case;
    use std::{collections::HashMap, path::PathBuf};

    // An example environment config to check parsing and templating
    const TEMPLATED_SCRIPT_ENVIRONMENT: &str = indoc!(
        r#"
        name: environment
        description: a templated environment
        variable_definitions:
          - name: foo
            description: a value foo
            allowed_values: ["foo1", "foo2"]
          - name: bar
            description: a value bar
            default: "bar"
        custom_providers:
          - kind: local
            relative_path: ../providers
            using:
              my_custom_provider: my_custom_provider.yaml
          - kind: github
            org: apollographql
            repo: test-providers
            path: /providers
            git_ref: main
            using:
              another_provider: another_provider.yaml
        setup:
          command:
            name: setup.sh
            kind: relative_path
            path: "/setup.sh"
          file_providers:
            - name: foo.txt
              env_var: FOO
              kind: relative_path
              path: "{{ foo }}"
        teardown:
          command:
            name: teardown.sh
            kind: relative_path
            path: "/teardown.sh"
          file_providers:
            - name: bar.txt
              env_var: BAR
              kind: relative_path
              path: "{{ bar }}"
    "#
    );

    // An example docker compose environment config to check parsing and templating
    const TEMPLATED_COMPOSE_ENVIRONMENT: &str = indoc!(
        r#"
        name: docker-compose-environment
        description: a templated docker compose environment
        variable_definitions:
          - name: foo
            description: a value foo
            allowed_values: ["foo1", "foo2"]
          - name: bar
            description: a value bar
            default: "bar"
        project_name: docker-compose-env
        env_vars:
            ENV_VAR: "value"
        compose_files:
          - name: foo.yaml
            kind: relative_path
            path: "{{ foo }}"
        file_providers:
          - name: bar.txt
            env_var: BAR
            kind: relative_path
            path: "{{ bar }}"
        "#
    );

    #[test]
    fn parse_script_environment_success() {
        let config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(TEMPLATED_SCRIPT_ENVIRONMENT)
                .expect("environment config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a deterministic order for the assert_eq

        assert_eq!(res, &["bar", "foo"], "expected variables to match");
        assert_eq!(config.custom_providers.len(), 2);

        let cp = &config.custom_providers[0];
        assert_eq!(
            cp.source,
            RawSource::Local {
                relative_path: PathBuf::from("../providers")
            }
        );
        assert_eq!(cp.using.len(), 1);
        assert_eq!(
            cp.using.get("my_custom_provider").unwrap(),
            "my_custom_provider.yaml",
        );

        let cp = &config.custom_providers[1];
        assert_eq!(
            cp.source,
            RawSource::Github {
                org: "apollographql".to_string(),
                repo: "test-providers".to_string(),
                path: PathBuf::from("/providers"),
                git_ref: Some("main".to_string())
            }
        );
        assert_eq!(cp.using.len(), 1);
        assert_eq!(
            cp.using.get("another_provider").unwrap(),
            "another_provider.yaml",
        );
    }

    #[test]
    fn parse_compose_environment_success() {
        let config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(TEMPLATED_COMPOSE_ENVIRONMENT)
                .expect("environment config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a deterministic order for the assert_eq

        assert_eq!(res, &["bar", "foo"], "expected variables to match");
        assert_eq!(config.custom_providers.len(), 0);
    }

    #[test_case(&[p("setup1"), p("setup2")], &[p("teardown1"), p("teardown2")], &["setup1", "setup2", "teardown1", "teardown2"]; "both setup and both teardown pending requires variables")]
    #[test_case(&[p("setup1"), p("setup2")], &[p("teardown1"), r("teardown2")], &["setup1", "setup2", "teardown1"]; "both setup and single teardown pending requires variables")]
    #[test_case(&[p("setup1"), p("setup2")], &[r("teardown1"), r("teardown2")], &["setup1", "setup2"]; "both setup and no teardown pending requires variables")]
    #[test_case(&[p("setup1"), r("setup2")], &[p("teardown1"), p("teardown2")], &["setup1", "teardown1", "teardown2"]; "single setup and both teardown pending requires variables")]
    #[test_case(&[p("setup1"), r("setup2")], &[p("teardown1"), r("teardown2")], &["setup1", "teardown1"]; "single setup and single teardown pending requires variables")]
    #[test_case(&[p("setup1"), r("setup2")], &[r("teardown1"), r("teardown2")], &["setup1"]; "single setup and no teardown pending requires variables")]
    #[test_case(&[r("setup1"), r("setup2")], &[p("teardown1"), p("teardown2")], &["teardown1", "teardown2"]; "no setup and both teardown pending requires variables")]
    #[test_case(&[r("setup1"), r("setup2")], &[p("teardown1"), r("teardown2")], &["teardown1"]; "no setup and single teardown pending requires variables")]
    #[test_case(&[r("setup1"), r("setup2")], &[r("teardown1"), r("teardown2")], &[]; "no setup and no teardown pending requires no variables")]
    #[test]
    fn required_variables(
        setup_fields: &[Field<String>],
        teardown_fields: &[Field<String>],
        expected: &[&str],
    ) {
        let environment = environment_with_fields(setup_fields, teardown_fields, &[]);

        let res = environment.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test_case(&["setup1", "setup2"], &["teardown1", "teardown2"]; "setup multi variable and teardown multi variable")]
    #[test_case(&["setup1", "setup2"], &["teardown1"]; "setup multi variable and teardown single variable")]
    #[test_case(&["setup1", "setup2"], &[]; "setup multi variable and teardown no variable")]
    #[test_case(&["setup1"], &["teardown1", "teardown2"]; "setup single variable and teardown multi variable")]
    #[test_case(&["setup1"], &["teardown1"]; "setup single variable and teardown single variable")]
    #[test_case(&["setup1"], &[]; "setup single variable and teardown no variable")]
    #[test_case(&[], &["teardown1", "teardown2"]; "setup no variable and teardown multi variable")]
    #[test_case(&[], &["teardown1"]; "setup no variable and teardown single variable")]
    #[test_case(&[], &[]; "setup no variable and teardown no variable")]
    #[test]
    fn try_template_script_succeeds(setup_fields: &[&str], teardown_fields: &[&str]) {
        let mut field_names: Vec<&str> = setup_fields.to_vec();
        field_names.extend_from_slice(teardown_fields);

        let ctx = template_context(field_names.as_slice());
        let mut environment =
            templatable_environment(field_names.as_slice(), setup_fields, teardown_fields, &[]);

        let res = environment.try_template(&mut Vec::new(), &StableSource::Environment, &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test]
    fn try_template_docker_compose_succeeds() {
        let field_names = &["compose", "file", "env"];
        let ctx = template_context(field_names);

        let mut env_vars = HashMap::new();
        env_vars.insert("ENV_VAR".to_string(), Field::Pending("env".to_string()));

        let mut environment = EnvironmentConfig {
            variable_definitions: variable_definitions(field_names),
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                project_name: None,
                compose_files: vec![NamedComposeFileProvider {
                    name: "compose.yaml".to_string(),
                    provider: ComposeFileProvider::RelativePath(RelativeFile {
                        path: Field::Pending("compose".to_string()),
                        src: None,
                    }),
                }],
                file_providers: templatable_file_providers(&["file"]),
                env_vars,
            }),
            ..EnvironmentConfig::empty()
        };

        let res = environment.try_template(&mut Vec::new(), &StableSource::Environment, &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    /// Helper function for asserting template errors are as expected
    fn assert_env_template_errors(
        environment: &mut EnvironmentConfig<EnvironmentExecution>,
        ctx: TemplateContext,
        expected_setup_err_fields: &[&str],
        expected_teardown_err_fields: &[&str],
    ) {
        let (mut expected_err_messages, mut expected_err_paths) =
            expected_error_details(expected_setup_err_fields, "setup");
        let (expected_messages, expected_paths) =
            expected_error_details(expected_teardown_err_fields, "teardown");
        expected_err_messages.extend(expected_messages);
        expected_err_paths.extend(expected_paths);

        assert_template_errors(environment, ctx, expected_err_messages, expected_err_paths);
    }

    #[test_case(&["missing"], &["setup"], &["setup"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["setup1", "setup2"], &["setup1", "setup2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["setup1", "missing2"], &["setup1", "setup2"], &["setup2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_setup_missing_variable_definitions(
        variable_defs: &[&str],
        setup_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["setup", "setup1", "setup2"]);
        let mut environment = templatable_environment(variable_defs, setup_fields, &[], &[]);

        assert_env_template_errors(&mut environment, ctx, expected_err_fields, &[]);
    }

    #[test_case(&["missing"], &["teardown"], &["teardown"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["teardown1", "teardown2"], &["teardown1", "teardown2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["teardown1", "missing2"], &["teardown1", "teardown2"], &["teardown2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_teardown_missing_variable_definitions(
        variable_defs: &[&str],
        teardown_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["teardown", "teardown1", "teardown2"]);
        let mut environment = templatable_environment(variable_defs, &[], teardown_fields, &[]);

        assert_env_template_errors(&mut environment, ctx, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variable_definitions() {
        let ctx = template_context(&["setup", "teardown"]);
        let mut environment = templatable_environment(&[], &["setup"], &["teardown"], &[]);

        assert_env_template_errors(&mut environment, ctx, &["setup"], &["teardown"]);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variables_not_provided() {
        let ctx = template_context(&[]);
        let mut environment =
            templatable_environment(&["setup", "teardown"], &["setup"], &["teardown"], &[]);

        assert_env_template_errors(&mut environment, ctx, &["setup"], &["teardown"]);
    }

    // The success test is here to complete the matrix of failure tests below (i.e. no failure)
    // It shows all parts of the env config that *could* fail not failing. In reality we could
    // just use an "empty" config here and get the same result but this is a more illustrative
    // example
    #[test]
    fn check_script_success() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: cmd_with_inline_file(),
                teardown: cmd_with_inline_file(),
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let res = environment.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn check_docker_compose_success() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                project_name: None,
                compose_files: vec![NamedComposeFileProvider {
                    name: "compose.yaml".to_string(),
                    provider: ComposeFileProvider::Inline(InlineFile {
                        content: "content".to_string(),
                    }),
                }],
                file_providers: vec![NamedFileProvider {
                    name: "file.txt".to_string(),
                    env_var: "FILE".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "content".to_string(),
                    }),
                }],
                env_vars: HashMap::new(),
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let res = environment.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test_case(
        cmd_with_required_file(),
        CommandSection::empty(),
        &[],
        &[ErrorKind::RequiredFileMissing];
        "setup only"
    )]
    #[test_case(
        CommandSection::empty(),
        cmd_with_required_file(),
        &[],
        &[ErrorKind::RequiredFileMissing];
        "teardown only"
    )]
    #[test_case(
        cmd_with_required_file(),
        cmd_with_required_file(),
        &["foo", "foo"],
        &[ErrorKind::DuplicateVariableNames, ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "environment duplicate variables"
    )]
    #[test]
    fn try_check_errors(
        setup_command: CommandSection,
        teardown_command: CommandSection,
        variables: &[&str],
        expected_err_kinds: &[ErrorKind],
    ) {
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(variables),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: setup_command,
                teardown: teardown_command,
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        assert_check_errors(environment, &ctx, expected_err_kinds);
    }

    #[test]
    fn try_check_docker_compose_compose_file_errors() {
        let variables = &["foo"];
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(variables),
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                project_name: None,
                compose_files: vec![NamedComposeFileProvider {
                    name: "compose.yaml".to_string(),
                    provider: ComposeFileProvider::Required(RequiredFile {
                        message: "this is a required file".to_string(),
                    }),
                }],
                file_providers: Vec::new(),
                env_vars: HashMap::new(),
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let expected_err_kinds = &[checks::ErrorKind::RequiredFileMissing];

        assert_check_errors(environment, &ctx, expected_err_kinds);
    }

    #[test]
    fn try_check_docker_compose_file_provider_errors() {
        let variables = &["foo"];
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(variables),
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                project_name: None,
                compose_files: Vec::new(),
                file_providers: vec![NamedFileProvider {
                    name: "file.txt".to_string(),
                    env_var: "FILE".to_string(),
                    provider: FileProvider::Required(RequiredFile {
                        message: "this is a required file".to_string(),
                    }),
                }],
                env_vars: HashMap::new(),
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let expected_err_kinds = &[checks::ErrorKind::RequiredFileMissing];

        assert_check_errors(environment, &ctx, expected_err_kinds);
    }

    #[tokio::test]
    async fn custom_provider_try_load_all_unknown_file_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([("missing_provider".into(), "missing.yaml".into())]),
        };

        let res = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await;

        assert!(res.is_err());
        let errors = res.unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "missing_provider");
        assert!(matches!(errors[0].1, providers::Error::Io(_)));
    }

    const CUSTOM_PROVIDER_WITH_NESTED: &str = indoc!(
        r#"
        name: invalid provider
        description: A custom provider with nested custom_providers
        variable_definitions: []
        command:
          name: script.sh
          kind: relative_path
          path: ./script.sh
        custom_providers:
          - kind: local
            relative_path: ./nested
            using:
              nested_provider: nested.yaml
        "#
    );

    #[tokio::test]
    async fn custom_provider_try_load_all_nested_custom_provider_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        providers_dir
            .child("invalid.yaml")
            .write_str(CUSTOM_PROVIDER_WITH_NESTED)
            .unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([("invalid_provider".into(), "invalid.yaml".into())]),
        };

        let result = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await;

        assert!(result.is_err());
        let errors = result.unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "invalid_provider");
        assert!(matches!(
            errors[0].1,
            providers::Error::NestedCustomProvider
        ));
    }

    #[tokio::test]
    async fn custom_provider_try_load_all_multiple_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let valid_provider = indoc!(
            r#"
            name: valid provider
            description: A valid custom provider
            variable_definitions: []
            command:
              name: script.sh
              kind: relative_path
              path: ./script.sh
            "#
        );

        providers_dir
            .child("valid.yaml")
            .write_str(valid_provider)
            .unwrap();

        providers_dir
            .child("invalid.yaml")
            .write_str(CUSTOM_PROVIDER_WITH_NESTED)
            .unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([
                ("valid_provider".into(), "valid.yaml".into()),
                ("invalid_provider".into(), "invalid.yaml".into()),
                ("missing_provider".into(), "missing.yaml".into()),
            ]),
        };

        let result = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await;

        assert!(result.is_err());
        let errors = result.unwrap_err();

        assert_eq!(errors.len(), 2);

        // Errors should be sorted alphabetically by key in the "using" map
        assert_eq!(errors[0].0, "invalid_provider");
        assert_eq!(errors[1].0, "missing_provider");

        assert!(matches!(
            errors[0].1,
            providers::Error::NestedCustomProvider
        ));
        assert!(matches!(errors[1].1, providers::Error::Io(_)));
    }

    #[tokio::test]
    async fn custom_providers_integration() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let simple_provider = indoc!(
            r#"
        name: simple provider
        description: A simple custom provider for integration testing
        variable_definitions:
          - name: test_var
            description: a test variable
        command:
          name: provider.sh
          kind: relative_path
          path: ./provider.sh
        "#
        );

        providers_dir
            .child("provider1.yaml")
            .write_str(simple_provider)
            .unwrap();
        providers_dir
            .child("provider2.yaml")
            .write_str(simple_provider)
            .unwrap();

        let env_config_yaml = indoc!(
            r#"
            name: test environment
            description: Environment with custom providers
            variable_definitions: []
            custom_providers:
              - kind: local
                relative_path: providers
                using:
                  provider1: provider1.yaml
                  provider2: provider2.yaml
            setup:
              command:
                name: setup.sh
                kind: relative_path
                path: ./setup.sh
            teardown:
              command:
                name: teardown.sh
                kind: relative_path
                path: ./teardown.sh
            "#
        );

        let env_config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(env_config_yaml).expect("environment config to parse");

        assert_eq!(env_config.custom_providers.len(), 1);

        let declaration = &env_config.custom_providers[0];
        let loaded_providers = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await
            .expect("custom providers should load successfully");

        assert_eq!(loaded_providers.len(), 2);

        let (provider1_source, provider1_def) = loaded_providers
            .get("provider1")
            .expect("provider1 should exist");
        assert_eq!(
            provider1_source,
            &SourceDir::local(providers_dir.canonicalize().unwrap())
        );
        assert_eq!(provider1_def.name, "simple provider");
        assert_eq!(
            provider1_def.description,
            "A simple custom provider for integration testing"
        );
        assert_eq!(provider1_def.variable_definitions.len(), 1);

        let (provider2_source, provider2_def) = loaded_providers
            .get("provider2")
            .expect("provider2 should exist");
        assert_eq!(
            provider2_source,
            &SourceDir::local(providers_dir.canonicalize().unwrap())
        );
        assert_eq!(provider2_def.name, "simple provider");
        assert_eq!(
            provider2_def.description,
            "A simple custom provider for integration testing"
        );
        assert_eq!(provider2_def.variable_definitions.len(), 1);
    }

    #[tokio::test]
    async fn script_environment_config_inline_succeeds() {
        let file_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", file_content);

        let mut ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());
        ctx.set_sources(Sources::with_custom_providers(
            SourceDir::default(),
            None,
            Some(src.clone()),
            Default::default(),
            Default::default(),
        ));

        let relative_command_provider = CommandProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let mut environment = EnvironmentConfig {
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection {
                    command: CommandSpec {
                        name: "setup.sh".to_string(),
                        command_provider: relative_command_provider.clone(),
                        args: Vec::new(),
                    },
                    ..CommandSection::empty()
                },
                teardown: CommandSection {
                    command: CommandSpec {
                        name: "teardown.sh".to_string(),
                        command_provider: relative_command_provider,
                        args: Vec::new(),
                    },
                    ..CommandSection::empty()
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let result = environment
            .inline(&InlineMode::All, &ctx, &mut HashMap::new())
            .await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");

        let expected_inline = CommandProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        let script = match &environment.execution {
            EnvironmentExecution::Script(script) => script,
            _ => panic!("Expected EnvironmentExecution::Script variant"),
        };

        assert_eq!(
            script.setup.command.command_provider, expected_inline,
            "Expected setup command provider to be inlined"
        );
        assert_eq!(
            script.teardown.command.command_provider, expected_inline,
            "Expected teardown command provider to be inlined"
        );
    }

    #[tokio::test]
    async fn docker_compose_environment_config_inline_succeeds() {
        let file_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", file_content);

        let mut ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());
        ctx.set_sources(Sources::with_custom_providers(
            SourceDir::default(),
            None,
            Some(src.clone()),
            Default::default(),
            Default::default(),
        ));

        let relative_compose_file = ComposeFileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let relative_file = FileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let mut environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                project_name: None,
                compose_files: vec![NamedComposeFileProvider {
                    name: "compose.yaml".to_string(),
                    provider: relative_compose_file,
                }],
                file_providers: vec![NamedFileProvider {
                    name: "file.txt".to_string(),
                    env_var: "FILE".to_string(),
                    provider: relative_file,
                }],
                env_vars: HashMap::new(),
            }),
            ..EnvironmentConfig::empty()
        };

        let result = environment
            .inline(&InlineMode::All, &ctx, &mut HashMap::new())
            .await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");

        let expected_inline_compose_file = NamedComposeFileProvider {
            name: "compose.yaml".to_string(),
            provider: ComposeFileProvider::Inline(InlineFile {
                content: "example file content".to_string(),
            }),
        };
        let expected_inline_file = NamedFileProvider {
            name: "file.txt".to_string(),
            env_var: "FILE".to_string(),
            provider: FileProvider::Inline(InlineFile {
                content: "example file content".to_string(),
            }),
        };

        let compose = match &environment.execution {
            EnvironmentExecution::DockerCompose(compose) => compose,
            _ => panic!("Expected EnvironmentExecution::DockerCompose variant"),
        };

        assert_eq!(
            compose.compose_files[0], expected_inline_compose_file,
            "Expected compose file to be inlined"
        );
        assert_eq!(
            compose.file_providers[0], expected_inline_file,
            "Expected file provider to be inlined"
        );
    }

    #[test]
    fn docker_compose_setup_as_command_and_args_builds_correct_command() {
        let docker_compose = docker_compose_env(Some("test-project"), &["compose.yaml"]);

        let mut ctx = Context::new();
        let (_temp_dir, paths) = register_compose_paths(&mut ctx, &docker_compose);

        let result = docker_compose.setup_as_command_and_args("test-project", &[], &ctx);
        assert!(result.is_ok(), "Expected command to build successfully");

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "docker");
        assert_eq!(
            args,
            vec![
                "compose",
                "-p",
                "test-project",
                "-f",
                &paths[0],
                "up",
                "-d",
                "--wait"
            ]
        );
    }

    #[test]
    fn docker_compose_setup_with_multiple_compose_files() {
        let docker_compose =
            docker_compose_env(Some("multi-compose"), &["base.yaml", "overlay.yaml"]);

        let mut ctx = Context::new();
        let (_temp_dir, paths) = register_compose_paths(&mut ctx, &docker_compose);

        let result = docker_compose.setup_as_command_and_args("multi-compose", &[], &ctx);
        assert!(result.is_ok(), "Expected command to build successfully");

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "docker");
        assert_eq!(
            args,
            vec![
                "compose",
                "-p",
                "multi-compose",
                "-f",
                &paths[0],
                "-f",
                &paths[1],
                "up",
                "-d",
                "--wait"
            ]
        );
    }

    #[test]
    fn docker_compose_teardown_as_command_and_args_builds_correct_command() {
        let docker_compose = docker_compose_env(Some("test-project"), &["compose.yaml"]);

        let result = docker_compose.teardown_as_command_and_args("test-project");
        assert!(result.is_ok(), "Expected command to build successfully");

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "docker");
        assert_eq!(args, vec!["compose", "-p", "test-project", "down"]);
    }

    #[test]
    fn docker_compose_command_fails_when_compose_file_path_unknown() {
        let docker_compose = docker_compose_env(Some("test-project"), &["compose.yaml"]);

        // Context without stored provider path
        let ctx = Context::new();

        let result = docker_compose.setup_as_command_and_args("test-project", &[], &ctx);
        assert!(
            result.is_err(),
            "Expected error when compose file path unknown"
        );

        let err = result.unwrap_err();
        assert!(
            matches!(&err, providers::Error::MissingProviderOutput { name } if name == "compose.yaml"),
            "Expected MissingProviderOutput error, got {err:?}"
        );
    }

    #[test]
    fn docker_compose_all_env_vars_includes_file_providers_and_explicit_vars() {
        let file_provider = FileProvider::Inline(InlineFile {
            content: "file content".to_string(),
        });

        let mut explicit_env_vars = HashMap::new();
        explicit_env_vars.insert(
            "EXPLICIT_VAR".to_string(),
            Field::Resolved(Scalar::from("explicit_value")),
        );

        let docker_compose = DockerComposeEnvironment {
            project_name: Some("test-project".to_string()),
            compose_files: Vec::new(),
            file_providers: vec![NamedFileProvider {
                name: "config.txt".to_string(),
                env_var: "CONFIG_FILE".to_string(),
                provider: file_provider.clone(),
            }],
            env_vars: explicit_env_vars,
        };

        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::File { fp: &file_provider },
            PathBuf::from("/tmp/providers/config.txt"),
        );

        let out_dir = PathBuf::from("/tmp/output");
        let output_path = out_dir.join("rtf_output");

        let result = docker_compose.all_env_vars(&out_dir, &output_path, &ctx);
        assert!(result.is_ok(), "Expected env vars to be built successfully");

        let env_vars = result.unwrap();

        assert_eq!(
            env_vars.get("CONFIG_FILE"),
            Some(&"/tmp/providers/config.txt".to_string()),
        );
        assert_eq!(
            env_vars.get("EXPLICIT_VAR"),
            Some(&"explicit_value".to_string()),
        );
        assert_eq!(env_vars.get("OUTDIR"), Some(&"/tmp/output".to_string()));
        assert_eq!(
            env_vars.get("RTF_OUTPUT"),
            Some(&"/tmp/output/rtf_output".to_string()),
        );
    }

    #[test]
    fn docker_compose_uses_environment_name_when_project_name_not_set() {
        let docker_compose = docker_compose_env(None, &["compose.yaml"]);

        let mut ctx = Context::new();
        let _temp_dir = register_compose_paths(&mut ctx, &docker_compose);

        // Pass "fallback-name" as the environment name
        let result = docker_compose.setup_as_command_and_args("fallback-name", &[], &ctx);
        assert!(result.is_ok(), "Expected command to build successfully");

        let (_, args) = result.unwrap();
        // The project name should be "fallback-name" (passed as argument)
        assert_eq!(args[2], "fallback-name");
    }

    #[test]
    fn docker_compose_setup_with_inline_dir_multiple_files() {
        // Create a temp directory with compose files to simulate InlineDir output
        let (tmp, base_file) = create_temp_dir_with_file("base.yaml", "version: '3'");
        let overlay_file = tmp.child("overlay.yaml");
        overlay_file.write_str("version: '3'").unwrap();

        let inline_dir = InlineDir {
            files: vec![
                DirFile {
                    path: base_file.path().to_path_buf(),
                    content: "version: '3'".to_string(),
                },
                DirFile {
                    path: overlay_file.path().to_path_buf(),
                    content: "version: '3'".to_string(),
                },
            ],
        };

        let docker_compose = DockerComposeEnvironment {
            project_name: Some("inline-dir-test".to_string()),
            compose_files: vec![NamedComposeFileProvider {
                name: "compose-dir".to_string(),
                provider: ComposeFileProvider::InlineDir(inline_dir),
            }],
            file_providers: Vec::new(),
            env_vars: HashMap::new(),
        };

        // Register the directory path (not individual files)
        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::ComposeFile {
                fp: &docker_compose.compose_files[0].provider,
            },
            tmp.path().to_path_buf(),
        );

        let res = docker_compose.setup_as_command_and_args("inline-dir-test", &[], &ctx);
        assert!(res.is_ok(), "Expected command to build successfully");

        let (cmd, args) = res.unwrap();
        assert_eq!(cmd, "docker");

        // Verify structure: compose -p name -f file1 -f file2 up -d --wait
        let expected_args = vec![
            "compose".to_string(),
            "-p".to_string(),
            "inline-dir-test".to_string(),
            "-f".to_string(),
            base_file.to_str().unwrap().to_string(),
            "-f".to_string(),
            overlay_file.to_str().unwrap().to_string(),
            "up".to_string(),
            "-d".to_string(),
            "--wait".to_string(),
        ];
        assert_eq!(args, expected_args, "expected args to match");
    }

    #[test_case("already-valid", "already-valid"; "already valid name unchanged")]
    #[test_case("My Environment", "my-environment"; "uppercase and spaces")]
    #[test_case("foo.bar.baz", "foo-bar-baz"; "dots replaced with dashes")]
    #[test_case("--leading-dashes", "leading-dashes"; "leading dashes stripped")]
    #[test_case("__leading_underscores", "leading_underscores"; "leading underscores stripped")]
    #[test_case("MiXeD_CaSe-123", "mixed_case-123"; "mixed case lowered")]
    #[test_case("foo@bar!baz", "foo-bar-baz"; "special chars replaced")]
    #[test_case("123-starts-with-digit", "123-starts-with-digit"; "leading digit preserved")]
    #[test]
    fn slugify_compose_project_name_produces_valid_name(input: &str, expected: &str) {
        assert_eq!(slugify_compose_project_name(input), expected);
    }

    #[test_case("true", &["svc"]; "detects bool true")]
    #[test_case("\"true\"", &["svc"]; "detects string true")]
    #[test_case("false", &[]; "ignores false")]
    #[test]
    fn labeled_services_in_compose(label_value: &str, res: &[&str]) {
        let yaml = formatdoc!(
            r#"
            services:
              svc:
                labels:
                  rtf.io/file-providers: {label_value}"#
        );

        assert_eq!(labeled_services_in_compose(&yaml), res.to_vec());
    }

    #[test]
    fn labeled_services_in_compose_only_returns_labeled_services() {
        let yaml = indoc!(
            r#"
            services:
              labeled:
                labels:
                  rtf.io/file-providers: true
              unlabeled:
                image: nginx"#
        );

        assert_eq!(labeled_services_in_compose(yaml), vec!["labeled"]);
    }

    #[test]
    fn labeled_services_in_compose_returns_empty_for_invalid_yaml() {
        assert!(labeled_services_in_compose("{ not: valid: yaml: [").is_empty());
    }

    #[test]
    fn generate_local_file_providers_overlay_adds_volume_and_env_vars() {
        let providers = vec![NamedFileProvider {
            name: "config.yaml".to_string(),
            env_var: "MY_CONFIG".to_string(),
            provider: FileProvider::Inline(InlineFile {
                content: "x".to_string(),
            }),
        }];

        let out = generate_local_file_providers_overlay(
            &["svc".to_string()],
            Path::new("/out/providers/setup_providers"),
            &providers,
        );

        let expected = indoc!(
            r#"
            services:
              svc:
                volumes:
                - /out/providers/setup_providers:/providers
                environment:
                - MY_CONFIG=/providers/config.yaml
            "#
        );

        assert_eq!(out, expected);
    }

    #[test]
    fn generate_local_file_providers_overlay_no_env_vars_when_no_providers() {
        let out = generate_local_file_providers_overlay(
            &["svc".to_string()],
            Path::new("/out/providers/setup_providers"),
            &[],
        );

        let expected = indoc!(
            r#"
            services:
              svc:
                volumes:
                - /out/providers/setup_providers:/providers
            "#
        );

        assert_eq!(out, expected);
    }
}

//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    checks::{self, Check},
    context::{PathKind, ResolutionContext},
    enum_impl_check, providers,
    templating::{self, Field, Scalar, Template},
};
use rtf_core::github::Client;
use rtf_derive::Template;
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fmt, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

pub mod apollo;
pub mod github;
mod source;
pub mod utility;

pub use source::{RawSource, Source};

/// Something that can obtain or synthesise utf-8 file content based on a user provided
/// specification.
///
/// This trait is deliberately pub(crate) rather than pub so that the validation and resolution
/// logic is only exposed through the public API as part of the methods on the config file structs.
#[allow(async_fn_in_trait)]
pub(crate) trait AsUtf8FileContent:
    Check + Serialize + DeserializeOwned + fmt::Debug
{
    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_get_file_content(
        &self,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String>;
}

/// Helper macro for stamping out implementations of the AsUtf8FileContent trait on an enum where
/// each variant is a wrapper around a type that already implements the trait.
#[macro_export]
macro_rules! enum_impl_as_utf8_file_content {
    ($enum:ident => $($variant:ident),+) => {
        impl AsUtf8FileContent for $enum {
            async fn try_get_file_content(
                &self,
                src: &Source,
                ctx: &impl ResolutionContext,
            ) -> providers::Result<String> {
                match self {
                    $(Self::$variant(inner) => inner.try_get_file_content(src, ctx).await,)+
                }
            }
        }
    };
}

impl<T> ResolveFileContent for T
where
    T: AsUtf8FileContent,
{
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<Vec<(PathBuf, String)>> {
        Ok(vec![(
            target.as_ref().to_path_buf(),
            self.try_get_file_content(src, ctx).await?,
        )])
    }
}

/// Something that can obtain or synthesise the contents of multiple utf-8 files based on a user
/// provided specification.
#[allow(async_fn_in_trait)]
pub(crate) trait ResolveFileContent:
    Check + Serialize + DeserializeOwned + fmt::Debug
{
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<Vec<(PathBuf, String)>>;
}

impl<T> ResolveAndWrite for T
where
    T: ResolveFileContent,
{
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        let files = self.try_get_all_file_contents(target, src, ctx).await?;
        for (path, content) in files.into_iter() {
            if let Some(parent) = path.parent() {
                ctx.create_dir_all(parent)?;
            }
            ctx.write(path, content)?;
        }

        Ok(())
    }
}

/// Logic for running a file provider and writing its output to the target [Path].
///
/// Most [FileProvider] implementations can safely ignore providing a custom implementation for
/// this trait if all they need to do is write out a single file, and instead just implement
/// [AsUtf8FileContent] or [ResolveFileContent] which will give a default implementation of this
/// trait. If however you need to write out multiple files or run some additional logic after
/// writing out a file (such as making it executable) then you should implement this trait
/// directly.
#[allow(async_fn_in_trait)]
pub(crate) trait ResolveAndWrite: Check + Serialize + DeserializeOwned + fmt::Debug {
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()>;
}

/// Helper macro for stamping out implementations of the ResolveAndWrite trait on an enum where
/// each variant is a wrapper around a type that already implements the trait.
#[macro_export]
macro_rules! enum_impl_resolve_and_write {
    ($enum:ident => $($variant:ident),+) => {
        impl ResolveAndWrite for $enum {
            async fn resolve_and_write(
                &self,
                target: impl AsRef<Path>,
                src: &Source,
                ctx: &mut impl ResolutionContext,
            ) -> $crate::providers::Result<()> {
                match self {
                    $(Self::$variant(inner) => inner.resolve_and_write(target, src, ctx).await,)+
                }
            }
        }
    };
}

/// # Named File Provider
///
/// Shared metadata that wraps every file provider.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct NamedFileProvider {
    /// The to use for the output produced by this provider
    ///
    /// This can be either a file or a directory depending on the file provider.
    pub name: String,
    /// The environment variable to place the absolute path to this providers output
    pub env_var: String,
    #[serde(flatten)]
    pub provider: FileProvider,
}

impl Deref for NamedFileProvider {
    type Target = FileProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

impl DerefMut for NamedFileProvider {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.provider
    }
}

// Adding this custom implementation so that file providers and the environment variable
// used to identify the file provider are added to the path when templating a
// NamedFileProvider
impl Template for NamedFileProvider {
    fn has_pending_fields(&self) -> bool {
        self.provider.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        self.provider.required_values()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        path.pop();
        path.push("file_providers".to_string());

        let tail = self.env_var.clone();
        errs.append(self.provider.try_template_nested(path, &tail, values));

        errs.into_result(())
    }
}

/// # File Provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(rename_all = "snake_case", tag = "kind")]
// If you are adding a FileProvider please make sure to also add a parse test to
// the all_fields_templated test in this file
pub enum FileProvider {
    BuildRouterFromSource(apollo::BuildRouterFromSource),
    FromCommand(utility::FromCommand),
    GithubFile(github::GithubFile),
    GraphosCannedOps(apollo::GraphosCannedOps),
    GraphosCannedOpsById(apollo::GraphosCannedOpsById),
    GraphosSubgraphDockerCompose(apollo::GraphosSubgraphDockerCompose),
    GraphosSubgraphRouterUrlOverrides(apollo::GraphosSubgraphRouterUrlOverrides),
    GraphosSubgraphs(apollo::GraphosSubgraphs),
    GraphosSupergraph(apollo::GraphosSupergraph),
    Inline(InlineFile),
    MergeYaml(utility::MergeYaml),
    OfflineGraphosLicense(apollo::OfflineGraphosLicense),
    RelativePath(RelativeFile),
    Required(RequiredFile),
    ResolvedValues(ResolvedValues),
    RouterDownloadScript(apollo::RouterDownloadScript),
}

impl FileProvider {
    pub fn json_schema() -> serde_json::Value {
        let mut settings = SchemaSettings::default();
        settings.inline_subschemas = true;
        let generator = settings.into_generator();
        let schema = generator.into_root_schema_for::<Self>();

        schema.to_value()
    }
}

// Each time we add a new variant to the FileProvider enum above we need to remember to add it to
// the macro invocation below in order to update the trait implementations for the enum. (You can't
// really forget to do this as the compiler will complain about missing match arms if you do!)
macro_rules! enum_impl_file_provider {
    ($($variant:ident,)+) => {
        enum_impl_check!(FileProvider => $($variant),+);
        enum_impl_resolve_and_write!(FileProvider => $($variant),+);
    };
}

enum_impl_file_provider!(
    BuildRouterFromSource,
    FromCommand,
    GithubFile,
    GraphosCannedOps,
    GraphosCannedOpsById,
    GraphosSubgraphDockerCompose,
    GraphosSubgraphRouterUrlOverrides,
    GraphosSubgraphs,
    GraphosSupergraph,
    Inline,
    MergeYaml,
    OfflineGraphosLicense,
    RelativePath,
    Required,
    ResolvedValues,
    RouterDownloadScript,
);

/// # Inline File
///
/// The simplest form of file provider: the user specifies the contents of the file inline within
/// their config file.
///
/// ```yaml
/// - name: "my-file.txt"
///   env_var: MY_FILE
///   kind: inline
///   content: |
///     my raw file content.
///     specified inline within an RTF config file.
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Template)]
pub struct InlineFile {
    /// The text to write out as the contents of the generated file.
    #[template(skip)]
    pub(crate) content: String,
}

impl AsUtf8FileContent for InlineFile {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        Ok(self.content.clone())
    }
}

impl Check for InlineFile {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

/// # Relative Path
///
/// A relative path from the containing config file to a target file that should be made available
/// as part of the test run. This provider works both with local files and files within GitHub
/// if the containing config file was pulled from a repository.
///
/// ```yaml
/// - name: "my-file.txt"
///   env_var: MY_FILE
///   kind: relative_path
///   path: "../../resources/test-data/my-file.txt"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct RelativeFile {
    /// The relative path from the containing config file to the target file.
    pub(crate) path: Field<String>,

    /// Set during TestPlan parsing as part of overrides. This should only ever be `Some` if this
    /// provider was defined as part of an `overrides` section in the test plan.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    #[template(skip)]
    pub(crate) src: Option<Source>,
}

impl RelativeFile {
    fn format_error_message(&self) -> String {
        format!("provided path was {:?}", self.path)
    }
}

impl AsUtf8FileContent for RelativeFile {
    async fn try_get_file_content(
        &self,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        // Prefer an explicitly provided Source if one was set during parsing of the test plan
        // as part of applying overrides.
        let src = self.src.as_ref().unwrap_or(src);

        match src {
            Source::Local { abs_path } => {
                let dir = match abs_path.parent() {
                    Some(dir) => dir.to_path_buf(),
                    None => PathBuf::new(),
                };
                let p = dir.join(self.path.as_resolved());
                Ok(ctx.read_path_to_string(p)?)
            }

            Source::Github {
                org,
                repo,
                path,
                git_ref,
            } => {
                let client = ctx
                    .github_client()
                    .ok_or(providers::Error::Github(rtf_core::github::Error::NoClient))?;
                let full_path = match path.parent() {
                    Some(parent) => parent.join(self.path.as_resolved()).display().to_string(),
                    None => self.path.as_resolved().to_string(),
                };

                Ok(client
                    .string_file_content(org, repo, &full_path, git_ref.as_ref())
                    .await?)
            }
        }
    }
}

impl Check for RelativeFile {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        // Prefer an explicitly provided Source if one was set during parsing of the test plan
        // as part of applying overrides.
        let src = self.src.as_ref().unwrap_or(src);

        let res = match src {
            Source::Local { abs_path } => {
                let dir = match abs_path.parent() {
                    Some(dir) => dir.to_path_buf(),
                    None => PathBuf::new(),
                };
                ctx.canonicalize_path(dir.join(self.path.as_resolved()))
            }

            Source::Github { .. } => {
                return if ctx.github_client().is_none() {
                    Err(checks::Errors::new(
                        checks::ErrorKind::MissingGithubApiKey,
                        "",
                        path,
                    ))
                } else {
                    Ok(())
                };
            }
        };

        let p = match res {
            Ok(p) => p,
            Err(e) => {
                let kind = if e.kind() == io::ErrorKind::NotFound {
                    checks::ErrorKind::FileNotFound
                } else {
                    checks::ErrorKind::InvalidRelativePath
                };

                return Err(checks::Errors::new(kind, self.format_error_message(), path));
            }
        };

        match ctx.path_kind(&p) {
            PathKind::File => Ok(()),
            PathKind::EmptyDir | PathKind::OccupiedDir => Err(checks::Errors::new(
                checks::ErrorKind::IsADirectory,
                self.format_error_message(),
                path,
            )),
            PathKind::Missing => Err(checks::Errors::new(
                checks::ErrorKind::FileNotFound,
                self.format_error_message(),
                path,
            )),
        }
    }
}

/// # Required File
///
/// The only purpose of this file provider is to throw an error if it still exists
/// when the file providers are being checked. All definitions of a required file
/// are expected to be replaced by user defined file providers.
///
/// ```yaml
/// - name: "router-config.yaml"
///   env_var: ROUTER_CONFIG
///   kind: required
///   message: "you must specify a router config file to use"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Template)]
pub struct RequiredFile {
    /// The error message to display to the user if this provider is not overwritten.
    #[template(skip)]
    pub(crate) message: String,
}

impl AsUtf8FileContent for RequiredFile {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        panic!(
            "Should not be able to get here. Required file should result in an error when checked."
        )
    }
}

impl Check for RequiredFile {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Err(checks::Errors::new(
            checks::ErrorKind::RequiredFileMissing,
            &self.message,
            path,
        ))
    }
}

/// # Resolved Values
///
/// Returns the JSON string representation of the resolved values for the test plan being run.
///
/// ```yaml
/// - name: "resolved-values.json"
///   env_var: VALUES
///   kind: resolved_values
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Template)]
pub struct ResolvedValues;

impl AsUtf8FileContent for ResolvedValues {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        let s = match ctx.values() {
            Some(values) => serde_json::to_string(&values)?,
            None => "{}".to_string(),
        };

        Ok(s)
    }
}

impl Check for ResolvedValues {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        mock_context::MockContext,
        providers::test_helpers::{assert_file_content, create_temp_dir_with_file},
        templating::ErrorKind,
    };
    use assert_fs::{
        TempDir,
        assert::PathAssert,
        fixture::{ChildPath, PathChild},
        prelude::FileWriteStr,
    };
    use indoc::indoc;
    use predicates::path;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    /// Create a RelativeFile for testing - returns the actual file in a tmp dir
    fn relative_file(path: &str) -> RelativeFile {
        RelativeFile {
            path: Field::Resolved(path.to_string()),
            src: None,
        }
    }

    /// Assert check errors
    pub(crate) fn assert_check_errors(
        c: impl Check,
        src: &Source,
        ctx: &Context,
        expected_err_kinds: &[checks::ErrorKind],
    ) {
        let res = c.try_check(&mut Vec::new(), src, ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err();
        let err_kinds: Vec<checks::ErrorKind> = err.iter().map(|e| e.kind).collect();
        assert_eq!(
            err_kinds, expected_err_kinds,
            "check the error kind is correct"
        );
    }

    /// Assert resolve and write success
    pub(crate) async fn assert_resolve_and_write_success(
        provider: FileProvider,
        target: &ChildPath,
        src: &Source,
        ctx: &mut impl ResolutionContext,
        expected_content: &str,
    ) {
        let res = provider.resolve_and_write(target, src, ctx).await;
        assert!(
            res.is_ok(),
            "expected file to resolve and write, got {res:?}"
        );

        assert_file_content(target, expected_content);
    }

    /// Assert resolve and write error
    pub(crate) async fn assert_resolve_and_write_error(
        provider: FileProvider,
        target: &ChildPath,
        src: &Source,
        ctx: &mut impl ResolutionContext,
        expected_err_str: &str,
    ) {
        let res = provider.resolve_and_write(target, src, ctx).await;
        assert!(
            res.is_err(),
            "expected file to fail to resolve and write, got {res:?}"
        );

        target.assert(path::missing());
        let err = res.unwrap_err();
        assert_eq!(err.to_string(), expected_err_str)
    }

    // Yaml snippets for all file providers
    const BUILD_ROUTER_FROM_SOURCE: &str = indoc!(
        r#"
        kind: build_router_from_source
        git_ref: "{{ git_ref }}"
        rust_version: "{{ rust_version }}"
    "#
    );
    const GITHUB_FILE: &str = indoc!(
        r#"
        kind: github_file
        org: "{{ org }}"
        repo: "{{ repo }}"
        path: "{{ path }}"
        git_ref: "{{ git_ref }}"
    "#
    );
    const GRAPHOS_CANNED_OPS: &str = indoc!(
        r#"
        kind: graphos_canned_ops
        graph_ref: "{{ graph_ref }}"
        top_n: "{{ top_n }}"
        skip_mutations: "{{ skip_mutations }}"
    "#
    );
    const GRAPHOS_CANNED_OPS_BY_ID: &str = indoc!(
        r#"
        kind: graphos_canned_ops_by_id
        graph_ref: "{{ graph_ref }}"
        operation_ids:
          - "{{ op_1 }}"
          - "{{ op_2 }}"
    "#
    );
    const GRAPHOS_SUBGRAPH_DOCKER_COMPOSE: &str = indoc!(
        r#"
        kind: graphos_subgraph_docker_compose
        graph_ref: "{{ graph_ref }}"
        image: "{{ image }}"
        replicas: "{{ replicas }}"
        resource_limits:
          cpus: "{{ resource_limits_cpus }}"
          memory: "{{ resource_limits_memory }}"
        resource_reservations:
          cpus: "{{ resource_reservations_cpus }}"
          memory: "{{ resource_reservations_memory }}"
        mem_swappiness: "{{ mem_swappiness }}"
        loadbalancer:
          resource_limits:
            cpus: "{{ loadbalancer_resource_limits_cpus }}"
            memory: "{{ loadbalancer_resource_limits_memory }}"
          resource_reservations:
            cpus: "{{ loadbalancer_resource_reservations_cpus }}"
            memory: "{{ loadbalancer_resource_reservations_memory }}"
          mem_swappiness: "{{ loadbalancer_mem_swappiness }}"
    "#
    );
    const GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES: &str = indoc!(
        r#"
        kind: graphos_subgraph_router_url_overrides
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const GRAPHOS_SUBGRAPHS: &str = indoc!(
        r#"
        kind: graphos_subgraphs
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const GRAPHOS_SUPERGRAPH: &str = indoc!(
        r#"
        kind: graphos_supergraph
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const FROM_COMMAND: &str = indoc!(
        r#"
        kind: from_command
        command:
          name: test.sh
          kind: relative_path
          path: "{{ test_script }}"
          args:
            - "{{ test_arg }}"
        env_vars:
          TEST_VAR: "{{ test_var }}"
        file_providers:
          - name: test-file.txt
            env_var: TEST_FILE
            kind: relative_path
            path: "{{ test_path }}"
    "#
    );
    const INLINE: &str = indoc!(
        r#"
        kind: inline
        content: |
            some content
    "#
    );
    const OFFLINE_GRAPHOS_LICENSE: &str = indoc!(
        r#"
        kind: offline_graphos_license
        graph_id: "{{ graph_id }}"
    "#
    );
    const RELATIVE_PATH: &str = indoc!(
        r#"
        kind: relative_path
        path: "{{ path }}"
    "#
    );
    const REQUIRED_FILE: &str = indoc!(
        r#"
        kind: required
        message: this file is required
    "#
    );
    const RESOLVED_VALUES: &str = indoc!(
        r#"
        kind: resolved_values
    "#
    );
    const ROUTER_DOWNLOAD_SCRIPT: &str = indoc!(
        r#"
        kind: router_download_script
        version: "{{ version }}"
    "#
    );
    const MERGE_YAML: &str = indoc!(
        r#"
        kind: merge_yaml
        base:
          kind: inline
          content: |
            key: value
        overrides:
          kind: inline
          content: |
            new_key: new_value
    "#
    );
    const MERGE_YAML_ARRAY: &str = indoc!(
        r#"
        kind: merge_yaml
        base:
          kind: inline
          content: |
            key: value
        overrides:
          - kind: inline
            content: |
              new_key: new_value
          - kind: inline
            content: |
              another_new_key: another_new_value
    "#
    );

    #[test_case(BUILD_ROUTER_FROM_SOURCE, &["git_ref", "rust_version"]; "build_router_from_source")]
    #[test_case(GITHUB_FILE, &["org", "repo", "path", "git_ref"]; "github_file")]
    #[test_case(GRAPHOS_CANNED_OPS, &["graph_ref", "top_n", "skip_mutations"]; "graphos_canned_ops")]
    #[test_case(GRAPHOS_CANNED_OPS_BY_ID, &["graph_ref", "op_1", "op_2"]; "graphos_canned_ops_by_id")]
    #[test_case(GRAPHOS_SUBGRAPH_DOCKER_COMPOSE, &[
        "graph_ref", "image", "replicas", "resource_limits_cpus", "resource_limits_memory", 
        "resource_reservations_cpus", "resource_reservations_memory", "mem_swappiness", 
        "loadbalancer_resource_limits_cpus", "loadbalancer_resource_limits_memory", 
        "loadbalancer_resource_reservations_cpus", "loadbalancer_resource_reservations_memory", 
        "loadbalancer_mem_swappiness"
    ]; "graphos_subgraph_docker_compose")]
    #[test_case(GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES, &["graph_ref"]; "graphos_subgraph_router_url_overrides")]
    #[test_case(GRAPHOS_SUBGRAPHS, &["graph_ref"]; "graphos_subgraphs")]
    #[test_case(GRAPHOS_SUPERGRAPH, &["graph_ref"]; "graphos_supergraph")]
    #[test_case(FROM_COMMAND, &["test_script", "test_arg", "test_var", "test_path"]; "from_command")]
    #[test_case(INLINE, &[]; "inline")]
    #[test_case(OFFLINE_GRAPHOS_LICENSE, &["graph_id"]; "offline_graphos_license")]
    #[test_case(RELATIVE_PATH, &["path"]; "relative_path")]
    #[test_case(REQUIRED_FILE, &[]; "required")]
    #[test_case(RESOLVED_VALUES, &[]; "resolved_values")]
    #[test_case(ROUTER_DOWNLOAD_SCRIPT, &["version"]; "router_download_script")]
    #[test_case(MERGE_YAML, &[]; "merge_yaml")]
    #[test_case(MERGE_YAML_ARRAY, &[]; "merge_yaml_array")]
    #[test]
    fn all_fields_templated(content: &str, expected_values: &[&str]) {
        let config: FileProvider = serde_yaml::from_str(content).unwrap();

        let res = config.required_values();
        assert_eq!(res, expected_values, "expected values to match")
    }

    #[test_case(Field::Pending("foo".to_string()), true; "field is pending")]
    #[test_case(Field::Resolved("foo".to_string()), false; "field is resolved")]
    #[test]
    fn named_file_provider_has_pending_fields(f: Field<String>, expected: bool) {
        let nfp = NamedFileProvider {
            name: "inline.txt".to_string(),
            env_var: "INLINE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
        };

        let res = nfp.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(Field::Pending("foo".to_string()), &["foo"]; "field is required")]
    #[test_case(Field::Resolved("foo".to_string()), &[]; "no fields required")]
    #[test]
    fn named_file_provider_required_values(f: Field<String>, expected: &[&str]) {
        let nfp = NamedFileProvider {
            name: "inline.txt".to_string(),
            env_var: "INLINE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
        };

        let res = nfp.required_values();
        assert_eq!(
            res, expected,
            "tests that required_values has expected value"
        )
    }

    #[test]
    fn named_file_provider_try_template_succeeds() {
        let mut nfp = NamedFileProvider {
            name: "inline.txt".to_string(),
            env_var: "INLINE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let mut values: HashMap<String, Scalar> = HashMap::new();
        values.insert("path".to_string(), Scalar::String("path".to_string()));

        let res = nfp.try_template(&mut Vec::new(), &values);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test_case(vec![], "file_providers.RELATIVE.path"; "no path entries")]
    #[test_case(vec!["path"], "file_providers.RELATIVE.path"; "single path entry")]
    #[test_case(vec!["two", "entries"], "two.file_providers.RELATIVE.path"; "two path entries")]
    #[test_case(vec!["multiple", "path", "entries"], "multiple.path.file_providers.RELATIVE.path"; "multiple path entries")]
    #[test]
    fn named_file_provider_try_template_unknown_value_error(path: Vec<&str>, expected_path: &str) {
        let mut nfp = NamedFileProvider {
            name: "relative.txt".to_string(),
            env_var: "RELATIVE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let mut values: HashMap<String, Scalar> = HashMap::new();
        values.insert("unused".to_string(), Scalar::String("unused".to_string()));
        let mut path: Vec<String> = path.into_iter().map(|s| s.to_string()).collect();

        let res = nfp.try_template(&mut path, &values);
        assert!(res.is_err(), "expected templating to error, got {res:?}");

        let errors = res.unwrap_err();
        let error = errors.unwrap_single();
        let error_kind = error.kind;
        let error_path = error.path;
        assert_eq!(
            error_kind,
            ErrorKind::UnknownValue,
            "expected ErrorKind to match"
        );
        assert_eq!(error_path, expected_path, "expected path to match")
    }

    #[test]
    fn inline_file_check_success() {
        let inline = InlineFile {
            content: "some content".to_string(),
        };

        let src = Source::Local {
            abs_path: "/".into(),
        };
        let ctx = Context::new();

        let res = inline.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_file_check_local_success() {
        let file_name = "file.txt";
        let (_temp, file) = create_temp_dir_with_file(file_name, "some content");

        let relative_file = relative_file(file_name);

        let ctx = Context::new();
        let src = Source::Local {
            abs_path: ctx.canonicalize_path(&file).unwrap(),
        };

        let res = relative_file.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_file_check_github_success() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let relative_file = relative_file("file.txt");

        let mut ctx = Context::new();
        ctx.with_github_config("dummy_token");
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        let res = relative_file.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_file_check_does_not_exist() {
        let (_temp, file) = create_temp_dir_with_file("file.txt", "some content");

        let relative_file = relative_file("does-not-exist.txt");

        let ctx = Context::new();
        let src = Source::Local {
            abs_path: ctx.canonicalize_path(&file).unwrap(),
        };

        assert_check_errors(
            relative_file,
            &src,
            &ctx,
            &[checks::ErrorKind::FileNotFound],
        );
    }

    #[test]
    fn relative_file_check_is_directory() {
        let (temp, _file) = create_temp_dir_with_file("dir/file.txt", "some content");

        let relative_file = relative_file("dir");

        let ctx = Context::new();
        let src = Source::Local {
            abs_path: ctx
                .canonicalize_path(format!("{}/dir", temp.path().to_string_lossy()))
                .unwrap(),
        };

        assert_check_errors(
            relative_file,
            &src,
            &ctx,
            &[checks::ErrorKind::IsADirectory],
        );
    }

    #[test]
    fn relative_file_check_missing_github_api_token() {
        let relative_file = relative_file("file.txt");

        // There is no github client added to this context so this fails
        let ctx = Context::new();
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        assert_check_errors(
            relative_file,
            &src,
            &ctx,
            &[checks::ErrorKind::MissingGithubApiKey],
        );
    }

    #[test]
    fn required_file_check_is_missing() {
        let required_file = RequiredFile {
            message: "need to override".to_string(),
        };

        let ctx = Context::new();
        let src = Source::Local {
            abs_path: "/".into(),
        };

        // Not reusing assert_check_error so the message can also be checked
        let res = required_file.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(
            err.kind,
            checks::ErrorKind::RequiredFileMissing,
            "check the error kind is correct"
        );
        assert_eq!(
            required_file.message, err.message,
            "check the error message matches the provider message"
        );
    }

    #[test]
    fn resolved_values_check_success() {
        let resolved_values = ResolvedValues {};

        let src = Source::Local {
            abs_path: "/".into(),
        };
        let ctx = Context::new();

        let res = resolved_values.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[tokio::test]
    async fn inline_file_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("inline.txt");

        let mut ctx = Context::new();
        let src = Source::Local {
            abs_path: PathBuf::new(),
        };

        let expected_content = "some content";
        let inline = FileProvider::Inline(InlineFile {
            content: expected_content.to_string(),
        });

        assert_resolve_and_write_success(inline, &target, &src, &mut ctx, expected_content).await;
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_local_success() {
        let expected_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", expected_content);
        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file
            .write_str("empty test plan")
            .expect("unable to write test plan");

        let target = temp.child("output/relative.txt");

        let mut ctx = Context::new();
        // Relative File paths are resolved relative to the test plan file's location.
        // We have created an empty test plan file so we can canonicalize its path (it must exist for this to work)
        // This allows us to read the relative file from the correct path
        let src = Source::Local {
            abs_path: ctx.canonicalize_path(&test_plan_file).unwrap(),
        };

        let relative = FileProvider::RelativePath(relative_file("file.txt"));

        assert_resolve_and_write_success(relative, &target, &src, &mut ctx, expected_content).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_github_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output/relative.txt");

        let content = "some content";

        let mut ctx = MockContext::with_github_client(content);
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        // The file does not exist but this does not matter since we mock a GitHub response
        let relative = FileProvider::RelativePath(relative_file("file.txt"));

        assert_resolve_and_write_success(relative, &target, &src, &mut ctx, content).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_local_does_not_exist() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output/relative.txt");

        let mut ctx = Context::new();
        let src = Source::Local {
            abs_path: ctx.canonicalize_path(&temp).unwrap(),
        };

        let expected_err = "No such file or directory (os error 2)";

        // The file.txt file has not been created in temp
        let relative = FileProvider::RelativePath(relative_file("file.txt"));

        assert_resolve_and_write_error(relative, &target, &src, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_local_is_not_text() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("file.txt");

        let crate_root_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let file_to_read = crate_root_path.join("resources/frog-no.gif");

        let mut ctx = Context::new();
        let src = Source::Local {
            abs_path: ctx.canonicalize_path(&file_to_read).unwrap(),
        };

        let expected_err = "stream did not contain valid UTF-8";

        // The frog-no.gif cannot be read since the file to read is not utf-8
        let relative = FileProvider::RelativePath(relative_file("frog-no.gif"));

        assert_resolve_and_write_error(relative, &target, &src, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_github_no_github_client() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output/relative.txt");

        let mut ctx = Context::new();
        let src = Source::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        let expected_err = "no GitHub client available";

        // The file does not exist but this does not matter since we mock a GitHub response
        let relative = FileProvider::RelativePath(relative_file("file.txt"));

        assert_resolve_and_write_error(relative, &target, &src, &mut ctx, expected_err).await
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Required file should result in an error when checked."
    )]
    async fn required_file_resolve_and_write_panics() {
        let mut ctx = Context::new();
        let src = Source::Local {
            abs_path: PathBuf::new(),
        };

        let required = FileProvider::Required(RequiredFile {
            message: "required file must be defined".to_string(),
        });

        let _res = required
            .resolve_and_write(Path::new("required.txt"), &src, &mut ctx)
            .await;
    }

    #[tokio::test]
    async fn resolved_values_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("values.json");

        let expected_content = r#"{"foo":"bar"}"#;

        let mut ctx = Context::new();
        let values: HashMap<String, Scalar> =
            [("foo".to_string(), "bar".into())].into_iter().collect();
        ctx.set_values(&values);

        let src = Source::Local {
            abs_path: PathBuf::new(),
        };

        let resolved_values = FileProvider::ResolvedValues(ResolvedValues);

        assert_resolve_and_write_success(resolved_values, &target, &src, &mut ctx, expected_content)
            .await
    }
}

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
    message: String,
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
        templating::ErrorKind,
        txtar_context::{MockHttpClient, TxtarContext},
    };
    use assert_fs::{TempDir, assert::PathAssert, prelude::PathChild};
    use indoc::indoc;
    use simple_test_case::{dir_cases, test_case};
    use simple_txtar::Archive;
    use std::path::PathBuf;

    /// Load a txtar [Archive] from the given file content and print the top level comment if there
    /// is one before returning it.
    fn load_archive(content: &str) -> Archive {
        let arr = Archive::from(content);
        let comment = arr.comment();
        if !comment.is_empty() {
            println!("{}", comment.trim());
        }

        arr
    }

    /// Read the requested file from the archive, panicking if it is missing
    fn get_file<'a>(arr: &'a Archive, fname: &str) -> &'a str {
        match arr.get(fname) {
            Some(f) => f.content.trim(),
            None => {
                panic!("required txtar file section {fname:?} was missing");
            }
        }
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/check-errors")]
    #[test]
    fn check_errors(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/check-errors")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let res = provider.try_check(&mut Vec::new(), &src, &ctx);

        assert!(res.is_err(), "expected check errors");
        let errs = res.unwrap_err();

        // Check errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(&concatenated_errs, expected, "wrong check errors: {errs:?}");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/expected-file-success")]
    #[tokio::test]
    async fn expected_file_success(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let temp = TempDir::new().unwrap();
        let file = temp.child("provider.txt");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/expected-file-success")
            .canonicalize()
            .unwrap();
        let mut ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));

        let res = provider.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected successful check but got: {res:?}");

        let res = provider.resolve_and_write(&file, &src, &mut ctx).await;
        assert!(res.is_ok(), "{res:?}");

        let expected = get_file(&arr, "expected-file-content");
        file.assert(expected);
    }

    #[dir_cases(
        "crates/rtf-config/resources/provider-tests/file/expected-file-success-mock-context"
    )]
    #[tokio::test]
    async fn expected_file_success_mock_context(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let temp = TempDir::new().unwrap();
        let file = temp.child("provider.txt");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/expected-file-success-mock-context")
            .canonicalize()
            .unwrap();
        let mut ctx = TxtarContext::with_http(arr.clone(), MockHttpClient::from_archive(&arr));
        let src = Source::local(dir.join("example.yaml"));

        let res = provider.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected successful check but got: {res:?}");

        let res = provider.resolve_and_write(&file, &src, &mut ctx).await;
        assert!(res.is_ok(), "{res:?}");

        let expected = get_file(&arr, "expected-file-content");
        file.assert(expected);
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/resolution-errors")]
    #[tokio::test]
    async fn resolution_errors(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "resolution-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/resolution-errors")
            .canonicalize()
            .unwrap();
        let mut ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let _ = provider.try_check(&mut Vec::new(), &src, &ctx);
        let res = provider
            .resolve_and_write("expected-file-content", &src, &mut ctx)
            .await;

        assert!(res.is_err(), "expected resolution failures, got {res:?}");
        let err = res.unwrap_err();
        assert_eq!(&err.to_string(), expected, "wrong resolution errors");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/resolution-errors-mock-context")]
    #[tokio::test]
    async fn resolution_errors_mock_context(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "resolution-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/resolution-errors-mock-context")
            .canonicalize()
            .unwrap();
        let mut ctx = TxtarContext::with_http(arr.clone(), MockHttpClient::from_archive(&arr));
        let src = Source::local(dir.join("example.yaml"));
        let _ = provider.try_check(&mut Vec::new(), &src, &ctx);
        let res = provider
            .resolve_and_write("expected-file-content", &src, &mut ctx)
            .await;

        assert!(res.is_err(), "expected resolution failures, got {res:?}");
        let err = res.unwrap_err();
        assert_eq!(&err.to_string(), expected, "wrong resolution errors");
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Required file should result in an error when checked."
    )]
    async fn required_file_provider_try_into_file_content_panics() {
        let required_file = RequiredFile {
            message: "required file must be defined".to_string(),
        };
        let ctx = Context::new();

        // Calling try_into_file_content should panic here
        _ = required_file
            .try_get_file_content(
                &Source::Local {
                    abs_path: PathBuf::new(),
                },
                &ctx,
            )
            .await;
    }

    #[tokio::test]
    async fn resolved_values_file_provider_returns_stored_values() {
        let mut ctx = Context::new();
        let values: HashMap<String, Scalar> =
            [("foo".to_string(), "bar".into())].into_iter().collect();
        ctx.set_values(&values);

        let s = ResolvedValues
            .try_get_file_content(
                &Source::Local {
                    abs_path: PathBuf::new(),
                },
                &ctx,
            )
            .await
            .expect("resolution to succeed");

        assert_eq!(s, r#"{"foo":"bar"}"#);
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
        assert!(
            res == expected,
            "expected has pending fields to be {expected:?}, got {res:?}"
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
        assert!(
            res == expected,
            "expected required values to be {expected:?}, got {res:?}"
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
}

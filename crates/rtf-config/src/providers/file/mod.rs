//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    checks::{self, Check},
    context::{PathKind, ResolutionContext},
    enum_impl_check, enum_impl_template, impl_template,
    providers::{self, Result},
    templating::{self, Field, Scalar, Template},
};
use rtf_core::github::Client;
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

/// A file provider is something that can obtain or synthesise utf-8 file content based on a user
/// provided specification.
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

impl<T> ResolveAndWrite for T
where
    T: AsUtf8FileContent,
{
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<Vec<(PathBuf, String)>> {
        Ok(vec![(
            target.as_ref().to_path_buf(),
            self.try_get_file_content(src, ctx).await?,
        )])
    }
}

/// Logic for running a file provider and writing its output to the target [Path].
///
/// Most [FileProvider] implementations can safely ignore providing a custom implementation for
/// this trait if all they need to do is write out a single file, and instead just implement
/// [AsUtf8FileContent] which will give a default implementation of this trait.
/// If however you need to write out multiple files or run some additional logic after writing out
/// a file (such as making it executable) then you should implement this trait directly.
#[allow(async_fn_in_trait)]
pub(crate) trait ResolveAndWrite: Check + Serialize + DeserializeOwned + fmt::Debug {
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<Vec<(PathBuf, String)>>;

    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
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

/// Helper macro for stamping out implementations of the ResolveAndWrite trait on an enum where
/// each variant is a wrapper around a type that already implements the trait.
#[macro_export]
macro_rules! enum_impl_resolve_and_write {
    ($enum:ident => $($variant:ident),+) => {
        impl ResolveAndWrite for $enum {
            async fn try_get_all_file_contents(
                &self,
                target: impl AsRef<Path>,
                src: &Source,
                ctx: &impl ResolutionContext,
            ) -> $crate::providers::Result<Vec<(PathBuf, String)>> {
                match self {
                    $(Self::$variant(inner) => inner.try_get_all_file_contents(target, src, ctx).await,)+
                }
            }

            async fn resolve_and_write(
                &self,
                target: impl AsRef<Path>,
                src: &Source,
                ctx: &impl ResolutionContext,
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

/// # File Provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileProvider {
    BuildRouterFromSource(apollo::BuildRouterFromSource),
    GithubFile(github::GithubFile),
    GraphosCannedOps(apollo::GraphosCannedOps),
    GraphosSubgraphDockerCompose(apollo::GraphosSubgraphDockerCompose),
    GraphosSubgraphRouterUrlOverrides(apollo::GraphosSubgraphRouterUrlOverrides),
    GraphosSubgraphs(apollo::GraphosSubgraphs),
    GraphosSupergraph(apollo::GraphosSupergraph),
    Inline(InlineFile),
    OfflineGraphosLicense(apollo::OfflineGraphosLicense),
    RelativePath(RelativeFile),
    Required(RequiredFile),
    ResolvedValues(ResolvedValues),
    RouterDownloadScript(apollo::RouterDownloadScript),
    MergeYaml(utility::MergeYaml),
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
        enum_impl_template!(FileProvider => $($variant),+);
        enum_impl_resolve_and_write!(FileProvider => $($variant),+);
    };
}

enum_impl_file_provider!(
    BuildRouterFromSource,
    GithubFile,
    GraphosCannedOps,
    GraphosSubgraphDockerCompose,
    GraphosSubgraphRouterUrlOverrides,
    GraphosSubgraphs,
    GraphosSupergraph,
    Inline,
    OfflineGraphosLicense,
    RelativePath,
    Required,
    ResolvedValues,
    RouterDownloadScript,
    MergeYaml,
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct InlineFile {
    /// The text to write out as the contents of the generated file.
    pub(crate) content: String,
}

impl AsUtf8FileContent for InlineFile {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> Result<String> {
        Ok(self.content.clone())
    }
}

impl_template!(InlineFile => []);

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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct RelativeFile {
    /// The relative path from the containing config file to the target file.
    pub(crate) path: Field<String>,

    /// Set during TestPlan parsing as part of overrides. This should only ever be `Some` if this
    /// provider was defined as part of an `overrides` section in the test plan.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) src: Option<Source>,
}

impl RelativeFile {
    fn format_error_message(&self) -> String {
        format!("provided path was {:?}", self.path)
    }
}

// in try_template we don't want to include a trailing ".path" in the resolution path we report to
// users in error messages so we had implement Template for this one.
impl Template for RelativeFile {
    fn has_pending_fields(&self) -> bool {
        self.path.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        self.path.required_values()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        self.path.try_template(path, values)
    }
}

impl AsUtf8FileContent for RelativeFile {
    async fn try_get_file_content(
        &self,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct RequiredFile {
    /// The error message to display to the user if this provider is not overwritten.
    message: String,
}

impl AsUtf8FileContent for RequiredFile {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> Result<String> {
        panic!(
            "Should not be able to get here. Required file should result in an error when checked."
        )
    }
}

impl_template!(RequiredFile => []);

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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct ResolvedValues;

impl AsUtf8FileContent for ResolvedValues {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
        let s = match ctx.values() {
            Some(values) => serde_json::to_string(&values)?,
            None => "{}".to_string(),
        };

        Ok(s)
    }
}

impl_template!(ResolvedValues => []);

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
        txtar_context::{MockHttpClient, TxtarContext},
    };
    use simple_test_case::dir_cases;
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

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/expected-file-success")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));

        let res = provider.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected successful check but got: {res:?}");

        // We resolve the file provider under a target of "expected-file-content".
        // For providers returning a single file only, this is the name of the txtar section that
        // they need to include. For providers that return multiple files the sections should be
        // named "expected-file-content/$name_of_file".
        let contents = provider
            .try_get_all_file_contents("expected-file-content", &src, &ctx)
            .await
            .unwrap();

        assert!(!contents.is_empty(), "no file contents returned");

        for (path, content) in contents.into_iter() {
            let key = path.display().to_string();
            let expected = get_file(&arr, &key);
            assert_eq!(content, expected, "wrong file content");
        }
    }

    #[dir_cases(
        "crates/rtf-config/resources/provider-tests/file/expected-file-success-mock-context"
    )]
    #[tokio::test]
    async fn expected_file_success_mock_context(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/expected-file-success-mock-context")
            .canonicalize()
            .unwrap();
        let ctx = TxtarContext::with_http(arr.clone(), MockHttpClient::from_archive(&arr));
        let src = Source::local(dir.join("example.yaml"));

        let res = provider.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected successful check but got: {res:?}");

        // We resolve the file provider under a target of "expected-file-content".
        // For providers returning a single file only, this is the name of the txtar section that
        // they need to include. For providers that return multiple files the sections should be
        // named "expected-file-content/$name_of_file".
        let contents = provider
            .try_get_all_file_contents("expected-file-content", &src, &ctx)
            .await
            .unwrap();

        assert!(!contents.is_empty(), "no file contents returned");

        for (path, content) in contents.into_iter() {
            let key = path.display().to_string();
            let expected = get_file(&arr, &key);
            assert_eq!(content, expected, "wrong file content");
        }
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<FileProvider> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
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
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let _ = provider.try_check(&mut Vec::new(), &src, &ctx);
        let res = provider
            .try_get_all_file_contents("expected-file-content", &src, &ctx)
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
        let ctx = TxtarContext::with_http(arr.clone(), MockHttpClient::from_archive(&arr));
        let src = Source::local(dir.join("example.yaml"));
        let _ = provider.try_check(&mut Vec::new(), &src, &ctx);
        let res = provider
            .try_get_all_file_contents("expected-file-content", &src, &ctx)
            .await;

        assert!(res.is_err(), "expected resolution failures, got {res:?}");
        let err = res.unwrap_err();
        assert_eq!(&err.to_string(), expected, "wrong resolution errors");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/template-errors")]
    #[test]
    fn template_errors(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let expected = get_file(&arr, "template-errors");

        let mut provider: FileProvider = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();

        assert!(provider.has_pending_fields(), "fields should be pending");

        let res = provider.try_template(&mut Vec::new(), &values);

        assert!(
            provider.has_pending_fields(),
            "fields should still be pending"
        );

        let errs = res.unwrap_err().into_vec();
        let str_errs: Vec<String> = errs.iter().map(|e| format!("{:?}", e.kind)).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/template-success")]
    #[test]
    fn template_success(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let raw_expected = get_file(&arr, "after-templating");

        let mut provider: FileProvider = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();
        let expected: FileProvider = serde_yaml::from_str(raw_expected).unwrap();

        assert!(provider.has_pending_fields(), "fields should be pending");

        let res = provider.try_template(&mut Vec::new(), &values);

        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(!provider.has_pending_fields(), "fields should be resolved");
        assert_eq!(provider, expected);
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
}

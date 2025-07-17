//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    checks::{self, Check},
    context::{PathKind, ResolutionContext},
    enum_impl_check, enum_impl_template, impl_template,
    providers::{self, Error, Result},
    templating::{self, Field, Scalar, Template},
};
use indoc::indoc;
use reqwest::StatusCode;
use rtf_core::{HttpClient, github::Client};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fmt, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

pub mod apollo;
pub mod github;

/// The source of how a particular config file was obtained.
///
/// In its simplest form this is a local file path to the directory containing the config file, but
/// this may also include things like pulling the file over the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Source {
    /// The config file was read from disk
    Local {
        /// The absolute path to the config file
        abs_path: PathBuf,
    },

    /// The config file was downloaded from GitHub
    Github {
        /// The GitHub org
        org: String,
        /// The GitHub repository
        repo: String,
        /// The path to the file within the GitHub repository
        path: PathBuf,
        /// An optional ref of the repo to use (the default branch is used when None)
        git_ref: Option<String>,
    },
}

impl Source {
    pub fn local(abs_path: impl Into<PathBuf>) -> Self {
        Self::Local {
            abs_path: abs_path.into(),
        }
    }

    pub async fn try_get_file_content(&self, ctx: &impl ResolutionContext) -> Result<String> {
        match self {
            Self::Local { abs_path } => Ok(ctx.read_path_to_string(abs_path)?),
            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => {
                let client = ctx
                    .github_client()
                    .ok_or(providers::Error::Github(rtf_core::github::Error::NoClient))?;

                Ok(client
                    .string_file_content(org, repo, &path.display().to_string(), git_ref.as_ref())
                    .await?)
            }
        }
    }
}

impl Default for Source {
    fn default() -> Self {
        Self::Local {
            abs_path: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RawSource {
    Local {
        relative_path: PathBuf,
    },
    Github {
        org: String,
        repo: String,
        path: String,
        #[serde(default)]
        git_ref: Option<String>,
    },
}

impl RawSource {
    pub fn try_into_source(
        self,
        tp_source: &Source,
        ctx: &impl ResolutionContext,
    ) -> io::Result<Source> {
        match self {
            Self::Local { relative_path } => match tp_source {
                Source::Local { abs_path } => {
                    let p = match abs_path.parent() {
                        Some(parent) => parent.join(relative_path),
                        None => relative_path,
                    };

                    Ok(Source::Local {
                        abs_path: ctx.canonicalize_path(p)?,
                    })
                }

                Source::Github {
                    org,
                    repo,
                    path,
                    git_ref,
                } => {
                    let path = match path.parent() {
                        Some(parent) => parent.join(relative_path),
                        None => relative_path,
                    };

                    Ok(Source::Github {
                        org: org.clone(),
                        repo: repo.clone(),
                        path,
                        git_ref: git_ref.clone(),
                    })
                }
            },

            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => Ok(Source::Github {
                org,
                repo,
                path: PathBuf::from(path),
                git_ref,
            }),
        }
    }
}

/// A file provider is something that can obtain or synthesise utf-8 file content based on a user
/// provided specification.
///
/// This trait is deliberately pub(crate) rather than pub so that the validation and resolution
/// logic is only exposed through the public API as part of the methods on the config file structs.
#[allow(async_fn_in_trait)]
pub(crate) trait AsUtf8FileContent: Check + DeserializeOwned + fmt::Debug {
    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_get_file_content(
        &self,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String>;
}

/// Helper macro for stamping out implementations of the [AsUtf8FileContent] trait on an enum where
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
pub(crate) trait ResolveAndWrite: Check + DeserializeOwned + fmt::Debug {
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

/// Helper macro for stamping out implementations of the [ResolveAndWrite] trait on an enum where
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct NamedFileProvider {
    pub name: String,
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileProvider {
    BuildRouterFromSource(BuildRouterFromSource),
    GithubFile(github::GithubFile),
    GraphosCannedOps(apollo::GraphosCannedOps),
    GraphosSubgraphs(apollo::GraphosSubgraphs),
    GraphosSupergraph(apollo::GraphosSupergraph),
    Inline(InlineFile),
    OfflineGraphosLicense(apollo::OfflineGraphosLicense),
    RelativePath(RelativeFile),
    Required(RequiredFile),
    ResolvedValues(ResolvedValues),
    RouterDownloadScript(RouterDownloadScript),
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
    GraphosSubgraphs,
    GraphosSupergraph,
    Inline,
    OfflineGraphosLicense,
    RelativePath,
    Required,
    ResolvedValues,
    RouterDownloadScript,
);

/// The simplest form of file provider: the user specifies the contents of the file inline within
/// their config file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct InlineFile {
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

/// The user specifies a path to a local file relative to the config
/// file containing this provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RelativeFile {
    pub(crate) path: Field<String>,
}

impl RelativeFile {
    fn format_error_message(&self) -> String {
        format!("Provided path was {:?}", self.path)
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

/// The only purpose of this file provider is to throw an error if it still exists
/// when the file providers are being checked. All definitions of a required file
/// are expected to be replaced by user defined file providers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RequiredFile {
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

/// Returns the JSON string representation of the resolved values for the test plan being run.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RouterDownloadScript {
    pub(crate) version: Field<String>,
}

impl_template!(RouterDownloadScript => [version]);

impl AsUtf8FileContent for RouterDownloadScript {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
        let version = self.version.as_resolved();
        let url = format!("https://router.apollo.dev/download/nix/{version}");
        let client = ctx.http_client().expect("to have an http client");
        let response = client.get(&url).await?;
        if response.status == StatusCode::NOT_FOUND {
            return Err(Error::UnknownRouterVersion(version.to_string()));
        }
        let script = std::str::from_utf8(&response.body)
            .map_err(|_| Error::Utf8DecodingError)?
            .to_string();

        Ok(script)
    }
}

impl Check for RouterDownloadScript {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        if ctx.http_client().is_none() {
            return Err(checks::Errors::new(
                checks::ErrorKind::HttpClientNotFound,
                "",
                path,
            ));
        }

        Ok(())
    }
}

/// A file provider used for building the Router from source at a specific git commit
/// or reference.
///
/// - `commit_ref`: A git reference that can be passed to `git checkout`. This may be
///   a full or partial commit hash, branch name, or tag. Defaults to `"main"` if unset.
/// - `rust_version`: A Rust version string that can be passed to `rustup run {rust_version}`,
///   such as `"1.78.0"`, `"beta"`, or `"nightly"`. Defaults to `"stable"` if unset.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BuildRouterFromSource {
    pub(crate) commit_ref: Option<Field<String>>,
    pub(crate) rust_version: Option<Field<String>>,
}

impl Template for BuildRouterFromSource {
    fn has_pending_fields(&self) -> bool {
        [&self.commit_ref, &self.rust_version].iter().any(|field| {
            field
                .as_ref()
                .map(|f| f.has_pending_fields())
                .unwrap_or(false)
        })
    }

    fn required_values(&self) -> Vec<String> {
        [&self.commit_ref, &self.rust_version]
            .iter()
            .flat_map(|field| {
                field
                    .as_ref()
                    .map(|f| f.required_values())
                    .unwrap_or_default()
            })
            .collect()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        if let Some(field) = self.commit_ref.as_mut() {
            errs.append(field.try_template(path, values));
        }

        if let Some(field) = self.rust_version.as_mut() {
            errs.append(field.try_template(path, values));
        }

        errs.into_result(())
    }
}

impl AsUtf8FileContent for BuildRouterFromSource {
    async fn try_get_file_content(
        &self,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> Result<String> {
        let commit_ref = match &self.commit_ref {
            Some(hash) => hash.as_resolved(),
            None => "main",
        };

        let rust_version = match &self.rust_version {
            Some(rust_version) => rust_version.as_resolved(),
            None => "stable",
        };

        let install_script = format!(
            indoc!(
                r#"mkdir router-source && \
                cd router-source && \
                git clone https://github.com/apollographql/router.git && \
                cd router && \
                git checkout {} && \
                rustup run {} cargo build --release && \
                cp ${{CARGO_TARGET_DIR}}/release/router ~/.cargo/bin/"#
            ),
            commit_ref, rust_version
        );

        Ok(install_script)
    }
}

impl Check for BuildRouterFromSource {
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
    use crate::context::{Context, NullClient};
    use bytes::Bytes;
    use rtf_core::HttpResponse;
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

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/valid")]
    #[tokio::test]
    async fn valid_providers(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/valid")
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

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<FileProvider> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/check-failures")]
    #[test]
    fn check_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/check-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let res = provider.try_check(&mut Vec::new(), &src, &ctx);

        assert!(res.is_err(), "expected check failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/valid-templates")]
    #[test]
    fn valid_templated_providers(_path: &str, content: &str) {
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

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/invalid-templates")]
    #[test]
    fn invalid_templated_providers(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let expected = get_file(&arr, "templating-errors");

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

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/resolution-failures")]
    #[tokio::test]
    async fn resolution_errors(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "resolution-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/resolution-failures")
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

    #[derive(Clone)]
    struct MockHttpClient;

    impl HttpClient for MockHttpClient {
        async fn get(&self, url: &str) -> anyhow::Result<HttpResponse, reqwest::Error> {
            if url.ends_with("v1.59.1") {
                Ok(HttpResponse {
                    status: StatusCode::OK,
                    body: Bytes::from_static(b"mock router download script"),
                })
            } else if url.ends_with("v12.25.85") {
                Ok(HttpResponse {
                    status: StatusCode::NOT_FOUND,
                    body: Bytes::new(),
                })
            } else {
                panic!("MockHttpClient is not configured to for url {url}")
            }
        }
    }

    #[derive(Clone)]
    struct MockContext {
        http: MockHttpClient,
    }

    impl MockContext {
        pub fn new() -> Self {
            Self {
                http: MockHttpClient,
            }
        }
    }

    impl ResolutionContext for MockContext {
        type PlatformClient = NullClient;
        type GithubClient = NullClient;
        type HttpClient = MockHttpClient;

        fn platform_client(&self) -> Option<&Self::PlatformClient> {
            None
        }

        fn http_client(&self) -> Option<&Self::HttpClient> {
            Some(&self.http)
        }

        fn canonicalize_path(&self, _path: impl AsRef<Path>) -> io::Result<PathBuf> {
            unimplemented!()
        }

        fn path_kind(&self, _path: impl AsRef<Path>) -> PathKind {
            unimplemented!()
        }

        fn read_path_to_string(&self, _path: impl AsRef<Path>) -> io::Result<String> {
            unimplemented!()
        }

        fn write(&self, _path: impl AsRef<Path>, _content: impl AsRef<[u8]>) -> io::Result<()> {
            unimplemented!()
        }

        fn run_command_blocking<'a>(
            &self,
            _prog: &str,
            _args: impl IntoIterator<Item = &'a str>,
            _env_vars: &HashMap<String, String>,
        ) -> io::Result<()> {
            unimplemented!()
        }

        fn set_current_dir(&mut self, _path: impl AsRef<Path>) -> io::Result<()> {
            unimplemented!()
        }

        fn remove_file(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            unimplemented!()
        }

        fn create_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            unimplemented!()
        }
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/resolution-failures-mock-context")]
    #[tokio::test]
    async fn resolution_errors_mock_context(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "resolution-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/resolution-failures-mock-context")
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

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/valid-mock-context")]
    #[tokio::test]
    async fn valid_providers_mock_context(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/valid-mock-context")
            .canonicalize()
            .unwrap();
        let ctx = MockContext::new();
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
}

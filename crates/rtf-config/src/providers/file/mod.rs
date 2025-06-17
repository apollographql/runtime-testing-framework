//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    context::{PathKind, ResolutionContext},
    impl_template,
    providers::Result,
    templating::{self, Field, Scalar, Template},
    validation::{self, Validate},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fmt, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

pub mod apollo;

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
    Local { relative_path: PathBuf },
}

impl RawSource {
    pub async fn try_get_file_content(
        &self,
        dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
        match self {
            Self::Local { relative_path } => Ok(ctx.read_path_to_string(dir.join(relative_path))?),
        }
    }

    pub fn try_into_source(self, dir: &Path, ctx: &impl ResolutionContext) -> io::Result<Source> {
        match self {
            Self::Local { relative_path } => {
                let abs_path = ctx.canonicalize_path(dir.join(relative_path))?;
                Ok(Source::Local { abs_path })
            }
        }
    }
}

/// A file provider is something that can obtain or synthesise utf-8 file content based on a user
/// provided specification.
///
/// This trait is deliberately pub(crate) rather than pub so that the validation and resolution
/// logic is only exposed through the public API as part of the methods on the config file structs.
#[allow(async_fn_in_trait)]
pub(crate) trait AsUtf8FileContent: Validate + DeserializeOwned + fmt::Debug {
    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_get_file_content(
        &self,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<String>;
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
pub(crate) trait ResolveAndWrite: Validate + DeserializeOwned + fmt::Debug {
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
    GraphosCannedOps(apollo::GraphosCannedOps),
    GraphosSubgraphs(apollo::GraphosSubgraphs),
    GraphosSupergraph(apollo::GraphosSupergraph),
    Inline(InlineFile),
    OfflineGraphosLicense(apollo::OfflineGraphosLicense),
    RelativePath(RelativeFile),
    Required(RequiredFile),
}

// Helper for generating boilerplate method impls where we just need to defer to the inner type
// that a FileProvider is wrapping.
macro_rules! delegate_to_inner {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            FileProvider::GraphosCannedOps(fp) => fp.$method($($arg),*),
            FileProvider::GraphosSubgraphs(fp) => fp.$method($($arg),*),
            FileProvider::GraphosSupergraph(fp) => fp.$method($($arg),*),
            FileProvider::Inline(fp) => fp.$method($($arg),*),
            FileProvider::OfflineGraphosLicense(fp) => fp.$method($($arg),*),
            FileProvider::RelativePath(fp) => fp.$method($($arg),*),
            FileProvider::Required(fp) => fp.$method($($arg),*),
        }
    };

    (@async $self:ident, $method:ident, $($arg:expr),*) => {
        match $self {
            FileProvider::GraphosCannedOps(fp) => fp.$method($($arg),*).await,
            FileProvider::GraphosSubgraphs(fp) => fp.$method($($arg),*).await,
            FileProvider::GraphosSupergraph(fp) => fp.$method($($arg),*).await,
            FileProvider::Inline(fp) => fp.$method($($arg),*).await,
            FileProvider::OfflineGraphosLicense(fp) => fp.$method($($arg),*).await,
            FileProvider::RelativePath(fp) => fp.$method($($arg),*).await,
            FileProvider::Required(fp) => fp.$method($($arg),*).await,
        }
    };
}

impl ResolveAndWrite for FileProvider {
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<Vec<(PathBuf, String)>> {
        delegate_to_inner!(@async self, try_get_all_file_contents, target, src, ctx)
    }
}

impl Template for FileProvider {
    fn has_pending_fields(&self) -> bool {
        delegate_to_inner!(self, has_pending_fields)
    }

    fn required_values(&self) -> Vec<String> {
        delegate_to_inner!(self, required_values)
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        delegate_to_inner!(self, try_resolve, path, values)
    }
}

impl Validate for FileProvider {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        delegate_to_inner!(self, try_validate, path, src, ctx)
    }
}

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

impl Validate for InlineFile {
    fn try_validate(
        &self,
        _path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
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

// in try_resolve we don't want to include a trailing ".path" in the resolution path we report to
// users in error messages so we had implement Template for this one.
impl Template for RelativeFile {
    fn has_pending_fields(&self) -> bool {
        self.path.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        self.path.required_values()
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        self.path.try_resolve(path, values)
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
        }
    }
}

impl Validate for RelativeFile {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        let res = match src {
            Source::Local { abs_path } => {
                let dir = match abs_path.parent() {
                    Some(dir) => dir.to_path_buf(),
                    None => PathBuf::new(),
                };
                ctx.canonicalize_path(dir.join(self.path.as_resolved()))
            }
        };

        let p = match res {
            Ok(p) => p,
            Err(e) => {
                let kind = if e.kind() == io::ErrorKind::NotFound {
                    validation::ErrorKind::FileNotFound
                } else {
                    validation::ErrorKind::InvalidRelativePath
                };

                return Err(validation::Errors::new(
                    kind,
                    self.format_error_message(),
                    path,
                ));
            }
        };

        match ctx.path_kind(&p) {
            PathKind::File => Ok(()),
            PathKind::EmptyDir | PathKind::OccupiedDir => Err(validation::Errors::new(
                validation::ErrorKind::IsADirectory,
                self.format_error_message(),
                path,
            )),
            PathKind::Missing => Err(validation::Errors::new(
                validation::ErrorKind::FileNotFound,
                self.format_error_message(),
                path,
            )),
        }
    }
}

/// The only purpose of this file provider is to throw an error if it still exists
/// when the file providers are being validated. All definitions of a required file
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
            "Should not be able to get here. Required file should result in an error when validated."
        )
    }
}

impl_template!(RequiredFile => []);

impl Validate for RequiredFile {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        _src: &Source,
        _ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        Err(validation::Errors::new(
            validation::ErrorKind::RequiredFileMissing,
            &self.message,
            path,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
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

        let res = provider.try_validate(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected to validate but got: {res:?}");

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

    #[dir_cases("crates/rtf-config/resources/provider-tests/file/validation-failures")]
    #[test]
    fn validation_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "validation-errors");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let dir = PathBuf::from("resources/provider-tests/file/validation-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir.join("example.yaml"));
        let res = provider.try_validate(&mut Vec::new(), &src, &ctx);

        assert!(res.is_err(), "expected validation failures");
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

        let res = provider.try_resolve(&mut Vec::new(), &values);

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

        let res = provider.try_resolve(&mut Vec::new(), &values);

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
        let _ = provider.try_validate(&mut Vec::new(), &src, &ctx);
        let res = provider
            .try_get_all_file_contents("expected-file-content", &src, &ctx)
            .await;

        assert!(res.is_err(), "expected resolution failures, got {res:?}");
        let err = res.unwrap_err();
        assert_eq!(&err.to_string(), expected, "wrong resolution errors");
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Required file should result in an error when validated."
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
}

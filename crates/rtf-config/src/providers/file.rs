//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    providers::{Context, Result},
    templating::{self, Scalar, Templatable},
    validation,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fmt, fs, io,
    ops::{Deref, DerefMut},
};

/// A file provider is something that can obtain or synthesise utf-8 file content based on a user
/// provided specification.
///
/// This trait is deliberately pub(crate) rather than pub so that the validation and resolution
/// logic is only exposed through the public API as part of the methods on the config file structs.
#[allow(async_fn_in_trait)]
pub(crate) trait IntoUtf8FileContent: DeserializeOwned + fmt::Debug {
    /// Run any initial static validation available to error early if this provider contains
    /// invalid data.
    fn validate(&self, ctx: &Context) -> validation::Result<()>;

    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_into_file_content(self, ctx: &Context) -> Result<String>;
}

#[derive(Debug, Clone, Deserialize)]
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

// TO DO - RR-50 will use this method in the environment config. Remove
// the dead_code annotation once this is used
#[allow(dead_code)]
impl NamedFileProvider {
    pub(crate) async fn try_into_file_name_and_content(
        self,
        ctx: &Context,
    ) -> (String, Result<String>) {
        let res = self.provider.try_into_file_content(ctx).await;

        (self.name, res)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileProvider {
    Inline(InlineFile),
    LocalPath(LocalFile),
    Required(RequiredFile),
}

macro_rules! delegate_to_inner {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            FileProvider::Inline(fp) => fp.$method($($arg),*),
            FileProvider::LocalPath(fp) => fp.$method($($arg),*),
            FileProvider::Required(fp) => fp.$method($($arg),*),
        }
    };

    (@async $self:ident, $method:ident, $($arg:expr),*) => {
        match $self {
            FileProvider::Inline(fp) => fp.$method($($arg),*).await,
            FileProvider::LocalPath(fp) => fp.$method($($arg),*).await,
            FileProvider::Required(fp) => fp.$method($($arg),*).await,
        }
    };
}

impl IntoUtf8FileContent for FileProvider {
    fn validate(&self, ctx: &Context) -> validation::Result<()> {
        delegate_to_inner!(self, validate, ctx)
    }

    async fn try_into_file_content(self, ctx: &Context) -> Result<String> {
        delegate_to_inner!(@async self, try_into_file_content, ctx)
    }
}

impl Templatable for FileProvider {
    fn has_pending_fields(&self) -> bool {
        delegate_to_inner!(self, has_pending_fields)
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<&'static str>,
        values: &HashMap<String, Scalar>,
        errs: &mut Vec<templating::Error>,
    ) {
        delegate_to_inner!(self, try_resolve, path, values, errs)
    }
}

/// The simplest form of file provider: the user specifies the contents of the file inline within
/// their config file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InlineFile {
    content: String,
}

impl IntoUtf8FileContent for InlineFile {
    fn validate(&self, _ctx: &Context) -> validation::Result<()> {
        Ok(())
    }

    async fn try_into_file_content(self, _ctx: &Context) -> Result<String> {
        Ok(self.content)
    }
}

impl Templatable for InlineFile {
    fn has_pending_fields(&self) -> bool {
        false
    }

    fn try_resolve(
        &mut self,
        _path: &mut Vec<&'static str>,
        _values: &HashMap<String, Scalar>,
        _errs: &mut Vec<templating::Error>,
    ) {
    }
}

/// The user specifies a path to a local file relative to the config
/// file containing this provider
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LocalFile {
    relative_path: String,
}

impl LocalFile {
    fn format_error_message(&self) -> String {
        format!("Provided path was {:?}", self.relative_path)
    }
}

impl Templatable for LocalFile {
    fn has_pending_fields(&self) -> bool {
        false
    }

    fn try_resolve(
        &mut self,
        _path: &mut Vec<&'static str>,
        _values: &HashMap<String, Scalar>,
        _errs: &mut Vec<templating::Error>,
    ) {
    }
}

impl IntoUtf8FileContent for LocalFile {
    fn validate(&self, ctx: &Context) -> validation::Result<()> {
        let p = ctx.config_dir.join(&self.relative_path);
        let p = match p.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                let kind = if e.kind() == io::ErrorKind::NotFound {
                    validation::ErrorKind::FileNotFound
                } else {
                    validation::ErrorKind::InvalidRelativePath
                };

                return Err(validation::Errors::new(kind, self.format_error_message()));
            }
        };

        if !p.exists() {
            return Err(validation::Errors::new(
                validation::ErrorKind::FileNotFound,
                self.format_error_message(),
            ));
        }
        if !p.is_file() {
            return Err(validation::Errors::new(
                validation::ErrorKind::IsADirectory,
                self.format_error_message(),
            ));
        }

        Ok(())
    }

    // TO DO: Can we get reading a file to come from the Context?
    async fn try_into_file_content(self, ctx: &Context) -> Result<String> {
        let p = ctx.config_dir.join(&self.relative_path).canonicalize()?;

        Ok(fs::read_to_string(p)?)
    }
}

/// The only purpose of this file provider is to throw an error if it still exists
/// when the file providers are being validated. All definitions of a required file
/// are expected to be replaced by user defined file providers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RequiredFile {
    message: String,
}

impl Templatable for RequiredFile {
    fn has_pending_fields(&self) -> bool {
        false
    }

    fn try_resolve(
        &mut self,
        _path: &mut Vec<&'static str>,
        _values: &HashMap<String, Scalar>,
        _errs: &mut Vec<templating::Error>,
    ) {
        // no-op as we never have anything to resolve but need to satisfy the trait so that
        // FileProviders can be resolved as a batch operation
    }
}

impl IntoUtf8FileContent for RequiredFile {
    fn validate(&self, _ctx: &Context) -> validation::Result<()> {
        Err(validation::Errors::new(
            validation::ErrorKind::RequiredFileMissing,
            &self.message,
        ))
    }

    async fn try_into_file_content(self, _ctx: &Context) -> Result<String> {
        panic!(
            "Should not be able to get here. Required file should result in an error when validated."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let expected = get_file(&arr, "expected-file-content");

        let provider: FileProvider = match serde_yaml::from_str(config) {
            Ok(provider) => provider,
            Err(e) => panic!("expected a valid FileProvider, got: {e}"),
        };

        let ctx = Context::new(
            PathBuf::from("resources/provider-tests/file/valid")
                .canonicalize()
                .unwrap(),
        );

        let res = provider.validate(&ctx);
        assert!(res.is_ok(), "expected to validate but got: {res:?}");

        let res = provider.try_into_file_content(&ctx).await;
        assert_eq!(res.unwrap(), expected, "wrong file content");
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

        let ctx = Context::new(
            PathBuf::from("resources/provider-tests/file/validation-failures")
                .canonicalize()
                .unwrap(),
        );
        let res = provider.validate(&ctx);

        assert!(res.is_err(), "expected validation failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind()));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
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

        let ctx = Context::new(
            PathBuf::from("resources/provider-tests/file/resolution-failures")
                .canonicalize()
                .unwrap(),
        );
        let _ = provider.validate(&ctx);
        let res = provider.try_into_file_content(&ctx).await;

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
        let ctx = Context::new(PathBuf::from("not/used/in/this/test"));

        // Calling try_into_file_content should panic here
        _ = required_file.try_into_file_content(&ctx).await;
    }
}

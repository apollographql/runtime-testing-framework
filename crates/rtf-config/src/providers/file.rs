//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    providers::{Context, Result},
    validation,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{fmt, fs, io};

/// A file provider is something that can obtain or synthesise utf-8 file content based on a user
/// provided specification.
#[allow(async_fn_in_trait)]
pub trait IntoUtf8FileContent: DeserializeOwned + fmt::Debug {
    /// Run any initial static validation available to error early if this provider contains
    /// invalid data.
    fn validate(&self, ctx: &Context) -> validation::Result<()>;

    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_into_file_content(self, ctx: &Context) -> Result<String>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileParam {
    pub name: String,
    #[serde(flatten)]
    pub provider: FileProvider,
}

impl FileParam {
    pub async fn try_into_file_name_and_content(self, ctx: &Context) -> (String, Result<String>) {
        let res = self.provider.try_into_file_content(ctx).await;

        (self.name, res)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileProvider {
    Inline(InlineFile),
    LocalPath(LocalFile),
}

impl IntoUtf8FileContent for FileProvider {
    fn validate(&self, ctx: &Context) -> validation::Result<()> {
        match self {
            Self::Inline(fp) => fp.validate(ctx),
            Self::LocalPath(fp) => fp.validate(ctx),
        }
    }

    async fn try_into_file_content(self, ctx: &Context) -> Result<String> {
        match self {
            Self::Inline(fp) => fp.try_into_file_content(ctx).await,
            Self::LocalPath(fp) => fp.try_into_file_content(ctx).await,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::ErrorKind;
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;
    use std::path::PathBuf;

    #[derive(Debug, Clone, Copy)]
    enum Scenario<'a> {
        Valid(&'a str),
        InvalidYaml,
        ValidationFailures(&'a str),
        ResolutionFailures(&'a str),
    }

    impl<'a> Scenario<'a> {
        /// This will panic if the archive is invalid
        fn from_archive(arr: &'a Archive) -> (&'a str, Self) {
            let config = match arr.get("config.yaml") {
                Some(f) => f.content.trim(),
                None => {
                    panic!("Error: 'config.yaml' not found in the archive");
                }
            };

            let valid = arr.get("expected-file-content");
            let invalid = arr.get("validation-error");
            let failed = arr.get("resolution-error");
            let yaml = arr.get("invalid-yaml");

            let scenario = match (valid, invalid, failed, yaml) {
                (Some(f), None, None, None) => Self::Valid(f.content.trim()),
                (None, Some(f), None, None) => Self::ValidationFailures(f.content.trim()),
                (None, None, Some(f), None) => Self::ResolutionFailures(f.content.trim()),
                (None, None, None, Some(_)) => Self::InvalidYaml,
                (None, None, None, None) => panic!("no assertion scenario provided"),
                _ => panic!("conflicting assertion scenarios provided"),
            };

            (config, scenario)
        }
    }

    #[dir_cases("crates/rtf-config/resources/provider-tests")]
    #[tokio::test]
    async fn file_provider_scenarios(_path: &str, content: &str) {
        let arr = Archive::from(content);
        let comment = arr.comment();
        if !comment.is_empty() {
            println!("{}", comment.trim());
        }

        let (config, scenario) = Scenario::from_archive(&arr);

        // Test that the fragment parses
        let res: serde_yaml::Result<FileProvider> = serde_yaml::from_str(config);
        match scenario {
            Scenario::InvalidYaml => {
                assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
                return;
            }
            _ => assert!(res.is_ok(), "invalid YAML: {res:?}"),
        }

        let provider = res.unwrap();
        let ctx = Context::new(
            PathBuf::from("resources/provider-tests")
                .canonicalize()
                .unwrap(),
        );

        // Test that the fragment validates
        let res = provider.validate(&ctx);
        match scenario {
            Scenario::ValidationFailures(expected) => {
                assert!(res.is_err(), "expected validation failures");
                let err = res.unwrap_err();
                assert_eq!(&err.to_string(), expected, "wrong validation errors");
                return;
            }
            _ => assert!(res.is_ok(), "validation failed: {res:?}"),
        }

        // Test that the file content is as expected
        let res = provider.try_into_file_content(&ctx).await;
        match scenario {
            Scenario::ResolutionFailures(expected) => {
                assert!(res.is_err(), "expected resolution failures, got {res:?}");
                let err = res.unwrap_err();
                assert_eq!(&err.to_string(), expected, "wrong resolution errors");
                return;
            }

            Scenario::Valid(expected) => {
                assert_eq!(res.unwrap(), expected, "wrong file content");
            }

            _ => unreachable!("other cases should have been handled above"),
        }
    }

    #[test]
    fn local_file_provider_path_does_not_exist_returns_not_found_error() {
        let provider = LocalFile {
            relative_path: "../does-not-exist.txt".to_string(),
        };
        let p = PathBuf::from("resources/provider-tests/")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);
        let res = provider.validate(&ctx);

        assert!(res.is_err(), "{res:?}");
        let res = res.unwrap_err().unwrap_single();

        assert!(
            matches!(res.kind(), ErrorKind::FileNotFound),
            "expected FileNotFound, got {res:?}"
        );
    }

    #[test]
    fn local_file_provider_path_is_directory_returns_is_directory_error() {
        let provider = LocalFile {
            relative_path: "".to_string(),
        };
        let p = PathBuf::from("resources/provider-tests")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);
        let res = provider.validate(&ctx);

        assert!(res.is_err(), "{res:?}");
        let res = res.unwrap_err().unwrap_single();

        assert!(
            matches!(res.kind(), ErrorKind::IsADirectory),
            "expected IsADirectory, got {res:?}"
        );
    }
}

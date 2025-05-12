//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::providers::{Context, Result};
use serde::{Deserialize, de::DeserializeOwned};
use std::{fmt, fs, io};

/// A file provider is something that can obtain or synthesise utf-8 file content based on a user
/// provided specification.
#[allow(async_fn_in_trait)]
pub trait IntoUtf8FileContent: DeserializeOwned + fmt::Debug {
    /// Run any initial static validation available to error early if this provider contains
    /// invalid data.
    fn validate(&self, ctx: &Context) -> Result<()>;

    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_into_file_content(self, ctx: &Context) -> Result<String>;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileProvider {
    Inline(InlineFile),
    LocalPath(LocalFile),
}

impl IntoUtf8FileContent for FileProvider {
    fn validate(&self, ctx: &Context) -> Result<()> {
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
    fn validate(&self, _ctx: &Context) -> Result<()> {
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

impl IntoUtf8FileContent for LocalFile {
    fn validate(&self, ctx: &Context) -> Result<()> {
        let p = ctx.config_dir.join(&self.relative_path).canonicalize()?;
        if !p.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", p.display()),
            )
            .into());
        }
        if !p.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("Specified file is a directory: {}", p.display()),
            )
            .into());
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
    use simple_test_case::dir_cases;
    use std::path::PathBuf;

    #[dir_cases("crates/rtf-config/resources/provider-tests/valid")]
    #[test]
    fn valid_provider_fragments_parse(_path: &str, content: &str) {
        let res: serde_yaml::Result<FileProvider> = serde_yaml::from_str(content);

        assert!(res.is_ok(), "{res:?}");
    }

    #[test]
    fn local_file_provider_validates() {
        let provider = LocalFile {
            relative_path: "../../example.txt".to_string(),
        };
        let p = PathBuf::from("resources/provider-tests/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);
        let res = provider.validate(&ctx);

        assert!(res.is_ok(), "{res:?}");
    }

    #[test]
    fn local_file_provider_path_does_not_exist_returns_not_found_error() {
        let provider = LocalFile {
            relative_path: "../does-not-exist.txt".to_string(),
        };
        let p = PathBuf::from("resources/provider-tests/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);
        let res = provider.validate(&ctx);

        assert!(res.is_err(), "{res:?}");
        let res = res.unwrap_err();
        assert!(matches!(res.kind(), io::ErrorKind::NotFound), "{res:?}");
    }

    #[test]
    fn local_file_provider_path_is_directory_returns_is_directory_error() {
        let provider = LocalFile {
            relative_path: "valid".to_string(),
        };
        let p = PathBuf::from("resources/provider-tests")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);
        let res = provider.validate(&ctx);

        assert!(res.is_err(), "{res:?}");
        let res = res.unwrap_err();
        assert!(matches!(res.kind(), io::ErrorKind::IsADirectory), "{res:?}");
    }

    #[tokio::test]
    async fn local_file_provider_returns_correct_file_content() {
        let provider = LocalFile {
            relative_path: "../../example.txt".to_string(),
        };
        let p = PathBuf::from("resources/provider-tests/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);
        let res = provider.try_into_file_content(&ctx).await;

        assert!(res.is_ok(), "{res:?}");
        assert_eq!(res.unwrap(), include_str!("../../resources/example.txt"))
    }
}

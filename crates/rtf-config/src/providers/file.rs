//! The core [FileProvider] trait and currently supported file provider implementations.
use serde::{Deserialize, de::DeserializeOwned};
use std::fmt;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum FileProvider {
    Inline(InlineFile),
}

/// A file provider is something that can obtain or synthesise file content
/// based on a user provided specification.
#[allow(async_fn_in_trait)]
pub trait IntoFileContent: DeserializeOwned + fmt::Debug {
    type Error; // should this be fixed? probably...or at least Into<UserFacingError>

    /// Run any initial static validation available to error early if this provider
    /// contains invalid data.
    fn validate(&self) -> Result<(), Self::Error>;

    // TODO: we will need to pass in things like API clients eventually

    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_into_file_content(self) -> Result<String, Self::Error>;
}

/// The simplest form of file provider: the user specifies the contents of the
/// file inline within their config file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InlineFile {
    content: String,
}

impl IntoFileContent for InlineFile {
    type Error = ();

    fn validate(&self) -> Result<(), ()> {
        Ok(())
    }

    async fn try_into_file_content(self) -> Result<String, ()> {
        Ok(self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::dir_cases;

    #[dir_cases("crates/rtf-config/resources/provider-tests/valid")]
    #[test]
    fn valid_provider_fragments_parse(_path: &str, content: &str) {
        let res: serde_yaml::Result<FileProvider> = serde_yaml::from_str(content);
        assert!(res.is_ok(), "{res:?}");
    }
}

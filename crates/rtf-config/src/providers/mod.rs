//! Providers are how we expose the rest of the framework to user facing config.
use crate::providers::{command::CommandProvider, file::FileProvider};
use rtf_integrations::graphos::supergraph::FetchError;
use serde::Serialize;
use std::io;

pub mod command;
pub mod file;

/// Errors that can be encountered while running file providers
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Fetch(#[from] FetchError),

    #[error(transparent)]
    Github(#[from] rtf_integrations::github::Error),

    #[error(transparent)]
    GraphOS(#[from] rtf_integrations::graphos::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error("Unable to execute the {name} command: {err}")]
    CommandFailed { name: String, err: String },

    #[error("Missing provider output for {name}")]
    MissingProviderOutput { name: String },

    #[error("Custom providers are not permitted to make use of nested custom providers")]
    NestedCustomProvider,

    #[error("Unable to resolve and write {name} file: {err}")]
    ResolveAndWriteFailed { name: String, err: String },

    #[error("Request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Error decoding bytes to utf8")]
    Utf8DecodingError,

    #[error("Unknown router version: {0}")]
    UnknownRouterVersion(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Wrapper enum for supporting caching of provider output
#[derive(Debug, Serialize)]
pub enum Provider<'a> {
    File {
        fp: &'a FileProvider,
    },
    Command {
        name: &'a str,
        cmd: &'a CommandProvider,
    },
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use assert_fs::{
        TempDir,
        assert::PathAssert,
        fixture::{ChildPath, FileWriteStr, PathChild},
    };
    use predicates::path;
    use std::fs::read_to_string;

    /// Create a temp directory with a file on the specified path
    pub(crate) fn create_temp_dir_with_file(
        file_path: &str,
        file_content: &str,
    ) -> (TempDir, ChildPath) {
        let temp = TempDir::new().unwrap();

        let file = temp.child(file_path);
        file.write_str(file_content).unwrap();

        (temp, file)
    }

    /// Assert file content matches expected
    pub(crate) fn assert_file_content(file: &ChildPath, expected_content: &str) {
        file.assert(path::exists());
        let file_contents = read_to_string(file).unwrap();
        assert_eq!(
            file_contents, expected_content,
            "ensure the file content is as expected"
        )
    }
}

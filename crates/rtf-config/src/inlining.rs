use crate::providers;

/// User facing descriptions of the reason that inlining a [`crate::providers::file::RelativeFile`] failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "Failed to retrieve file content")]
    FailedToRetrieveFileContent,
}

impl crate::error::ErrorKind for ErrorKind {
    const HEADER: &str = "Inlining failed";
}

// Type aliases for inlining error handling.
// Elsewhere in the codebase we should always refer to these aliases rather than parameterising the
// generic types from the error module.

pub type Error = crate::error::Error<ErrorKind>;
pub type Errors = crate::error::Errors<ErrorKind>;
pub type ErrorBuilder = crate::error::ErrorBuilder<ErrorKind>;
pub type Result<T> = std::result::Result<T, Errors>;

impl From<providers::Error> for Errors {
    fn from(err: providers::Error) -> Self {
        Self::new(ErrorKind::FailedToRetrieveFileContent, err.to_string(), &[])
    }
}

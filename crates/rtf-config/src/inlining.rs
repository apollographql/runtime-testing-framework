use crate::{
    providers,
    providers::file::{InlineDir, InlineFile},
    templating,
};
use serde::Serialize;
use std::hash::{DefaultHasher, Hash, Hasher};

/// User facing descriptions of the reason that inlining a [`crate::providers::file::RelativeFile`] failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "Failed to retrieve file content")]
    FailedToRetrieveFileContent,

    #[strum(to_string = "Failed to template test plan")]
    FailedToTemplateTestPlan,
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

impl From<templating::Errors> for Errors {
    fn from(errs: templating::Errors) -> Self {
        let mut builder = ErrorBuilder::new();

        for err in errs.iter() {
            builder.push(ErrorKind::FailedToTemplateTestPlan, err.to_string(), &[]);
        }

        match builder.into_result(()) {
            Ok(_) => Self::new(
                ErrorKind::FailedToTemplateTestPlan,
                "Templating failed",
                &[],
            ),
            Err(e) => e,
        }
    }
}

/// The mode used to inline files
#[derive(Debug)]
pub enum InlineMode {
    All,
    RelativeFiles,
}

/// The cached result of fully resolving a file provider during inlining.
#[derive(Debug, Clone)]
pub enum InlinedProvider {
    File(InlineFile),
    Dir(InlineDir),
}

/// Compute a stable `u64` cache key for a serializable provider.
///
/// We rely on YAML serialization to produce a canonical byte representation which we then hash to
/// generate the key in order to avoid arbitrary length strings as keys for the cache.
/// This means that two providers with identical YAML representations will produce the same cache
/// key, which is fine as we would deserialize them as the same type from YAML anyway.
pub(crate) fn provider_cache_key<T>(value: &T) -> u64
where
    T: Serialize,
{
    let mut hasher = DefaultHasher::new();
    serde_yaml::to_string(value)
        .expect("to serialize")
        .hash(&mut hasher);

    hasher.finish()
}

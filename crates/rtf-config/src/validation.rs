//! Helpers for validating config files
use crate::{context::ResolutionContext, providers::file::Source};
use enum_dispatch::enum_dispatch;
use std::collections::HashSet;

// We need to bring these into scope for enum_dispatch to be able to pick them up
use crate::providers::{command::CommandProvider, file::FileProvider};

/// User facing descriptions of the reason that validation failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "non-unique environment variables found.")]
    DuplicateEnvironmentVariables,

    #[strum(to_string = "non-unique value names found.")]
    DuplicateValueNames,

    #[strum(to_string = "the requested file did not exist.")]
    FileNotFound,

    #[strum(to_string = "the provided string was not a valid graph ref")]
    InvalidGraphRef,

    #[strum(to_string = "the given relative path was not a valid path.")]
    InvalidRelativePath,

    #[strum(to_string = "a directory was provided when a file was expected.")]
    IsADirectory,

    #[strum(to_string = "no API key provided for calling the Apollo GraphOS API")]
    MissingGraphOsApiKey,

    #[strum(to_string = "a required file has not been defined.")]
    RequiredFileMissing,

    #[strum(to_string = "http client not found.")]
    HttpClientNotFound,
}

// Type aliases for validation error handling.
// Elsewhere in the codebase we should always refer to these aliases rather than parameterising the
// generic types from the error module.

pub type Error = crate::error::Error<ErrorKind>;
pub type Errors = crate::error::Errors<ErrorKind>;
pub type ErrorBuilder = crate::error::ErrorBuilder<ErrorKind>;
pub type Result<T> = std::result::Result<T, Errors>;

#[enum_dispatch]
pub trait Validate {
    /// Run any initial static validation available to error early if this provider contains
    /// invalid data.
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<()>;

    fn try_validate_nested(
        &self,
        path: &mut Vec<String>,
        tail: impl Into<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.into());

        self.try_validate(&mut path, src, ctx)
    }
}

/// Determine if there are any duplicates within a given slices of elements using a given key
/// function.
///
/// See the tests in the validation module for example usage.
pub(crate) fn duplicate_keys<'a, T: 'a>(
    elems: impl Iterator<Item = T>,
    key_fn: impl Fn(T) -> &'a str,
) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();

    for elem in elems.into_iter() {
        let k = key_fn(elem);
        if seen.contains(k) {
            duplicates.push(k);
        } else {
            seen.insert(k);
        }
    }

    // ensure that our reported duplicates are in alphabetical order
    duplicates.sort_unstable();
    duplicates.dedup();

    duplicates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Mapping {
        k: &'static str,
        _v: usize,
    }

    #[test]
    fn duplicate_keys_returns_empty_vec_for_no_duplicates() {
        let vals = [
            Mapping { k: "a", _v: 1 },
            Mapping { k: "c", _v: 2 },
            Mapping { k: "b", _v: 3 },
            Mapping { k: "d", _v: 4 },
            Mapping { k: "x", _v: 5 },
        ];
        let duplicates = duplicate_keys(vals.iter(), |s| s.k);

        assert!(
            duplicates.is_empty(),
            "expected no duplicates, got {duplicates:?}"
        );
    }

    #[test]
    fn duplicate_keys_returns_sorted_results() {
        let vals = [
            Mapping { k: "c", _v: 1 },
            Mapping { k: "c", _v: 2 },
            Mapping { k: "a", _v: 3 },
            Mapping { k: "a", _v: 4 },
            Mapping { k: "b", _v: 5 },
        ];
        let duplicates = duplicate_keys(vals.iter(), |s| s.k);

        assert_eq!(duplicates, vec!["a", "c"]);
    }

    #[test]
    fn duplicate_keys_returns_unique_results() {
        let vals = [
            Mapping { k: "a", _v: 1 },
            Mapping { k: "c", _v: 2 },
            Mapping { k: "c", _v: 3 },
            Mapping { k: "c", _v: 4 },
            Mapping { k: "c", _v: 5 },
        ];
        let duplicates = duplicate_keys(vals.iter(), |s| s.k);

        assert_eq!(duplicates, vec!["c"]);
    }
}

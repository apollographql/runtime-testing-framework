//! Helpers for checking config files
use crate::{
    ValueDefinition,
    context::ResolutionContext,
    providers::file::{NamedFileProvider, Source},
};
use std::{collections::HashMap, hash::Hash, mem};

/// User facing descriptions of the reason that validation failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "Non-unique environment variables found")]
    DuplicateEnvironmentVariables,

    #[strum(to_string = "Non-unique value names found")]
    DuplicateValueNames,

    #[strum(to_string = "The requested file did not exist")]
    FileNotFound,

    #[strum(to_string = "The provided string was not a valid graph ref")]
    InvalidGraphRef,

    #[strum(to_string = "The given relative path was not a valid path")]
    InvalidRelativePath,

    #[strum(to_string = "A directory was provided when a file was expected")]
    IsADirectory,

    #[strum(to_string = "No API key provided for calling the GitHub API")]
    MissingGithubApiKey,

    #[strum(to_string = "No API key provided for calling the Apollo GraphOS API")]
    MissingGraphOsApiKey,

    #[strum(to_string = "A required file has not been defined")]
    RequiredFileMissing,
}

impl crate::error::ErrorKind for ErrorKind {
    const HEADER: &str = "Static analysis checks failed";
}

// Type aliases for validation error handling.
// Elsewhere in the codebase we should always refer to these aliases rather than parameterising the
// generic types from the error module.

pub type Error = crate::error::Error<ErrorKind>;
pub type Errors = crate::error::Errors<ErrorKind>;
pub type ErrorBuilder = crate::error::ErrorBuilder<ErrorKind>;
pub type Result<T> = std::result::Result<T, Errors>;

pub trait Check {
    /// Run any initial static check available to error early if this provider contains
    /// invalid data.
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<()>;

    fn try_check_nested(
        &self,
        path: &mut Vec<String>,
        tail: impl Into<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.into());

        self.try_check(&mut path, src, ctx)
    }
}

/// Helper macro for stamping out implementations of the [Check] trait on an enum where each
/// variant is a wrapper around a type that already implements the trait.
#[macro_export]
macro_rules! enum_impl_check {
    ($enum:ident => $($variant:ident),+) => {
        impl Check for $enum {
            fn try_check(
                &self,
                path: &mut Vec<String>,
                src: &Source,
                ctx: &impl ResolutionContext,
            ) -> $crate::checks::Result<()> {
                match self {
                    $(Self::$variant(inner) => inner.try_check(path, src, ctx),)+
                }
            }
            fn try_check_nested(
                &self,
                path: &mut Vec<String>,
                tail: impl Into<String>,
                src: &Source,
                ctx: &impl ResolutionContext,
            ) -> $crate::checks::Result<()> {
                match self {
                    $(Self::$variant(inner) => inner.try_check_nested(path, tail, src, ctx),)+
                }
            }
        }
    };
}

#[derive(Debug)]
pub enum DedupArray<'a> {
    ValueDef(&'static str, &'a mut Vec<ValueDefinition>),
    Nfp(&'static str, &'a mut Vec<NamedFileProvider>),
}

/// Checks for conflicts between different arrays are handled in the implementation of [Check]
pub trait CheckArrayDuplicates {
    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<DedupArray<'a>>;

    fn ensure_no_duplicate_keys(&mut self, path: &mut Vec<String>) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for arr in self.deduplicated_arrays() {
            let (p, duplicates) = match arr {
                DedupArray::ValueDef(p, vds) => (p, duplicate_keys(vds.iter(), |vd| &vd.name)),
                DedupArray::Nfp(p, nfps) => (p, duplicate_keys(nfps.iter(), |nfp| &nfp.env_var)),
            };

            if !duplicates.is_empty() {
                let mut nested_path = path.clone();
                nested_path.push(p.to_string());
                errs.push(
                    ErrorKind::DuplicateValueNames,
                    duplicates.join("\n"),
                    &nested_path,
                );
            }
        }

        errs.into_result(())
    }

    fn dedup_and_sort(&mut self, path: &mut Vec<String>) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for arr in self.deduplicated_arrays() {
            match arr {
                DedupArray::ValueDef(p, vds) => {
                    errs.append(dedup_and_sort_by_key(vds, |vd| vd.name.clone(), path, p))
                }
                DedupArray::Nfp(p, nfps) => errs.append(dedup_and_sort_by_key(
                    nfps,
                    |nfp| nfp.env_var.clone(),
                    path,
                    p,
                )),
            }
        }

        errs.into_result(())
    }
}

/// Helper function for deduplicating list entries. We are depulicating in this
/// way because we know the user is overriding specific items. We are taking
/// the last entry on the list as the one to keep.
fn dedup_and_sort_by_key<T>(
    v: &mut Vec<T>,
    key_fn: fn(&T) -> String,
    path: &mut [String],
    p: &str,
) -> Result<()> {
    let duplicates = duplicate_keys_with_threshold(v.iter(), key_fn, 2);
    if !duplicates.is_empty() {
        let mut nested_path = path.to_vec();
        nested_path.push(p.to_string());
        return Err(Errors::new(
            ErrorKind::DuplicateValueNames,
            duplicates.join("\n"),
            &nested_path,
        ));
    }

    let mut m = HashMap::with_capacity(v.len());
    for item in mem::take(v).into_iter() {
        m.insert(key_fn(&item), item);
    }
    let mut deduped: Vec<T> = m.into_values().collect();
    deduped.sort_by_key(key_fn);

    *v = deduped;

    Ok(())
}

fn duplicate_keys_with_threshold<'a, T: 'a, K>(
    elems: impl Iterator<Item = T>,
    key_fn: impl Fn(T) -> K,
    threshold: usize,
) -> Vec<K>
where
    K: Eq + Hash + Ord,
{
    let mut counts: HashMap<K, usize> = HashMap::new();
    for elem in elems.into_iter() {
        *counts.entry(key_fn(elem)).or_default() += 1;
    }
    counts.retain(|_, n| *n > threshold);

    let mut duplicates: Vec<_> = counts.into_keys().collect();
    duplicates.sort_unstable();

    duplicates
}

/// Determine if there are any duplicates within a given slices of elements using a given key
/// function.
///
/// See the tests in the validation module for example usage.
pub(crate) fn duplicate_keys<'a, T: 'a>(
    elems: impl Iterator<Item = T>,
    key_fn: impl Fn(T) -> &'a str,
) -> Vec<&'a str> {
    duplicate_keys_with_threshold(elems, key_fn, 1)
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

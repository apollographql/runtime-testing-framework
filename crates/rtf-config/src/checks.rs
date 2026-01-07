//! Helpers for checking config files
use crate::{VariableDefinition, context::ResolutionContext, providers::file::NamedFileProvider};
use std::{collections::HashMap, hash::Hash, mem};

/// User facing descriptions of the reason that validation failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "Non-unique environment variables found")]
    DuplicateEnvironmentVariables,

    #[strum(to_string = "Non-unique file provider names found")]
    DuplicateFileProviderNames,

    #[strum(to_string = "Non-unique variable names found")]
    DuplicateVariableNames,

    #[strum(to_string = "An array was empty when at least one element was expected")]
    EmptyArray,

    #[strum(to_string = "The requested file did not exist")]
    FileNotFound,

    #[strum(to_string = "The provided string was not a valid duration")]
    InvalidDuration,

    #[strum(to_string = "The provided string was not a valid graph ref")]
    InvalidGraphRef,

    #[strum(to_string = "Invalid path specifiers")]
    InvalidPathSpecifiers,

    #[strum(to_string = "The given relative path was not a valid path")]
    InvalidRelativePath,

    #[strum(to_string = "A directory was provided when a file was expected")]
    IsADirectory,

    #[strum(to_string = "No API key provided for calling the GitHub API")]
    MissingGithubApiKey,

    #[strum(to_string = "No API key provided for calling the Apollo GraphOS API")]
    MissingGraphOsApiKey,

    #[strum(to_string = "No matching conditional cases for provided variables")]
    NoMatchingCases,

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
    fn try_check(&self, path: &mut Vec<String>, ctx: &impl ResolutionContext) -> Result<()>;

    fn try_check_nested(
        &self,
        path: &mut Vec<String>,
        tail: impl Into<String>,
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.into());

        self.try_check(&mut path, ctx)
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
                ctx: &impl ResolutionContext,
            ) -> $crate::checks::Result<()> {
                match self {
                    $(Self::$variant(inner) => inner.try_check(path, ctx),)+
                }
            }
            fn try_check_nested(
                &self,
                path: &mut Vec<String>,
                tail: impl Into<String>,
                ctx: &impl ResolutionContext,
            ) -> $crate::checks::Result<()> {
                match self {
                    $(Self::$variant(inner) => inner.try_check_nested(path, tail, ctx),)+
                }
            }
        }
    };
}

/// Checks for conflicts between different arrays are handled in the implementation of [Check]
pub trait CheckArrayDuplicates {
    const BASE_PATH: &str;

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)>;

    fn ensure_no_duplicate_keys(&mut self) -> Result<()> {
        let mut errs = ErrorBuilder::new();
        for (p, arr) in self.deduplicated_arrays() {
            errs.append(arr.ensure_no_duplicate_keys(Self::BASE_PATH, p));
        }

        errs.into_result(())
    }

    fn sort_arrays(&mut self) {
        for (_, mut arr) in self.deduplicated_arrays() {
            arr.sort();
        }
    }

    fn try_dedup_and_sort(&mut self) -> Result<()> {
        let mut errs = ErrorBuilder::new();
        let mut arrs = self.deduplicated_arrays();

        // Allow for up to two duplicates when applying overrides (one in the base and one in the
        // overrides). We check this first to ensure that all arrays are valid before we attempt
        // to deduplicate any of them.
        for (p, arr) in arrs.iter_mut() {
            errs.append(arr.at_most_two_duplicates(Self::BASE_PATH, p));
        }

        errs.into_result(())?;

        for (_, mut arr) in arrs {
            arr.dedup_and_sort();
        }

        Ok(())
    }
}

/// Wrapper around the array types we need to be able to check and dedup as part of merging
/// overrides in test plans.
#[derive(Debug, PartialEq)]
pub enum DedupArray<'a> {
    VariableDef(&'a mut Vec<VariableDefinition>),
    Nfp(&'a mut Vec<NamedFileProvider>),
}

impl<'a> DedupArray<'a> {
    fn ensure_no_duplicate_keys(&self, base_path: &str, p: &str) -> Result<()> {
        let duplicates = match self {
            DedupArray::VariableDef(vds) => duplicate_keys(vds.iter(), |vd| &vd.name),
            DedupArray::Nfp(nfps) => duplicate_keys(nfps.iter(), |nfp| &nfp.env_var),
        };

        if !duplicates.is_empty() {
            let path = vec![base_path.to_string(), p.to_string()];
            return Err(Errors::new(
                ErrorKind::DuplicateVariableNames,
                duplicates.join("\n"),
                &path,
            ));
        }

        Ok(())
    }

    fn sort(&mut self) {
        match self {
            DedupArray::VariableDef(vds) => vds.sort_by_key(|vd| vd.name.clone()),
            DedupArray::Nfp(nfps) => nfps.sort_by_key(|nfp| nfp.env_var.clone()),
        }
    }

    fn at_most_two_duplicates(&self, base_path: &str, p: &str) -> Result<()> {
        fn inner<T>(v: &[T], key_fn: fn(&T) -> String, base_path: &str, p: &str) -> Result<()> {
            let duplicates = duplicate_keys_with_threshold(v.iter(), key_fn, 2);
            if !duplicates.is_empty() {
                let path = vec![base_path.to_string(), p.to_string()];
                return Err(Errors::new(
                    ErrorKind::DuplicateVariableNames,
                    duplicates.join("\n"),
                    &path,
                ));
            }

            Ok(())
        }

        match self {
            DedupArray::VariableDef(vds) => inner(vds, |vd| vd.name.clone(), base_path, p),
            DedupArray::Nfp(nfps) => inner(nfps, |nfp| nfp.env_var.clone(), base_path, p),
        }
    }

    fn dedup_and_sort(&mut self) {
        /// We are depulicating in this way because we know the user is overriding specific items.
        /// We are taking the last entry on the list as the one to keep.
        fn inner<T>(v: &mut Vec<T>, key_fn: fn(&T) -> String) {
            let mut m = HashMap::with_capacity(v.len());
            for item in mem::take(v).into_iter() {
                m.insert(key_fn(&item), item);
            }
            let mut deduped: Vec<T> = m.into_values().collect();
            deduped.sort_by_key(key_fn);

            *v = deduped;
        }

        match self {
            DedupArray::VariableDef(vds) => inner(vds, |vd| vd.name.clone()),
            DedupArray::Nfp(nfps) => inner(nfps, |nfp| nfp.env_var.clone()),
        }
    }
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
    use crate::{
        providers::file::{FileProvider, InlineFile},
        templating::Scalar,
    };
    use simple_test_case::test_case;

    #[derive(Debug)]
    struct Mapping {
        k: &'static str,
        _v: usize,
    }

    #[test]
    fn duplicate_keys_returns_empty_for_no_duplicates() {
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

    fn vd(name: &str, default: Option<Scalar>) -> VariableDefinition {
        VariableDefinition {
            name: name.into(),
            description: format!("description for {name}"),
            default,
            allowed_values: None,
        }
    }

    fn nfp(name: &str, env_var: &str, content: &str) -> NamedFileProvider {
        NamedFileProvider {
            name: name.to_string(),
            env_var: env_var.to_string(),
            provider: FileProvider::Inline(InlineFile {
                content: content.to_string(),
            }),
        }
    }

    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("b", None)]),
        false;
        "vd no duplicates"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", ""), nfp("b", "B", "")]),
        false;
        "nfp no duplicates"
    )]
    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("a", None)]),
        true;
        "vd single duplicate"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", ""), nfp("a", "A", "")]),
        true;
        "nfp single duplicate"
    )]
    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("a", None), vd("a", None)]),
        true;
        "vd multiple duplicates"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", ""), nfp("a", "A", ""), nfp("a", "A", "")]),
        true;
        "nfp multiple duplicates"
    )]
    #[test]
    fn ensure_no_duplicate_keys_errors_correctly(arr: DedupArray<'_>, is_err: bool) {
        let res = arr.ensure_no_duplicate_keys("BASE", "path");

        assert_eq!(
            res.is_err(),
            is_err,
            "expected is_err={is_err}, got {res:?}"
        );
    }

    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("b", None), vd("c", None), vd("a", None)]),
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("b", None), vd("c", None)]);
        "variable defs"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("z", "B", ""), nfp("x", "C", ""), nfp("y", "A", "")]),
        DedupArray::Nfp(&mut vec![nfp("y", "A", ""), nfp("z", "B", ""), nfp("x", "C", "")]);
        "named file providers"
    )]
    #[test]
    fn dedup_array_sorts_by_the_correct_key(mut arr: DedupArray<'_>, expected: DedupArray<'_>) {
        arr.sort();
        assert_eq!(arr, expected);
    }

    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("b", None)]),
        false;
        "vd no duplicates"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", ""), nfp("b", "B", "")]),
        false;
        "nfp no duplicates"
    )]
    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("a", None)]),
        false;
        "vd single duplicate"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", ""), nfp("a", "A", "")]),
        false;
        "nfp single duplicate"
    )]
    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("a", None), vd("a", None)]),
        true;
        "vd multiple duplicates"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", ""), nfp("a", "A", ""), nfp("a", "A", "")]),
        true;
        "nfp multiple duplicates"
    )]
    #[test]
    fn at_most_two_duplicates_errors_correctly(arr: DedupArray<'_>, is_err: bool) {
        let res = arr.at_most_two_duplicates("BASE", "path");

        assert_eq!(
            res.is_err(),
            is_err,
            "expected is_err={is_err}, got {res:?}"
        );
    }

    #[test_case(
        DedupArray::VariableDef(&mut vec![vd("a", Some(42.into())), vd("b", None), vd("a", None)]),
        DedupArray::VariableDef(&mut vec![vd("a", None), vd("b", None)]);
        "variable defs"
    )]
    #[test_case(
        DedupArray::Nfp(&mut vec![nfp("a", "A", "original"), nfp("b", "B", ""), nfp("a", "A", "override")]),
        DedupArray::Nfp(&mut vec![nfp("a", "A", "override"), nfp("b", "B", "")]);
        "named file providers"
    )]
    #[test]
    fn dedup_and_sort_keeps_the_second_element_with_a_given_key(
        mut arr: DedupArray<'_>,
        expected: DedupArray<'_>,
    ) {
        arr.dedup_and_sort();
        assert_eq!(arr, expected);
    }

    #[derive(Debug)]
    struct DedupMe {
        variables: Vec<VariableDefinition>,
        providers: Vec<NamedFileProvider>,
    }

    impl CheckArrayDuplicates for DedupMe {
        const BASE_PATH: &str = "base";

        fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
            vec![
                ("variables", DedupArray::VariableDef(&mut self.variables)),
                ("providers", DedupArray::Nfp(&mut self.providers)),
            ]
        }
    }

    #[test]
    fn ensure_no_duplicate_keys_runs_for_all_arrays() {
        let mut dedup_me = DedupMe {
            variables: vec![vd("a", Some(42.into())), vd("a", None)],
            providers: vec![nfp("b", "B", ""), nfp("b", "B", "")],
        };

        let res = dedup_me.ensure_no_duplicate_keys();
        assert!(res.is_err(), "expected to error");

        let errs = res.unwrap_err().into_vec();
        assert_eq!(errs.len(), 2, "expected 2 errors, got {errs:?}");
    }

    #[test]
    fn sort_arrays_runs_for_all_arrays() {
        let mut dedup_me = DedupMe {
            variables: vec![vd("b", None), vd("a", None)],
            providers: vec![nfp("b", "B", ""), nfp("a", "A", "")],
        };

        dedup_me.sort_arrays();

        assert_eq!(&dedup_me.variables, &[vd("a", None), vd("b", None)]);
        assert_eq!(&dedup_me.providers, &[nfp("a", "A", ""), nfp("b", "B", "")]);
    }

    #[test]
    fn dedup_array_try_dedup_and_sort_doesnt_sort_when_returning_errors() {
        let mut dedup_me = DedupMe {
            variables: vec![vd("b", None), vd("b", None), vd("b", None), vd("a", None)],
            providers: vec![
                nfp("b", "B", ""),
                nfp("b", "B", ""),
                nfp("b", "B", ""),
                nfp("a", "A", ""),
            ],
        };

        let res = dedup_me.try_dedup_and_sort();

        assert!(res.is_err(), "expected to error");
        let errs = res.unwrap_err().into_vec();
        assert_eq!(errs.len(), 2, "expected 2 errors, got {errs:?}");

        assert_eq!(
            &dedup_me.variables,
            &[vd("b", None), vd("b", None), vd("b", None), vd("a", None)]
        );
        assert_eq!(
            &dedup_me.providers,
            &[
                nfp("b", "B", ""),
                nfp("b", "B", ""),
                nfp("b", "B", ""),
                nfp("a", "A", "")
            ]
        );
    }

    #[test]
    fn dedup_array_try_dedup_and_sort_runs_for_all_arrays() {
        let mut dedup_me = DedupMe {
            variables: vec![vd("b", Some(42.into())), vd("b", None), vd("a", None)],
            providers: vec![
                nfp("b", "B", "original"),
                nfp("b", "B", "override"),
                nfp("a", "A", ""),
            ],
        };

        let res = dedup_me.try_dedup_and_sort();
        assert!(res.is_ok(), "expected OK, got {res:?}");

        assert_eq!(&dedup_me.variables, &[vd("a", None), vd("b", None)]);
        assert_eq!(
            &dedup_me.providers,
            &[nfp("a", "A", ""), nfp("b", "B", "override"),]
        );
    }
}

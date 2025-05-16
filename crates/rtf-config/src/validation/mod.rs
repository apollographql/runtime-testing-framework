//! Helpers for validating config files
use std::collections::HashSet;

pub mod error;

pub use error::{Error, ErrorBuilder, ErrorKind, Errors, Result};

/// Determine if there are any duplicates within a given slices of elements using a given key
/// function.
///
/// See the tests in the validation module for example usage.
pub(crate) fn duplicate_keys<'a, T: 'a>(
    elems: impl Iterator<Item = &'a T>,
    key_fn: impl Fn(&T) -> &str,
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

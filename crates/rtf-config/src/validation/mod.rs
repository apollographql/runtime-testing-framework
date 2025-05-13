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

    struct S(&'static str);

    #[test]
    fn duplicate_keys_returns_empty_vec_for_no_duplicates() {
        let vals = [S("a"), S("c"), S("b"), S("d"), S("x")];
        let duplicates = duplicate_keys(vals.iter(), |s| s.0);

        assert!(
            duplicates.is_empty(),
            "expected no duplicates, got {duplicates:?}"
        );
    }

    #[test]
    fn duplicate_keys_returns_sorted_results() {
        let vals = [S("c"), S("c"), S("a"), S("a"), S("b")];
        let duplicates = duplicate_keys(vals.iter(), |s| s.0);

        assert_eq!(duplicates, vec!["a", "c"]);
    }

    #[test]
    fn duplicate_keys_returns_unique_results() {
        let vals = [S("a"), S("c"), S("c"), S("c"), S("c")];
        let duplicates = duplicate_keys(vals.iter(), |s| s.0);

        assert_eq!(duplicates, vec!["c"]);
    }
}

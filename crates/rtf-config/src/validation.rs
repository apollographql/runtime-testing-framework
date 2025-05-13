//! Helpers for validating config files
use std::collections::HashSet;

/// Determine if there are any duplicates within a given slices of elements using a given key
/// function.
///
/// For example, checking if a list of file providers contains duplicate names.
pub(crate) fn duplicate_keys<T>(elems: &[T], key_fn: impl Fn(&T) -> &str) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();

    for elem in elems.iter() {
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

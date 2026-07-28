use crate::status::Status;
use rtf_config::StableSource;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod trigger;

pub use trigger::{GitHubPayload, PreparedPayload, TriggerPayload};

#[derive(Debug, Deserialize, Serialize)]
pub struct GenerateUploadUrlsPayload {}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetStatusPayload {
    pub status: Status,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub exit_code: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKey {
    pub src: StableSource,
    pub k: String,
    pub index: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKeyedArrayMap<T> {
    pub keys: Vec<SourceKey>,
    pub data: Vec<T>,
}

impl<T> SourceKeyedArrayMap<T> {
    pub fn empty() -> Self {
        Self {
            keys: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Look up a value by its `(StableSource, key)` pair.
    pub fn get(&self, src: StableSource, key: &str) -> Option<&T> {
        self.keys
            .iter()
            .find(|sk| sk.src == src && sk.k == key)
            .map(|sk| &self.data[sk.index])
    }

    /// Return true if any entry under `stable_src` has a key with `prefix/` as a path prefix.
    ///
    /// Used to distinguish an unknown path (error) from an occupied directory.
    pub fn has_path_prefix(&self, stable_src: &StableSource, prefix: &str) -> bool {
        let prefix_with_sep = format!("{prefix}/");
        self.keys
            .iter()
            .any(|sk| &sk.src == stable_src && sk.k.starts_with(&prefix_with_sep))
    }
}

impl<T> SourceKeyedArrayMap<T>
where
    T: PartialEq,
{
    pub fn from_data(raw: HashMap<(StableSource, String), T>) -> Self {
        let mut keys = Vec::with_capacity(raw.len());
        let mut data = Vec::with_capacity(raw.len());

        let mut raw: Vec<_> = raw.into_iter().collect();
        raw.sort_unstable_by_key(|(k, _)| k.clone());

        for ((src, k), t) in raw.into_iter() {
            let index = match data.iter().position(|known| known == &t) {
                Some(i) => i,
                None => {
                    let i = data.len();
                    data.push(t);
                    i
                }
            };

            keys.push(SourceKey { src, k, index });
        }

        Self { keys, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtf_config::StableSource;

    #[test]
    fn source_keyed_array_map_from_data_deduplicates_equal_values() {
        let raw = [
            (
                (StableSource::Scenario, "a".to_string()),
                "content".to_string(),
            ),
            (
                (StableSource::Environment, "b".to_string()),
                "content".to_string(),
            ),
        ]
        .into();

        let result = SourceKeyedArrayMap::from_data(raw);

        assert_eq!(result.data.len(), 1);
        assert_eq!(result.keys[0].index, 0);
        assert_eq!(result.keys[1].index, 0);
    }

    #[test]
    fn source_keyed_array_map_from_data_distinct_values_are_separate_entries() {
        let raw = [
            ((StableSource::Scenario, "a".to_string()), "foo".to_string()),
            (
                (StableSource::Environment, "b".to_string()),
                "bar".to_string(),
            ),
        ]
        .into();

        let result = SourceKeyedArrayMap::from_data(raw);

        assert_eq!(result.data.len(), 2);
    }
}

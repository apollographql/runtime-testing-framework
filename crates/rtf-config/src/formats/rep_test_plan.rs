use crate::{
    StableSource,
    formats::{CustomProviderDefinition, TestPlanConfig},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize)]
pub struct RepTestPlan {
    pub test_plan: TestPlanConfig,
    pub relative_files: SourceKeyedArrayMap<String>,
    pub custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKey {
    pub src: StableSource,
    pub k: String,
    pub index: usize,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceKeyedArrayMap<T> {
    pub keys: Vec<SourceKey>,
    pub data: Vec<T>,
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
    use crate::StableSource;

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

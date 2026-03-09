use anyhow::bail;
use rtf_config::{
    StableSource,
    formats::{CustomProviderDefinition, TestPlanConfig},
    templating::CustomProviderDefinitions,
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

    pub fn into_map(self) -> SourceKeyedMap<T> {
        SourceKeyedMap {
            keys: self
                .keys
                .into_iter()
                .map(|SourceKey { src, k, index }| ((src, k), index))
                .collect(),
            data: self.data,
        }
    }
}

impl SourceKeyedArrayMap<CustomProviderDefinition> {
    pub fn try_into_custom_provider_definitions(self) -> anyhow::Result<CustomProviderDefinitions> {
        let mut cpd = CustomProviderDefinitions::default();
        for SourceKey { src, k, index } in self.keys.into_iter() {
            match src {
                StableSource::TestPlan => cpd.test_plan.insert(k, self.data[index].clone()),
                StableSource::Environment => cpd.environment.insert(k, self.data[index].clone()),
                StableSource::Scenario => cpd.scenario.insert(k, self.data[index].clone()),
                src => bail!("Invalid REP Test Plan: unexpected custom provider source: {src:?}"),
            };
        }

        Ok(cpd)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SourceKeyedMap<T> {
    keys: HashMap<(StableSource, String), usize>,
    data: Vec<T>,
}

impl<T> SourceKeyedMap<T> {
    // TODO: this sig can be better / smarter
    pub fn get(&self, source: StableSource, key: &str) -> Option<&T> {
        let i = self.keys.get(&(source, key.to_string()))?;

        self.data.get(*i)
    }
}

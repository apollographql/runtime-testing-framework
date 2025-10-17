use crate::templating::{self, Scalar};
use itertools::Itertools;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, mem};

/// A matrix of user provided value dimensions that is expanded out into multiple value sets for
/// templating the test plan containing the matrix.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Matrix {
    #[serde(default)]
    pub variant_names: Option<String>,
    pub dimensions: HashMap<String, Vec<Scalar>>,
}

impl Matrix {
    pub fn is_empty(&self) -> bool {
        self.dimensions.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.dimensions.keys()
    }

    pub fn clear(&mut self) {
        self.dimensions.clear();
    }

    /// Ensure that we have a consistent ordering for the vec we return.
    /// The choice of ordering by the map key here is arbitrary but it is easy to document and
    /// quickly check by hand for users when needed.
    pub fn sorted_dimensions(&self) -> Vec<(&String, &Vec<Scalar>)> {
        let mut pairs: Vec<_> = self.dimensions.iter().collect();
        pairs.sort_unstable_by(|(k1, _), (k2, _)| k1.cmp(k2));

        pairs
    }

    pub fn n_variants(&self) -> usize {
        self.dimensions
            .iter()
            .map(|(k, vals)| vals.iter().map(|v| (k.clone(), v.clone())))
            .multi_cartesian_product()
            .count()
    }

    /// We expand out matrix values as a cartesean product over all possible sets of values we can
    /// obtain when combined with any scalar values we have.
    pub fn expand(
        &self,
        values: &HashMap<String, Scalar>,
    ) -> Vec<(String, HashMap<String, Scalar>)> {
        if self.is_empty() {
            return vec![(variant_name(None, values, 0), values.clone())];
        }

        self.sorted_dimensions()
            .into_iter()
            .map(|(k, vals)| vals.iter().map(|v| (k.clone(), v.clone())))
            .multi_cartesian_product()
            .enumerate()
            .map(|(n, matrix_vals)| {
                let mut values = values.clone();
                values.extend(matrix_vals);
                let name = variant_name(self.variant_names.as_deref(), &values, n);

                (name, values)
            })
            .collect()
    }

    /// Check that all matrix arrays are non-empty and homogeneous
    pub fn check_dimensions(&self, errs: &mut templating::ErrorBuilder) {
        for (k, vals) in self.dimensions.iter() {
            let discriminant = match vals.first() {
                Some(val) => mem::discriminant(val),
                None => {
                    errs.push(templating::ErrorKind::EmptyMatrixValue, k, &[]);
                    continue;
                }
            };

            if !vals.iter().all(|v| mem::discriminant(v) == discriminant) {
                errs.push(templating::ErrorKind::InconsistentMatrixValue, k, &[]);
            }
        }
    }

    /// Check if we have any conflicts between matrix values and scalar values
    pub fn check_conflicting_keys(
        &self,
        values: &HashMap<String, Scalar>,
        errs: &mut templating::ErrorBuilder,
    ) {
        let mut conflicting_keys: Vec<String> = values
            .keys()
            .filter(|k| self.dimensions.contains_key(*k))
            .cloned()
            .collect();

        if !conflicting_keys.is_empty() {
            conflicting_keys.sort_unstable(); // ensure consistent ordering
            errs.push(
                templating::ErrorKind::ConflictingValues,
                conflicting_keys.join(", "),
                &[],
            )
        }
    }
}

fn variant_name(template: Option<&str>, values: &HashMap<String, Scalar>, n: usize) -> String {
    let mut s = match template {
        Some(s) => s.to_string(),
        None => return format!("matrix_variant_{}", n + 1),
    };

    for (k, v) in values.iter() {
        // ${k} is what we are replacing but we need to escape the curlies
        s = s.replace(&format!("${{{k}}}"), &v.to_string());
    }

    s
}

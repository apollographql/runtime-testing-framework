use crate::{
    checks::duplicate_keys,
    formats::{Error, Result},
    templating::{self, Scalar},
};
use itertools::Itertools;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    cmp::max,
    collections::HashMap,
    mem::{self, Discriminant},
    sync::LazyLock,
};

// Used to extract "unknown_val" from "...${unknown_val}..."
static RE_UNKNOWN_VAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\$\{(.*?)\}"#).expect("valid regex"));

/// A matrix of user provided value dimensions that is expanded out into multiple value sets for
/// templating the test plan containing the matrix.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Matrix {
    /// Optional template string for customising the names of each variants output subdirectory.
    #[serde(default)]
    pub variant_names: Option<String>,

    /// The actual dimensions and their allowed values that will be used to produce the matrix
    pub dimensions: HashMap<String, Vec<Scalar>>,

    /// Additional _groups_ of values that will be combined with known dimensions to produce the
    /// full set of values for each dimension.
    ///
    /// Used for tying subsets of values together and reducing the number of variants we expand to.
    /// Each entry within this array is required to define the same keys and scalar value types.
    #[serde(default)]
    pub include: Vec<HashMap<String, Scalar>>,
}

impl Matrix {
    pub fn is_empty(&self) -> bool {
        self.dimensions.is_empty() && self.include.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.dimensions
            .keys()
            .chain(self.include.iter().take(1).flat_map(|m| m.keys()))
    }

    pub fn clear(&mut self) {
        self.dimensions.clear();
        self.include.clear();
    }

    pub fn n_variants(&self) -> usize {
        self.dimensions
            .iter()
            .fold(1, |n, (_, vals)| n * vals.len())
            * max(1, self.include.len())
    }

    /// Ensure that we have a consistent ordering for the vec we return.
    /// The choice of ordering by the map key here is arbitrary but it is easy to document and
    /// quickly check by hand for users when needed.
    pub fn sorted_dimensions(&self) -> Vec<(&String, &Vec<Scalar>)> {
        let mut pairs: Vec<_> = self.dimensions.iter().collect();
        pairs.sort_unstable_by(|(k1, _), (k2, _)| k1.cmp(k2));

        pairs
    }

    /// We expand out matrix values as a cartesean product over all possible sets of values we can
    /// obtain when combined with any scalar values we have.
    ///
    /// When generating variant names from a user provided template we validate the template itself
    /// and also ensure that the generated names are unique.
    pub fn try_expand(
        &self,
        values: &HashMap<String, Scalar>,
    ) -> Result<Vec<(String, HashMap<String, Scalar>)>> {
        // First, we construct an iterator over the cartesian product of the (sorted) dimesions.
        // Note, if `self.dimesions` is empty, the iterator returned by `multi_cartesian_product`
        // will yield exactly one item, which will be an empty vector.
        let dims = self
            .sorted_dimensions()
            .into_iter()
            .map(|(k, vals)| vals.iter().map(|v| (k.clone(), v.clone())))
            .multi_cartesian_product();

        // Next, for each entry in the product, we want to yield a set of sets to `include`. If
        // `self.include` is empty, we should, at minimum, yield a set a singular empty set.
        let mut it = self.include.clone().into_iter();
        let include = std::iter::once(it.next().unwrap_or_default()).chain(it);

        // Lastly, we stitch these together and validate
        let digest = dims.flat_map(|matrix_vals| {
            let mut values = values.clone();
            values.extend(matrix_vals);

            include.clone().map(move |mut include| {
                include.extend(values.clone());
                include
            })
        });

        self.name_and_validate(digest)
    }

    fn name_and_validate<I>(&self, it: I) -> Result<Vec<(String, HashMap<String, Scalar>)>>
    where
        I: Iterator<Item = HashMap<String, Scalar>>,
    {
        let variants: Vec<_> = it
            .enumerate()
            .map(|(i, m)| variant_name(self.variant_names.as_deref(), &m, i).map(|name| (name, m)))
            .collect::<Result<_>>()?;

        let duplicates = duplicate_keys(variants.iter().map(|(name, _)| name.as_str()), |s| s);
        if !duplicates.is_empty() {
            let duplicates: Vec<String> = duplicates.into_iter().map(String::from).collect();
            return Err(Error::NonUniqueMatrixVariantNames { duplicates });
        }

        Ok(variants)
    }

    /// Check that all matrix arrays are non-empty and homogeneous, and that all include maps share
    /// the same keys and types.
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

        if self.include.len() <= 1 {
            return;
        }

        let expected: HashMap<&String, Discriminant<Scalar>> = self.include[0]
            .iter()
            .map(|(k, v)| (k, mem::discriminant(v)))
            .collect();

        for map in self.include.iter().skip(1) {
            let key_types: HashMap<&String, Discriminant<Scalar>> =
                map.iter().map(|(k, v)| (k, mem::discriminant(v))).collect();
            if key_types != expected {
                errs.push(
                    templating::ErrorKind::InconsistentMatrixInclude,
                    "matrix include maps must share consistent keys and types",
                    &[],
                );
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
            .filter(|k| {
                self.dimensions.contains_key(*k)
                    || self
                        .include
                        .first()
                        .map(|m| m.contains_key(*k))
                        .unwrap_or(false)
            })
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

fn variant_name(
    template: Option<&str>,
    values: &HashMap<String, Scalar>,
    n: usize,
) -> Result<String> {
    let mut s = match template {
        Some(s) => s.to_string(),
        None => return Ok(format!("matrix_variant_{}", n + 1)),
    };

    for (k, v) in values.iter() {
        // ${k} is what we are replacing but we need to escape the curlies
        s = s.replace(&format!("${{{k}}}"), &v.to_string());
    }

    // Ensure that all template patterns have been filled
    let remaining_template_vals: Vec<_> = RE_UNKNOWN_VAL
        .captures_iter(&s)
        .map(|cap| {
            let (_, [val]) = cap.extract();
            val.to_string()
        })
        .collect();

    if !remaining_template_vals.is_empty() {
        return Err(Error::UnknownMatrixVariantTemplateValues {
            values: remaining_template_vals,
        });
    }

    Ok(slugify(&s))
}

// Replace whitespace and path separators with underscores
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            c if c.is_whitespace() => '_',
            '/' | '\\' => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    fn dims<T>(dimensions: &[(&str, &[T])]) -> HashMap<String, Vec<Scalar>>
    where
        T: Copy + Into<Scalar>,
    {
        dimensions
            .iter()
            .map(|(name, vals)| (name.to_string(), vals.iter().map(|t| (*t).into()).collect()))
            .collect()
    }

    #[test]
    fn sorted_dimensions_orders_by_key() {
        let m = Matrix {
            variant_names: None,
            dimensions: dims(&[
                ("z", &[1, 2]),
                ("a", &[3, 4]),
                ("b", &[5, 6]),
                ("C", &[7, 8]),
            ]),
            include: Vec::new(),
        };

        let sorted_keys: Vec<_> = m
            .sorted_dimensions()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();

        // we're sorting ascii-betical so uppercase comes first
        assert_eq!(&sorted_keys, &["C", "a", "b", "z"]);
    }

    #[test_case(&[("a", &[1, 2, 3])], 3; "1x3")]
    #[test_case(&[("a", &[1]), ("b", &[2]), ("c", &[3])], 1; "3x1")]
    #[test_case(&[("a", &[1, 2]), ("b", &[3, 4])], 4; "2x2")]
    #[test_case(&[("a", &[1, 2]), ("b", &[3, 4, 5]), ("c", &[6, 7])], 12; "mixed")]
    #[test]
    fn n_variants_returns_the_correct_value(dimensions: &[(&str, &[usize])], expected: usize) {
        for n_include in 0..3 {
            let m = Matrix {
                variant_names: None,
                dimensions: dims(dimensions),
                include: vec![HashMap::new(); n_include],
            };

            assert_eq!(
                m.n_variants(),
                expected * max(1, n_include),
                "n_include={n_include}"
            );
        }
    }

    fn value_map(vals: &[(&str, &str)]) -> HashMap<String, Scalar> {
        vals.iter()
            .map(|&(name, val)| (name.to_string(), Scalar::String(val.to_string())))
            .collect()
    }

    #[test_case(
        dims(&[("a", &["X", "Y"]), ("b", &["1", "2"])]),
        Vec::new(),
        &[
            value_map(&[("a", "X"), ("b", "1"), ("Z", "Z")]),
            value_map(&[("a", "X"), ("b", "2"), ("Z", "Z")]),
            value_map(&[("a", "Y"), ("b", "1"), ("Z", "Z")]),
            value_map(&[("a", "Y"), ("b", "2"), ("Z", "Z")])
        ];
        "only dimensions"
    )]
    #[test_case(
        HashMap::new(),
        vec![
            value_map(&[("b", "1"), ("c", "2")]),
            value_map(&[("b", "3"), ("c", "4")])
        ],
        &[
            value_map(&[("b", "1"), ("c", "2"), ("Z", "Z")]),
            value_map(&[("b", "3"), ("c", "4"), ("Z", "Z")])
        ];
        "only include"
    )]
    #[test_case(
        dims(&[("a", &["X", "Y"])]),
        vec![
            value_map(&[("b", "1"), ("c", "2")]),
            value_map(&[("b", "3"), ("c", "4")])
        ],
        &[
            value_map(&[("a", "X"), ("b", "1"), ("c", "2"), ("Z", "Z")]),
            value_map(&[("a", "X"), ("b", "3"), ("c", "4"), ("Z", "Z")]),
            value_map(&[("a", "Y"), ("b", "1"), ("c", "2"), ("Z", "Z")]),
            value_map(&[("a", "Y"), ("b", "3"), ("c", "4"), ("Z", "Z")]),
        ];
        "dimensions and include"
    )]
    #[test_case(
        HashMap::new(),
        Vec::new(),
        &[value_map(&[("Z", "Z")])];
        "no dimensions or include"
    )]
    #[test]
    fn try_expand_generates_the_expected_values(
        dimensions: HashMap<String, Vec<Scalar>>,
        include: Vec<HashMap<String, Scalar>>,
        expected: &[HashMap<String, Scalar>],
    ) {
        let m = Matrix {
            variant_names: None,
            dimensions,
            include,
        };

        let expanded: Vec<_> = m
            .try_expand(&value_map(&[("Z", "Z")]))
            .unwrap()
            .into_iter()
            .map(|(_, vals)| vals)
            .collect();

        assert_eq!(expanded, expected);
    }

    #[test_case(
        None,
        &["matrix_variant_1", "matrix_variant_2", "matrix_variant_3", "matrix_variant_4"];
        "no template"
    )]
    #[test_case(Some("${a}_${b}"), &["X_1", "X_2", "Y_1", "Y_2"]; "valid template")]
    #[test_case(Some("${a}@${b}"), &["X@1", "X@2", "Y@1", "Y@2"]; "template with non-ascii")]
    #[test_case(Some("${a}/${b}"), &["X_1", "X_2", "Y_1", "Y_2"]; "template with forward slash")]
    #[test_case(Some("${a}\\${b}"), &["X_1", "X_2", "Y_1", "Y_2"]; "template with back slash")]
    #[test_case(Some("${a} \t${b}"), &["X__1", "X__2", "Y__1", "Y__2"]; "template with whitespace")]
    #[test_case(Some("${a}_${b}_${c}"), &["X_1_Z", "X_2_Z", "Y_1_Z", "Y_2_Z"]; "valid template using include value")]
    #[test]
    fn try_expand_generates_the_expected_variant_names(
        variant_names: Option<&str>,
        expected: &[&str],
    ) {
        let mut m = HashMap::new();
        m.insert("c".to_string(), Scalar::from("Z"));

        let m = Matrix {
            variant_names: variant_names.map(String::from),
            dimensions: dims(&[("a", &["X", "Y"]), ("b", &["1", "2"])]),
            include: vec![m],
        };

        let expanded = m
            .try_expand(&Default::default())
            .expect("expansion to succeed");
        let names: Vec<_> = expanded.iter().map(|(name, _)| name.as_str()).collect();

        assert_eq!(names, expected);
    }

    #[test_case("${a}-${b}-${c}", &["c"]; "single unknown value with known")]
    #[test_case("${c}", &["c"]; "single unknown value")]
    #[test_case("${x}-${c}", &["x", "c"]; "multiple unknown values")]
    #[test]
    fn try_expand_errors_on_unknown_template_values(variant_names: &str, expected: &[&str]) {
        let m = Matrix {
            variant_names: Some(variant_names.into()),
            dimensions: dims(&[("a", &["X", "Y"]), ("b", &["1", "2"])]),
            include: Vec::new(),
        };

        match m.try_expand(&Default::default()) {
            Err(Error::UnknownMatrixVariantTemplateValues { values }) => {
                assert_eq!(values, expected)
            }

            Ok(ok) => panic!("expected error, got {ok:?}"),
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn try_expand_errors_on_duplicate_variant_names() {
        let m = Matrix {
            variant_names: Some("${a}".into()),
            dimensions: dims(&[("a", &["X", "Y"]), ("b", &["1", "2"])]),
            include: Vec::new(),
        };

        match m.try_expand(&Default::default()) {
            Err(Error::NonUniqueMatrixVariantNames { duplicates }) => {
                assert_eq!(duplicates, &["X", "Y"])
            }

            Ok(ok) => panic!("expected error, got {ok:?}"),
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}

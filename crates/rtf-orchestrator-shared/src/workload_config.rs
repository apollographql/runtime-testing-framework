//! Configuration applied to the workload cluster a run executes in.
//!
//! A [WorkloadConfig] is defined per pool and a known test plan can override individual values
//! through a [WorkloadConfigPatch].
use serde::{Deserialize, Serialize};

/// The maximum length of a k8s label value.
const MAX_LABEL_VALUE_LEN: usize = 63;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadConfig {
    /// Grant scenarios elevated k8s write permissions within their namespace.
    #[serde(default)]
    pub allow_k8s_write: bool,
    /// `(label value, weight)` pairs describing how to split the nodes of a dedicated cluster
    /// between labels. Empty means no node labelling.
    #[serde(default)]
    pub node_label_weights: Vec<(String, u32)>,
}

impl WorkloadConfig {
    pub fn patched(mut self, patch: &WorkloadConfigPatch) -> Self {
        if let Some(allow_k8s_write) = patch.allow_k8s_write {
            self.allow_k8s_write = allow_k8s_write;
        }
        if let Some(weights) = &patch.node_label_weights {
            self.node_label_weights = weights.clone();
        }

        self
    }

    pub fn validate(&self) -> Result<(), Error> {
        validate_node_label_weights(&self.node_label_weights)
    }
}

/// A set of overrides to a [WorkloadConfig]: a field that is `None` leaves the underlying value
/// unchanged.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_k8s_write: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_label_weights: Option<Vec<(String, u32)>>,
}

impl WorkloadConfigPatch {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn sets_node_labels(&self) -> bool {
        self.node_label_weights
            .as_ref()
            .is_some_and(|weights| !weights.is_empty())
    }

    pub fn validate(
        &self,
        pool: &str,
        is_pinned: bool,
        requires_dedicated: bool,
        supports_dedicated: impl Fn(&str) -> bool,
    ) -> Result<(), Error> {
        if requires_dedicated && !supports_dedicated(pool) {
            return Err(Error::DedicatedClusterNotSupported {
                pool: pool.to_string(),
            });
        }

        if self.sets_node_labels() {
            if !requires_dedicated {
                return Err(Error::NodeLabelsRequireDedicatedCluster);
            }
            if !is_pinned {
                return Err(Error::NodeLabelsRequirePinnedPool);
            }
        }

        if let Some(weights) = &self.node_label_weights {
            validate_node_label_weights(weights)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("node label weights must not all be zero")]
    ZeroWeights,

    #[error("this test plan requires a dedicated cluster but pool {pool} does not support them")]
    DedicatedClusterNotSupported { pool: String },

    #[error("node label value '{0}' appears more than once")]
    DuplicateLabelValue(String),

    #[error(
        "'{0}' is not a valid node label value: it must be 1-63 characters, start and end with \
         an alphanumeric character and contain only alphanumerics, '-', '_' or '.'"
    )]
    InvalidLabelValue(String),

    #[error("node labels can only be set for a test plan that requires a dedicated cluster")]
    NodeLabelsRequireDedicatedCluster,

    #[error("node labels can only be set for a test plan that is pinned to pool")]
    NodeLabelsRequirePinnedPool,
}

fn validate_node_label_weights(weights: &[(String, u32)]) -> Result<(), Error> {
    if weights.is_empty() {
        return Ok(());
    }

    if weights.iter().all(|(_, weight)| *weight == 0) {
        return Err(Error::ZeroWeights);
    }

    for (i, (value, _)) in weights.iter().enumerate() {
        if !is_valid_label_value(value) {
            return Err(Error::InvalidLabelValue(value.clone()));
        }
        if weights[..i].iter().any(|(seen, _)| seen == value) {
            return Err(Error::DuplicateLabelValue(value.clone()));
        }
    }

    Ok(())
}

/// Non-empty label values following the k8s syntax rules.
fn is_valid_label_value(value: &str) -> bool {
    let is_alphanumeric = |c: char| c.is_ascii_alphanumeric();

    !value.is_empty()
        && value.len() <= MAX_LABEL_VALUE_LEN
        && value.starts_with(is_alphanumeric)
        && value.ends_with(is_alphanumeric)
        && value
            .chars()
            .all(|c| is_alphanumeric(c) || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use simple_test_case::test_case;

    fn weights(pairs: &[(&str, u32)]) -> Vec<(String, u32)> {
        pairs.iter().map(|(v, w)| (v.to_string(), *w)).collect()
    }

    fn base() -> WorkloadConfig {
        WorkloadConfig {
            allow_k8s_write: true,
            node_label_weights: weights(&[("a", 1)]),
        }
    }

    #[test_case(
        WorkloadConfigPatch::default(),
        base();
        "empty patch leaves the config unchanged"
    )]
    #[test_case(
        WorkloadConfigPatch { allow_k8s_write: Some(false), ..Default::default() },
        WorkloadConfig { allow_k8s_write: false, ..base() };
        "patch can turn a bool off"
    )]
    #[test_case(
        WorkloadConfigPatch { node_label_weights: Some(weights(&[("b", 2), ("c", 3)])), ..Default::default() },
        WorkloadConfig { node_label_weights: weights(&[("b", 2), ("c", 3)]), ..base() };
        "patch replaces weights"
    )]
    #[test_case(
        WorkloadConfigPatch { node_label_weights: Some(Vec::new()), ..Default::default() },
        WorkloadConfig { node_label_weights: Vec::new(), ..base() };
        "an explicitly empty list clears the weights"
    )]
    #[test]
    fn patched(patch: WorkloadConfigPatch, expected: WorkloadConfig) {
        assert_eq!(base().patched(&patch), expected);
    }

    #[test_case(&[], Ok(()); "no weights is valid")]
    #[test_case(&[("a", 1), ("b", 0)], Ok(()); "a zero weight alongside a non-zero one is valid")]
    #[test_case(&[("a", 0), ("b", 0)], Err(Error::ZeroWeights); "all zero")]
    #[test_case(
        &[("a", 1), ("a", 2)],
        Err(Error::DuplicateLabelValue("a".into()));
        "duplicate values"
    )]
    #[test_case(&[("", 1)], Err(Error::InvalidLabelValue("".into())); "empty value")]
    #[test_case(
        &[("-a", 1)],
        Err(Error::InvalidLabelValue("-a".into()));
        "leading punctuation"
    )]
    #[test_case(
        &[("a b", 1)],
        Err(Error::InvalidLabelValue("a b".into()));
        "illegal character"
    )]
    #[test_case(&[("a-b_c.d", 1)], Ok(()); "allowed punctuation in the middle")]
    #[test]
    fn validate_weights(pairs: &[(&str, u32)], expected: Result<(), Error>) {
        let config = WorkloadConfig {
            node_label_weights: weights(pairs),
            ..Default::default()
        };

        assert_eq!(config.validate(), expected);
    }

    #[test]
    fn label_values_are_limited_to_63_characters() {
        let ok = "a".repeat(63);
        let too_long = "a".repeat(64);

        assert!(is_valid_label_value(&ok));
        assert!(!is_valid_label_value(&too_long));
    }

    #[test_case(None, false; "unset")]
    #[test_case(Some(Vec::new()), false; "explicitly empty")]
    #[test_case(Some(weights(&[("a", 1)])), true; "non-empty")]
    #[test]
    fn sets_node_labels(node_label_weights: Option<Vec<(String, u32)>>, expected: bool) {
        let patch = WorkloadConfigPatch {
            node_label_weights,
            ..Default::default()
        };

        assert_eq!(patch.sets_node_labels(), expected);
    }

    #[test]
    fn patch_treats_null_and_absent_fields_as_unset() {
        let patch: WorkloadConfigPatch =
            serde_json::from_str(r#"{"allow_k8s_write": null}"#).unwrap();

        assert!(patch.is_empty());
    }

    #[test]
    fn patch_round_trips_through_json() {
        let patch = WorkloadConfigPatch {
            allow_k8s_write: Some(true),
            node_label_weights: Some(weights(&[("a", 1), ("b", 2)])),
        };
        let value = serde_json::to_value(&patch).unwrap();

        assert_eq!(
            value,
            json!({"allow_k8s_write": true, "node_label_weights": [["a", 1], ["b", 2]]})
        );
        assert_eq!(
            serde_json::from_value::<WorkloadConfigPatch>(value).unwrap(),
            patch
        );
    }
}

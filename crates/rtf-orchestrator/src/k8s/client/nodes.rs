use crate::k8s::client::NODE_LABEL_PREFIX;
use k8s_openapi::api::core::v1::Node;
use kube::ResourceExt;
use std::collections::HashMap;

/// A struct to represent an allocation of node labels to nodes
// Not wired up yet - pending the node-allocation feature work that will call this, only used in
// tests
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug)]
pub(crate) struct NodeAllocationPlan {
    // Map of node ID to the label (key, value) to apply to it
    inner: HashMap<String, (String, String)>,
}

impl NodeAllocationPlan {
    #[cfg_attr(not(test), expect(dead_code))]
    /// Create an allocation, testing whether it's valid throughout
    pub(crate) fn try_new(
        mut nodes: Vec<Node>,
        allocation: HashMap<String, u32>,
    ) -> Result<Self, NodeAllocationError> {
        use NodeAllocationError::{Unsatisfiable, WeightOverflow, ZeroWeights};

        let node_total: u32 = nodes.len() as u32;

        let allocation_total: u32 = allocation.values().sum();

        // If somehow we try to allocate 0 weight then just stop because it will make other
        // calculations nonsensical.
        if allocation_total == 0 {
            return Err(ZeroWeights);
        }

        // Use checked multiply here so we don't fall victim to overflows (or at least we can
        // be alerted when they happen).
        let proposed_node_totals: HashMap<String, u32> = allocation
            .into_iter()
            .map(|(k, v)| {
                v.checked_mul(node_total)
                    .map(|scaled| (k, scaled / allocation_total))
                    .ok_or(WeightOverflow)
            })
            .collect::<Result<_, _>>()?;
        if proposed_node_totals.values().sum::<u32>() != node_total {
            return Err(Unsatisfiable);
        }
        // Transform the allocation into the inner data structure
        let mut inner = HashMap::new();
        for (label, amount) in proposed_node_totals {
            for _ in 0..amount {
                let node = nodes.pop().unwrap();
                inner.insert(
                    node.name_any(),
                    (NODE_LABEL_PREFIX.to_string(), label.clone()),
                );
            }
        }

        Ok(NodeAllocationPlan { inner })
    }
}

impl IntoIterator for NodeAllocationPlan {
    type Item = (String, (String, String));
    type IntoIter = std::collections::hash_map::IntoIter<String, (String, String)>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl From<HashMap<String, (String, String)>> for NodeAllocationPlan {
    fn from(value: HashMap<String, (String, String)>) -> Self {
        Self { inner: value }
    }
}

impl PartialEq for NodeAllocationPlan {
    fn eq(&self, other: &Self) -> bool {
        self.inner.eq(&other.inner)
    }
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum NodeAllocationError {
    #[error("Could not satisfy requested node allocation")]
    Unsatisfiable,
    #[error("Weights supplied equalled zero")]
    ZeroWeights,
    #[error("Allocation weight too large to compute a node split")]
    WeightOverflow,
}

#[cfg(test)]
mod test {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use simple_test_case::test_case;

    fn summary(plan: &NodeAllocationPlan) -> HashMap<String, u32> {
        let mut summary = HashMap::new();
        for (_, label) in plan.inner.values() {
            summary
                .entry(label.clone())
                .and_modify(|counter| *counter += 1)
                .or_insert(1);
        }

        summary
    }

    fn generate_nodes(nodes: Vec<&str>) -> Vec<Node> {
        nodes
            .into_iter()
            .map(|name| Node {
                metadata: ObjectMeta {
                    name: Some(name.to_string()),
                    ..Default::default()
                },
                spec: None,
                status: None,
            })
            .collect()
    }

    fn allocation_map(weights: &[(&str, u32)]) -> HashMap<String, u32> {
        weights.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test_case(
        generate_nodes(vec!["node-1", "node-2", "node-3"]),
        &[("a", 1), ("b", 1), ("c", 1)],
        &[("a", 1), ("b", 1), ("c", 1)];
        "three way split"
    )]
    #[test_case(
        generate_nodes(vec!["node-1", "node-2", "node-3"]),
        &[("a", 1), ("b", 2)],
        &[("a", 1), ("b", 2)];
        "lop-sided split"
    )]
    #[test_case(
        generate_nodes(vec!["node-1", "node-2", "node-3", "node-4"]),
        &[("only", 1)],
        &[("only", 4)];
        "single bucket takes all nodes"
    )]
    #[test]
    fn test_generates_correct_node_split(
        nodes: Vec<Node>,
        allocation: &[(&str, u32)],
        expected_summary: &[(&str, u32)],
    ) {
        let actual = NodeAllocationPlan::try_new(nodes, allocation_map(allocation)).unwrap();
        let proportions = summary(&actual);
        assert_eq!(proportions, allocation_map(expected_summary));
    }

    #[test_case(
        generate_nodes(vec![
            "node-1", "node-2", "node-3", "node-4", "node-5",
            "node-6", "node-7", "node-8", "node-9", "node-10",
        ]),
        &[("a", 1), ("b", 3)],
        NodeAllocationError::Unsatisfiable;
        "ratio not exactly divisible by node count"
    )]
    #[test_case(
        generate_nodes(vec!["node-1", "node-2", "node-3"]),
        &[],
        NodeAllocationError::ZeroWeights;
        "no allocation weights supplied"
    )]
    #[test_case(
        generate_nodes(vec!["node-1", "node-2", "node-3"]),
        &[("a", 0), ("b", 0)],
        NodeAllocationError::ZeroWeights;
        "all weights are zero"
    )]
    #[test_case(
        generate_nodes(vec!["node-1", "node-2", "node-3"]),
        &[("a", u32::MAX)],
        NodeAllocationError::WeightOverflow;
        "weight too large to scale against node count"
    )]
    #[test]
    fn test_returns_expected_error(
        nodes: Vec<Node>,
        allocation: &[(&str, u32)],
        expected_error: NodeAllocationError,
    ) {
        let actual = NodeAllocationPlan::try_new(nodes, allocation_map(allocation));
        assert_eq!(actual, Err(expected_error));
    }
}

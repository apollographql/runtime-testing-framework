use rtf_orchestrator_shared::{
    cluster_summary::ClusterSummaryResponse,
    known_test_plan::{KnownTestPlanSummary, Role},
};

#[derive(Debug)]
pub struct PoolOptionView {
    pub name: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug)]
pub struct TestPlanEditView {
    pub is_admin: bool,
    pub pools: Vec<PoolOptionView>,
    pub pools_error: Option<String>,
    pub owners: String,
    pub trigger_users: String,
    pub node_label_weights: String,
    pub error: Option<String>,
}

impl TestPlanEditView {
    pub fn new(
        plan: &KnownTestPlanSummary,
        is_admin: bool,
        pools: Option<Result<ClusterSummaryResponse, String>>,
        error: Option<String>,
    ) -> Self {
        let pinned = plan.pinned_workload_pool.as_deref();
        let (mut pools, pools_error) = match pools {
            Some(Ok(summary)) => (
                summary
                    .pools
                    .into_iter()
                    .map(|pool| PoolOptionView {
                        label: if pool.supports_dedicated {
                            format!("{} (supports dedicated clusters)", pool.name)
                        } else {
                            pool.name.clone()
                        },
                        selected: Some(pool.name.as_str()) == pinned,
                        name: pool.name,
                    })
                    .collect(),
                None,
            ),
            Some(Err(error)) => (Vec::new(), Some(error)),
            None => (Vec::new(), None),
        };

        if let Some(pinned) = pinned
            && !pools.iter().any(|pool| pool.selected)
        {
            pools.push(PoolOptionView {
                name: pinned.to_owned(),
                label: pinned.to_owned(),
                selected: true,
            });
        }

        let users_with = |role: Role| {
            plan.associated_users
                .iter()
                .filter(|(_, r)| **r == role)
                .map(|(email, _)| email.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };

        Self {
            is_admin,
            pools,
            pools_error,
            owners: users_with(Role::Owner),
            trigger_users: users_with(Role::CanTrigger),
            node_label_weights: plan
                .workload_config_patch
                .node_label_weights
                .iter()
                .flatten()
                .map(|(label, weight)| format!("{label}={weight}"))
                .collect::<Vec<_>>()
                .join("\n"),
            error,
        }
    }
}

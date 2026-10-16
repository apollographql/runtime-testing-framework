use rtf_orchestrator_shared::{
    known_test_plan::{KnownTestPlanSummary, Role},
    whoami::WhoAmIResponse,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerBadge {
    pub label: &'static str,
    pub class: &'static str,
}

impl ViewerBadge {
    pub fn for_whoami(whoami: &WhoAmIResponse) -> Option<Self> {
        match whoami {
            WhoAmIResponse::Admin { .. } => Some(Self {
                label: "Admin",
                class: "status--active",
            }),
            WhoAmIResponse::Automation { .. } => Some(Self {
                label: "Automation",
                class: "status--pending",
            }),
            WhoAmIResponse::User { .. } | WhoAmIResponse::Unknown => None,
        }
    }

    pub fn for_test_plan(whoami: &WhoAmIResponse, plan: &KnownTestPlanSummary) -> Option<Self> {
        Self::for_whoami(whoami).or_else(|| whoami.role_for(plan).map(Self::for_role))
    }

    pub fn for_role(role: Role) -> Self {
        match role {
            Role::Owner => Self {
                label: "Owner",
                class: "status--success",
            },
            Role::CanTrigger => Self {
                label: "Can trigger",
                class: "status--pending",
            },
        }
    }
}

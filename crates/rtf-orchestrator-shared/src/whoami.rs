use crate::known_test_plan::{KnownTestPlanSummary, Role};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WhoAmIResponse {
    Admin {
        email: String,
    },
    User {
        email: String,
    },
    Automation {
        email: String,
        org: String,
        repo: String,
    },
    Unknown,
}

impl WhoAmIResponse {
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Admin { .. })
    }

    pub fn role_for(&self, plan: &KnownTestPlanSummary) -> Option<Role> {
        match self {
            Self::Admin { email } | Self::User { email } => {
                plan.associated_users.get(email).copied()
            }
            Self::Automation { .. } | Self::Unknown => None,
        }
    }

    pub fn can_trigger(&self, plan: &KnownTestPlanSummary) -> bool {
        match self {
            Self::Admin { .. } => true,
            Self::User { email } => {
                !plan.restrict_triggers || plan.associated_users.contains_key(email)
            }
            Self::Automation { .. } => !plan.restrict_automated_triggers,
            Self::Unknown => !plan.restrict_triggers,
        }
    }
}

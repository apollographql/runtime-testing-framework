use rtf_orchestrator_shared::whoami::WhoAmIResponse;

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
}

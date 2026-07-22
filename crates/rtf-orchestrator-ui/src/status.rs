use rep_orchestrator_shared::status::{Status, StatusCategory};

/// CSS modifier class for a status, used to colour-code the UI.
pub fn css_class(status: Status) -> &'static str {
    match status.category() {
        StatusCategory::Pending => "status--pending",
        StatusCategory::Active => "status--active",
        StatusCategory::Success => "status--success",
        StatusCategory::Failed => "status--failed",
        StatusCategory::Unrunnable => "status--unrunnable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_class_matches_the_status_category() {
        let cases = [
            (Status::Initialising, "status--pending"),
            (Status::Resolving, "status--pending"),
            (Status::Provisioning, "status--pending"),
            (Status::EnvironmentReady, "status--active"),
            (Status::Running, "status--active"),
            (Status::Successful, "status--success"),
            (Status::Failed, "status--failed"),
            (Status::Unrunnable, "status--unrunnable"),
        ];

        for (status, expected) in cases {
            assert_eq!(css_class(status), expected, "{status:?}");
        }
    }
}

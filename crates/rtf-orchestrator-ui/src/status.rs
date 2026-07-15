use rep_orchestrator_shared::status::Status;

/// Human-facing label for a status, matching the serde/DB `SCREAMING_SNAKE_CASE` representation.
pub fn label(status: Status) -> &'static str {
    match status {
        Status::Initialising => "INITIALISING",
        Status::Resolving => "RESOLVING",
        Status::Provisioning => "PROVISIONING",
        Status::EnvironmentReady => "ENVIRONMENT_READY",
        Status::Running => "RUNNING",
        Status::Successful => "SUCCESSFUL",
        Status::Failed => "FAILED",
        Status::Unrunnable => "UNRUNNABLE",
    }
}

/// CSS modifier class for a status, used to colour-code the UI.
pub fn css_class(status: Status) -> &'static str {
    match status {
        Status::Initialising | Status::Resolving | Status::Provisioning => "status--pending",
        Status::EnvironmentReady | Status::Running => "status--active",
        Status::Successful => "status--success",
        Status::Failed | Status::Unrunnable => "status--failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_uses_serde_representation_not_display() {
        // The shared `Display` impl prints `ENV_READY`; the UI must show the serde/DB form.
        assert_eq!(label(Status::EnvironmentReady), "ENVIRONMENT_READY");
        assert_ne!(
            label(Status::EnvironmentReady),
            Status::EnvironmentReady.to_string()
        );
    }
}

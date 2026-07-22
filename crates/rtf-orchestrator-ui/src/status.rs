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

/// The exit code to display for a `Successful`/`Failed` entity. The orchestrator only persists an
/// exit code on `Failed` (see `execution_status::post_handler`), even though `Successful` is
/// guaranteed to mean exit code 0 by `Status::validate_update`'s invariant - so a missing exit code
/// on a `Successful` row is inferred, not actually absent. Every other status's missing exit code
/// reflects a row that genuinely never exited on its own terms (still running, or killed before it
/// could report one), so it is left as `None` rather than guessing.
pub fn effective_exit_code(status: Status, exit_code: Option<i32>) -> Option<i32> {
    match (status, exit_code) {
        (Status::Successful, None) => Some(0),
        _ => exit_code,
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

    #[test]
    fn effective_exit_code_infers_zero_for_successful_with_no_recorded_code() {
        assert_eq!(effective_exit_code(Status::Successful, None), Some(0));
    }

    #[test]
    fn effective_exit_code_leaves_other_missing_codes_alone() {
        for status in [
            Status::Initialising,
            Status::Resolving,
            Status::Provisioning,
            Status::EnvironmentReady,
            Status::Running,
            Status::Failed,
            Status::Unrunnable,
        ] {
            assert_eq!(effective_exit_code(status, None), None, "{status:?}");
        }
    }

    #[test]
    fn effective_exit_code_passes_through_a_recorded_code_unchanged() {
        assert_eq!(effective_exit_code(Status::Failed, Some(42)), Some(42));
        assert_eq!(effective_exit_code(Status::Successful, Some(0)), Some(0));
    }
}

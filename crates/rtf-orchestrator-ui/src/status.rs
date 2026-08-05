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
    use simple_test_case::test_case;

    #[test_case(Status::Initialising, "status--pending"; "initialising")]
    #[test_case(Status::Resolving, "status--pending"; "resolving")]
    #[test_case(Status::Provisioning, "status--pending"; "provisioning")]
    #[test_case(Status::EnvironmentReady, "status--active"; "environment ready")]
    #[test_case(Status::Running, "status--active"; "running")]
    #[test_case(Status::Successful, "status--success"; "successful")]
    #[test_case(Status::Failed, "status--failed"; "failed")]
    #[test_case(Status::Unrunnable, "status--unrunnable"; "unrunnable")]
    #[test]
    fn css_class_matches_the_status_category(status: Status, expected: &str) {
        assert_eq!(css_class(status), expected);
    }

    #[test_case(Status::Successful, None, Some(0); "successful with no recorded code infers zero")]
    #[test_case(Status::Initialising, None, None; "initialising with no recorded code stays none")]
    #[test_case(Status::Resolving, None, None; "resolving with no recorded code stays none")]
    #[test_case(Status::Provisioning, None, None; "provisioning with no recorded code stays none")]
    #[test_case(Status::EnvironmentReady, None, None; "environment ready with no recorded code stays none")]
    #[test_case(Status::Running, None, None; "running with no recorded code stays none")]
    #[test_case(Status::Failed, None, None; "failed with no recorded code stays none")]
    #[test_case(Status::Unrunnable, None, None; "unrunnable with no recorded code stays none")]
    #[test_case(Status::Failed, Some(42), Some(42); "failed recorded code passes through unchanged")]
    #[test_case(Status::Successful, Some(0), Some(0); "successful recorded code passes through unchanged")]
    #[test]
    fn effective_exit_code_cases(status: Status, exit_code: Option<i32>, expected: Option<i32>) {
        assert_eq!(effective_exit_code(status, exit_code), expected);
    }
}

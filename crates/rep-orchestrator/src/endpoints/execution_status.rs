//! Fetch the status of a given [TestExecution] by its UUID.
use crate::{
    Error, Result, conn,
    db::{Status, StatusTracked, TestExecution},
    endpoints::BearerToken,
};
use axum::{Json, extract::Path};
use rep_orchestrator_shared::{
    payload::SetStatusPayload,
    status::{Status as SharedStatus, StatusUpdate as SharedStatusUpdate},
    summary::TestExecutionSummary,
};
use uuid::Uuid;

pub async fn get_handler(Path(id): Path<Uuid>) -> Result<Json<TestExecutionSummary>> {
    let conn = conn!();

    match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => Ok(Json(ex.try_into_summary(conn).await?)),
        None => Err(Error::UnknownTestExecution { id }),
    }
}

pub async fn post_handler(
    auth: BearerToken,
    Path(id): Path<Uuid>,
    Json(payload): Json<SetStatusPayload>,
) -> Result<Json<SharedStatusUpdate>> {
    let conn = conn!();

    let mut ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::Unauthorized),
    };

    auth.verify(ex.token())?;

    let current = ex.current_status(conn).await?;
    validate(&payload, current.status.into())?;

    ex.set_status(payload.status.into(), payload.message.clone(), conn)
        .await?;
    if let (SharedStatus::Failed, Some(code)) = (payload.status, payload.exit_code) {
        ex.set_exit_code(code, conn).await?;
    }

    let new = ex.current_status(conn).await?;

    Ok(Json(new.into()))
}

fn validate(payload: &SetStatusPayload, current_status: SharedStatus) -> Result<()> {
    use SharedStatus::*;

    let db_current_status: Status = current_status.into();
    let db_payload_status: Status = payload.status.into();

    // We check strictly greater than in order to allow multiple updates at the same Status
    // with different messages (e.g. the different stages of provisioning). But we disallow
    // moving backward through the statuses or setting multiple terminal statuses.
    if db_current_status > db_payload_status || db_current_status.is_complete() {
        return Err(Error::InvalidExecutionStatus {
            current: current_status,
            requested: payload.status,
        });
    }

    match (payload.status, payload.exit_code) {
        (Failed, None) => return Err(Error::MissingExitCode),
        (Failed, Some(0)) => return Err(Error::InvalidFailedExitCode),
        (Failed, Some(_)) => (),
        (Successful, Some(0)) => (),
        (_, Some(code)) => {
            return Err(Error::InvalidExitCode {
                status: payload.status,
                code,
            });
        }
        _ => (),
    };

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::TestRun, test_helpers::TestServerState};
    use SharedStatus::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use reqwest::StatusCode;
    use simple_test_case::test_case;

    fn bearer(token: &Uuid) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
    }

    // valid
    #[test_case(Running, None, Ok(()); "valid non-error")]
    #[test_case(Provisioning, None, Ok(()); "valid repeat of current status")]
    #[test_case(Successful, Some(0), Ok(()); "valid successful with 0 exit code")]
    #[test_case(Failed, Some(1), Ok(()); "valid error")]
    // invalid
    #[test_case(Failed, Some(0), Err(Error::InvalidFailedExitCode); "error with 0 exit code")]
    #[test_case(Failed, None, Err(Error::MissingExitCode); "error without exit code")]
    #[test_case(Running, Some(0), Err(Error::InvalidExitCode { status: Running, code: 0 }); "unexpected exit code")]
    #[test_case(
        Initialising,
        None,
        Err(Error::InvalidExecutionStatus { current: Provisioning, requested: Initialising });
        "status rollback"
    )]
    #[test]
    fn payload_validation_works(status: SharedStatus, exit_code: Option<u8>, expected: Result<()>) {
        let payload = SetStatusPayload {
            status,
            message: None,
            exit_code,
        };
        let res = validate(&payload, Provisioning);

        match (expected, res) {
            (Ok(()), Ok(())) => (),
            (Err(e1), Err(e2)) if e1.to_string() == e2.to_string() => (),
            (r1, r2) => panic!("expected {r1:?}, got {r2:?}"),
        }
    }

    #[test_case(Successful; "successful")]
    #[test_case(Failed; "failed")]
    #[test_case(Unrunnable; "unrunnable")]
    #[test]
    fn validate_second_terminal_status_is_invalid(current: SharedStatus) {
        let payload = SetStatusPayload {
            status: Successful,
            message: None,
            exit_code: None,
        };
        let res = validate(&payload, current);

        assert!(
            matches!(
                res,
                Err(Error::InvalidExecutionStatus {
                    current: _,
                    requested: Successful
                })
            ),
            "{res:?}"
        );
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_handler_returns_200_for_known_execution() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let ex_id = tr.init_execution("test", conn).await?.uuid();

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_id}/status"))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_handler_returns_404_for_unknown_execution() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let ex_id = Uuid::new_v4();

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_id}/status"))
            .await;
        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }

    // Helper for the parameterised test below
    fn su(status: SharedStatus, exit_code: Option<u8>) -> SetStatusPayload {
        SetStatusPayload {
            status,
            message: None,
            exit_code,
        }
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(&[su(Running, None), su(Successful, None)]; "successful")]
    #[test_case(&[su(Running, None), su(Successful, Some(0))]; "successful with 0 exit code")]
    #[test_case(&[su(Running, None), su(Failed, Some(1))]; "failed")]
    #[test_case(&[su(Unrunnable, None)]; "unrunnable")]
    #[tokio::test]
    async fn post_handler_accepts_valid_update_sequence(
        payloads: &[SetStatusPayload],
    ) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_id, token) = {
            let conn = conn!();
            let tr = TestRun::init("test", conn).await?;
            let ex = tr.init_execution("test", conn).await?;
            (ex.uuid(), ex.token().to_owned())
        };

        // This isn't the true update sequence for a happy-path test execution going through the
        // main event loop, but for the purposes of accepting the update requests via this endpoint
        // all we really care about is the update sequence we are attempting to send to the server.
        let mut final_statuses = vec![Initialising];

        for (i, payload) in payloads.iter().enumerate() {
            let resp = tss
                .test_server
                .post(&format!("/test-execution/{ex_id}/status"))
                .add_header(AUTHORIZATION, bearer(&token))
                .json(payload)
                .await;

            assert_eq!(resp.status_code(), StatusCode::OK, "POST {i}");
            let update: SharedStatusUpdate = resp.json();

            assert_eq!(update.status, payload.status, "POST {i}");
            final_statuses.push(update.status);
        }

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_id}/status"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "GET");

        let queried: TestExecutionSummary = resp.json();
        let queried_statuses: Vec<SharedStatus> =
            queried.status_history.iter().map(|u| u.status).collect();
        final_statuses.reverse(); // order in the summary is most recent first

        assert_eq!(queried_statuses, final_statuses);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(&[su(Running, None)], su(Provisioning, None); "status rollback")]
    #[test_case(&[su(Failed, Some(1))], su(Successful, None); "second terminal status")]
    #[test_case(&[], su(Failed, None); "failed without exit code")]
    #[test_case(&[], su(Failed, Some(0)); "failed with 0 exit code")]
    #[test_case(&[], su(Successful, Some(1)); "successful with non-0 exit code")]
    #[test_case(&[], su(Unrunnable, Some(2)); "unrunnable with exit code")]
    #[test_case(&[], su(Running, Some(3)); "non-terminal with exit code")]
    #[tokio::test]
    async fn post_handler_rejects_invalid_update_sequence(
        valid_payloads: &[SetStatusPayload],
        invalid_payload: SetStatusPayload,
    ) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_id, token) = {
            let conn = conn!();
            let tr = TestRun::init("test", conn).await?;
            let ex = tr.init_execution("test", conn).await?;
            (ex.uuid(), ex.token().to_owned())
        };

        for (i, payload) in valid_payloads.iter().enumerate() {
            let resp = tss
                .test_server
                .post(&format!("/test-execution/{ex_id}/status"))
                .add_header(AUTHORIZATION, bearer(&token))
                .json(payload)
                .await;

            assert_eq!(resp.status_code(), StatusCode::OK, "POST {i}");
            let update: SharedStatusUpdate = resp.json();

            assert_eq!(update.status, payload.status, "POST {i}");
        }

        let resp = tss
            .test_server
            .post(&format!("/test-execution/{ex_id}/status"))
            .add_header(AUTHORIZATION, bearer(&token))
            .json(&invalid_payload)
            .await;

        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{:?}",
            resp.json::<serde_json::Value>()
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn post_handler_sets_exit_code_for_failed_status() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_id, token) = {
            let conn = conn!();
            let tr = TestRun::init("test", conn).await?;
            let ex = tr.init_execution("test", conn).await?;
            (ex.uuid(), ex.token().to_owned())
        };

        let resp = tss
            .test_server
            .post(&format!("/test-execution/{ex_id}/status"))
            .add_header(AUTHORIZATION, bearer(&token))
            .json(&su(Failed, Some(42)))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "POST");
        let update: SharedStatusUpdate = resp.json();

        assert_eq!(update.status, Failed);

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_id}/status"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "GET");
        let summary: TestExecutionSummary = resp.json();

        assert_eq!(summary.exit_code, Some(42));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn post_handler_returns_403_without_token() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        tr.init_execution("test", conn).await?;

        let resp = tss
            .test_server
            .post(&format!("/test-execution/{}/status", Uuid::new_v4()))
            .json(&su(Running, None))
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn post_handler_returns_403_for_unknown_execution() -> anyhow::Result<()> {
        let tss = TestServerState::new();

        let resp = tss
            .test_server
            .post(&format!("/test-execution/{}/status", Uuid::new_v4()))
            .json(&su(Running, None))
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn post_handler_returns_403_with_wrong_token() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let ex_id = {
            let conn = conn!();
            let tr = TestRun::init("test", conn).await?;
            tr.init_execution("test", conn).await?.uuid()
        };

        let resp = tss
            .test_server
            .post(&format!("/test-execution/{ex_id}/status"))
            .add_header(AUTHORIZATION, bearer(&Uuid::new_v4()))
            .json(&su(Running, None))
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_handler_does_not_require_token() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let ex_id = tr.init_execution("test", conn).await?.uuid();

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_id}/status"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        Ok(())
    }
}

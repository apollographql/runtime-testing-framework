//! Fetch the status of a given [TestExecution] by its UUID.
use crate::{
    Error, Result, conn,
    db::{Status, StatusTracked, StatusUpdate, TestExecution},
    response_types::TestExecutionSummary,
};
use axum::{Json, extract::Path};
use serde::Deserialize;
use uuid::Uuid;

pub async fn get_handler(Path(id): Path<Uuid>) -> Result<Json<TestExecutionSummary>> {
    let conn = conn!();

    match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => Ok(Json(ex.try_into_summary(conn).await?)),
        None => Err(Error::UnknownTestExecution { id }),
    }
}

// TODO: This endpoint needs to be authenticated
pub async fn post_handler(
    Path(id): Path<Uuid>,
    Json(payload): Json<SetStatusPayload>,
) -> Result<Json<StatusUpdate>> {
    let conn = conn!();

    let mut ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::UnknownTestExecution { id }),
    };

    let current = ex.current_status(conn).await?;
    payload.validate(current.status)?;

    ex.set_status(payload.status, payload.message, conn).await?;
    if let (Status::Failed, Some(code)) = (payload.status, payload.exit_code) {
        ex.set_exit_code(code, conn).await?;
    }

    let new = ex.current_status(conn).await?;

    Ok(Json(new))
}

#[derive(Debug, Deserialize)]
pub struct SetStatusPayload {
    pub status: Status,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub exit_code: Option<u8>,
}

impl SetStatusPayload {
    fn validate(&self, current_status: Status) -> Result<()> {
        use Status::*;

        // We check strictly greater than in order to allow multiple updates at the same Status
        // with different messages (e.g. the different stages of provisioning). But we disallow
        // moving backward through the statuses or setting multiple terminal statuses.
        if current_status > self.status || current_status.is_complete() {
            return Err(Error::InvalidExecutionStatus {
                current: current_status,
                requested: self.status,
            });
        }

        match (self.status, self.exit_code) {
            (Failed, None) => return Err(Error::MissingExitCode),
            (Failed, Some(0)) => return Err(Error::InvalidFailedExitCode),
            (Failed, Some(_)) => (),
            (Successful, Some(0)) => (),
            (_, Some(code)) => {
                return Err(Error::InvalidExitCode {
                    status: self.status,
                    code,
                });
            }
            _ => (),
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;
    use simple_test_case::test_case;

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
    fn payload_validation_works(status: Status, exit_code: Option<u8>, expected: Result<()>) {
        let payload = SetStatusPayload {
            status,
            message: None,
            exit_code,
        };
        let res = payload.validate(Provisioning);

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
    fn attempt_to_set_second_terminal_status_is_invalid(current: Status) {
        let payload = SetStatusPayload {
            status: Successful,
            message: None,
            exit_code: None,
        };
        let res = payload.validate(current);

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
}

//! Fetch uploaded output for a given [TestExecution] by its UUID.
use crate::{Error, Result, conn, db::TestExecution, gcs::Client, state::ServerState};
use axum::{
    extract::{Path, State},
    response::Redirect,
};
use sqlx::PgConnection;
use uuid::Uuid;

/// Download and return the string content of the uploaded log file
pub async fn log_file_handler(
    Path(id): Path<Uuid>,
    State(ServerState { gcs_client, .. }): State<ServerState>,
) -> Result<String> {
    let ex = get_validated_execution(id, conn!()).await?;
    let bytes = gcs_client
        .download_bytes(ex.log_file_gcs_object_name())
        .await?;

    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// 307 redirect to a signed download URL for the output zip file
pub async fn output_zip_handler(
    Path(id): Path<Uuid>,
    State(ServerState { gcs_client, .. }): State<ServerState>,
) -> Result<Redirect> {
    let ex = get_validated_execution(id, conn!()).await?;
    let url = gcs_client
        .signed_download_url(ex.output_zip_gcs_object_name())
        .await?;

    Ok(Redirect::temporary(&url))
}

async fn get_validated_execution(id: Uuid, conn: &mut PgConnection) -> Result<TestExecution> {
    let ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::UnknownTestExecution { id }),
    };

    if !ex.has_file_upload() {
        return Err(Error::FileUploadNotAvailable);
    } else if !ex.is_complete() {
        return Err(Error::FileUploadNotReady);
    }

    Ok(ex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{Status, StatusTracked, TestRun},
        gcs::{GCSClient, MockClient},
        test_helpers::TestServerState,
    };
    use reqwest::{StatusCode, header::LOCATION};
    use simple_test_case::test_case;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Successful; "status is successful")]
    #[test_case(Status::Failed; "status is failed")]
    #[tokio::test]
    async fn log_file_handler_returns_expected_content(status: Status) -> anyhow::Result<()> {
        let tss = TestServerState::new_with_gcs_client(GCSClient::new_mock(
            "internal_url",
            "public_url",
            "bucket",
            Some("hello, world!".into()),
        ));
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let mut ex = tr.init_execution("test", 0, conn).await?;
        ex.mark_has_file_upload(conn).await?;
        ex.set_status(status, None, conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{}/log.txt", ex.uuid()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        assert_eq!(resp.text(), "hello, world!");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Successful; "status is successful")]
    #[test_case(Status::Failed; "status is failed")]
    #[tokio::test]
    async fn output_zip_handler_redirects(status: Status) -> anyhow::Result<()> {
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let mut ex = tr.init_execution("test", 0, conn).await?;
        ex.mark_has_file_upload(conn).await?;
        ex.set_status(status, None, conn).await?;

        let client = MockClient::new("internal_url", "public_url", "bucket");
        let expected_redirect_url = client.public_url_for_object(ex.output_zip_gcs_object_name());

        let tss = TestServerState::new_with_gcs_client(GCSClient::Mock(client));
        let resp = tss
            .test_server
            .get(&format!("/test-execution/{}/output.zip", ex.uuid()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(resp.header(LOCATION), expected_redirect_url);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("log.txt"; "log file")]
    #[test_case("output.zip"; "output zip")]
    #[tokio::test]
    async fn handlers_returns_404_for_unknown_execution(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let resp = tss
            .test_server
            .get(&format!("/test-execution/{}/{endpoint}", Uuid::new_v4()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("log.txt"; "log file")]
    #[test_case("output.zip"; "output zip")]
    #[tokio::test]
    async fn handlers_returns_404_for_known_execution_without_file_upload(
        endpoint: &str,
    ) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let ex = tr.init_execution("test", 0, conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{}/{endpoint}", ex.uuid()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("log.txt"; "log file")]
    #[test_case("output.zip"; "output zip")]
    #[tokio::test]
    async fn handlers_returns_409_for_when_execution_is_not_complete(
        endpoint: &str,
    ) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let mut ex = tr.init_execution("test", 0, conn).await?;
        ex.mark_has_file_upload(conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{}/{endpoint}", ex.uuid()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::CONFLICT);

        Ok(())
    }
}

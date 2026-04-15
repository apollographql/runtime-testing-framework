//! Request signed upload URLs for GCS for a given execution
use crate::{Error, Result, conn, db::TestExecution, gcs::Client, state::ServerState};
use axum::{
    Json,
    extract::{Path, State},
};
use rep_orchestrator_shared::{payload::GenerateUploadUrlsPayload, upload_urls::UploadUrls};
use uuid::Uuid;

// TODO: This endpoint needs to be authenticated
pub async fn handler(
    Path(id): Path<Uuid>,
    State(ServerState { gcs_client, .. }): State<ServerState>,
    Json(_): Json<GenerateUploadUrlsPayload>,
) -> Result<Json<UploadUrls>> {
    let conn = conn!();

    let mut ex = match TestExecution::get_by_uuid(&id, conn).await? {
        None => return Err(Error::UnknownTestExecution { id }),
        Some(ex) => ex,
    };

    if ex.has_file_upload() {
        return Err(Error::FileUploadAlreadyRequested);
    }

    let log_file_url = gcs_client
        .signed_upload_url(ex.log_file_gcs_object_name())
        .await?;
    let output_zip_url = gcs_client
        .signed_upload_url(ex.output_zip_gcs_object_name())
        .await?;

    ex.mark_has_file_upload(conn).await?;

    Ok(Json(UploadUrls {
        log_file_url,
        output_zip_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{Queryable, TestRun},
        test_helpers::TestServerState,
    };
    use reqwest::StatusCode;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_marks_file_upload_as_requested() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let ex = tr.init_execution("test", conn).await?;

        let resp = tss
            .test_server
            .post(&format!(
                "/test-execution/{}/generate-upload-urls",
                ex.uuid()
            ))
            .json(&GenerateUploadUrlsPayload {})
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let queried = TestExecution::get_by_id_unchecked(ex.id(), conn)
            .await
            .unwrap();

        assert!(queried.has_file_upload(), "should have file upload marked");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handle_returns_400_to_second_upload_request() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let ex = tr.init_execution("test", conn).await?;

        for (i, expected) in [StatusCode::OK, StatusCode::BAD_REQUEST].iter().enumerate() {
            let resp = tss
                .test_server
                .post(&format!(
                    "/test-execution/{}/generate-upload-urls",
                    ex.uuid()
                ))
                .json(&GenerateUploadUrlsPayload {})
                .await;

            assert_eq!(resp.status_code(), *expected, "request {}", i + 1);
        }

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_404_for_unknown_execution() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let resp = tss
            .test_server
            .post(&format!(
                "/test-execution/{}/generate-upload-urls",
                Uuid::new_v4()
            ))
            .json(&GenerateUploadUrlsPayload {})
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}

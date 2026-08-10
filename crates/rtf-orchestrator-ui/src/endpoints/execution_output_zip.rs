use crate::{endpoints::download_response, orchestrator::Client};
use axum::{
    extract::{Path, State},
    response::Response,
};
use uuid::Uuid;

/// `GET /ui/execution/{eid}/output.zip`
///
/// The orchestrator answers this with a 307 to a signed GCS URL, which this handler relays
/// verbatim rather than following - the UI never downloads the artifact's bytes itself.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    download_response(
        orchestrator_client.execution_output_zip(execution_id).await,
        "application/zip",
        format!("{execution_id}-output.zip"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{endpoints::body_text, orchestrator::mocks::MockClient};
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;

    #[tokio::test]
    async fn execution_output_zip_calls_the_client_and_streams_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = handler(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "output zip contents");
    }
}

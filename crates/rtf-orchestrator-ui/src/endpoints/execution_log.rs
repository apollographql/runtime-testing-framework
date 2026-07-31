use crate::{endpoints::download_response, orchestrator::Client};
use axum::{
    extract::{Path, State},
    response::Response,
};
use uuid::Uuid;

/// `GET /ui/execution/{eid}/log.txt`
///
/// Proxies the execution's log file from the orchestrator, so
/// the browser only ever talks to the UI's own origin.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    download_response(
        orchestrator_client.execution_log(execution_id).await,
        "text/plain; charset=utf-8",
        format!("{execution_id}-log.txt"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{endpoints::body_text, orchestrator::mocks::MockClient};
    use rep_orchestrator_shared::status::Status;
    use reqwest::StatusCode;

    #[tokio::test]
    async fn execution_log_calls_the_client_and_streams_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = handler(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "log contents");
    }
}

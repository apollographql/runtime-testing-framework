use crate::{Result, db::pool::check_db_conn};
use axum::Json;
use serde::Serialize;

pub async fn handler() -> Result<Json<HealthResponse>> {
    check_db_conn().await?;

    Ok(Json(HealthResponse { ok: true }))
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    ok: bool,
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::TestServerState;
    use reqwest::StatusCode;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_200() {
        let tss = TestServerState::new();

        let resp = tss.test_server.get("/health").await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }
}

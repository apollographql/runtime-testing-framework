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

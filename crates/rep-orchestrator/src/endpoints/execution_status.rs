//! Fetch the status of a given [TestExecution] by its UUID.
use crate::{Error, Result, conn, db::TestExecution, response_types::TestExecutionSummary};
use axum::{Json, extract::Path};
use uuid::Uuid;

pub async fn handler(Path(id): Path<Uuid>) -> Result<Json<TestExecutionSummary>> {
    let conn = conn!();

    match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => Ok(Json(ex.try_into_summary(conn).await?)),
        None => Err(Error::UnknownTestExecution { id }),
    }
}

//! Fetch the status of a given [TestRun] by its UUID.
use crate::{Error, Result, conn, db::TestRun};
use axum::{Json, extract::Path};
use rep_orchestrator_shared::summary::TestRunSummary;
use uuid::Uuid;

pub async fn handler(Path(id): Path<Uuid>) -> Result<Json<TestRunSummary>> {
    let conn = conn!();

    match TestRun::get_by_uuid(&id, conn).await? {
        Some(ex) => Ok(Json(ex.try_into_summary(conn).await?)),
        None => Err(Error::UnknownTestRun { id }),
    }
}

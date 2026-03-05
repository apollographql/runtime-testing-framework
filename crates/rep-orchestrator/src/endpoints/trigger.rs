//! Trigger a new test run
use crate::AppError;
use axum::{Json, body::Bytes};
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use rtf_config::{formats::TestPlanConfig, templating::Scalar};
use serde::Serialize;
use std::{collections::HashMap, io::Read};
use tar::Archive;
use uuid::Uuid;

// TODO: move these out of the handler file
#[derive(Default, Debug, Serialize)]
pub struct TestRunSummary {
    pub id: Uuid,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub executions: Vec<TestExecutionSummary>,
}

#[derive(Default, Debug, Serialize)]
pub struct TestExecutionSummary {
    pub id: Uuid,
    pub name: String,
    pub status: ExecutionStatus,
    pub vars: HashMap<String, Scalar>,
}

#[derive(Default, Debug, Serialize)]
pub enum RunStatus {
    #[default]
    Init,
}

#[derive(Default, Debug, Serialize)]
pub enum ExecutionStatus {
    #[default]
    Init,
}

pub async fn handler(body: Bytes) -> Result<Json<TestRunSummary>, AppError> {
    inner(body).await.map_err(AppError)
}

async fn inner(body: Bytes) -> anyhow::Result<Json<TestRunSummary>> {
    let mut archive = Archive::new(GzDecoder::new(body.as_ref()));
    let mut summary = TestRunSummary {
        id: Uuid::new_v4(),
        ..Default::default()
    };

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();

        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let name = stem.strip_suffix("-inlined-test-plan").unwrap().to_string();

        let mut contents = String::new();
        entry.read_to_string(&mut contents)?;
        let tp: TestPlanConfig = serde_yaml::from_str(&contents)?;
        summary.executions.push(TestExecutionSummary {
            id: Uuid::new_v4(),
            name,
            status: Default::default(),
            vars: tp.variables,
        });
    }

    Ok(Json(summary))
}

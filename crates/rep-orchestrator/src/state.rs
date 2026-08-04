use crate::{
    Result, config::Config, db::TestRun, event_loop::EventQueueState, gcs::GCSClient,
    iap_identity::extract_authenticated_user_email,
};
use axum::http::HeaderMap;
use rep_orchestrator_shared::payload::PreparedPayload;
use std::{io::ErrorKind, sync::Arc};
use tokio::fs;
use tracing::error;

#[derive(Debug, Clone)]
pub struct ServerState {
    pub eq_state: EventQueueState,
    pub gcs_client: Arc<GCSClient>,
}

impl ServerState {
    pub fn new(eq_state: EventQueueState, gcs_client: GCSClient) -> Self {
        Self {
            eq_state,
            gcs_client: Arc::new(gcs_client),
        }
    }

    /// Load and parse the admins list (one email per line) if the admins file exists.
    /// A missing file is treated as an empty admins list rather than a hard error.
    pub async fn admins(&self) -> Result<Vec<String>> {
        let path = &Config::get().admins_path;

        match fs::read_to_string(path).await {
            Ok(content) => Ok(content
                .lines()
                .filter_map(|raw| {
                    let s = raw.trim();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                })
                .collect()),

            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),

            Err(e) => {
                error!(%e, path, "unable to read admins file");
                Err(e.into())
            }
        }
    }

    pub async fn identify_user(&self, headers: &HeaderMap) -> Result<UserType> {
        let email = match extract_authenticated_user_email(headers) {
            Some(email) => email,
            None => return Ok(UserType::Unknown),
        };

        let admins = self.admins().await?;

        Ok(if admins.contains(&email) {
            UserType::Admin(email)
        } else {
            UserType::User(email)
        })
    }
}

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: PreparedPayload,
}

#[derive(Debug)]
pub enum UserType {
    Admin(String),
    User(String),
    Unknown,
}

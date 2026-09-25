use crate::{
    Result,
    config::Config,
    db::TestRun,
    event_loop::EventQueueState,
    gcs::GCSClient,
    iap_identity::{extract_authenticated_user_email, try_extract_trigger_repo},
};
use axum::http::HeaderMap;
use rtf_orchestrator_shared::payload::PreparedPayload;
use std::{io::ErrorKind, sync::Arc};
use tokio::fs;
use tracing::error;

#[derive(Debug, Clone)]
pub struct ServerState {
    pub eq_state: EventQueueState,
    pub gcs_client: Arc<GCSClient>,
    /// Overrides [ServerState::admins] with a fixed list for tests, bypassing the admins file
    /// entirely so admin-gated endpoints can be exercised without touching the filesystem.
    /// `None` (the default) falls through to the file-based lookup.
    #[cfg(test)]
    pub test_admins: Option<Vec<String>>,
}

impl ServerState {
    pub fn new(eq_state: EventQueueState, gcs_client: GCSClient) -> Self {
        Self {
            eq_state,
            gcs_client: Arc::new(gcs_client),
            #[cfg(test)]
            test_admins: None,
        }
    }

    /// Load and parse the admins list (one email per line) if the admins file exists.
    /// A missing file is treated as an empty admins list rather than a hard error.
    pub async fn admins(&self) -> Result<Vec<String>> {
        #[cfg(test)]
        if let Some(admins) = &self.test_admins {
            return Ok(admins.clone());
        }

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

        let utype = if Config::get().automation_users.contains(&email) {
            let (org, repo) = try_extract_trigger_repo(headers)
                .unwrap_or_else(|| ("unknown".into(), "unknown".into()));

            UserType::Automation { email, org, repo }
        } else if self.admins().await?.contains(&email) {
            UserType::Admin(email)
        } else {
            UserType::User(email)
        };

        Ok(utype)
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
    Automation {
        email: String,
        org: String,
        repo: String,
    },
    Unknown,
}

impl UserType {
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Admin(_))
    }

    /// Full user identity string used as the per-user rate limit key and stored in the DB
    pub fn user_identity(&self) -> Option<String> {
        match self {
            Self::Admin(email) | Self::User(email) => Some(email.to_string()),
            Self::Automation { email, org, repo } => {
                Some(format!("automation:{email}:{org}:{repo}"))
            }
            Self::Unknown => None,
        }
    }
}

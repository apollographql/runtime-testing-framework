use crate::{
    Error, Result,
    config::Config,
    db::TestRun,
    event_loop::EventQueueState,
    gcs::GCSClient,
    iap_identity::{extract_authenticated_user_email, try_extract_trigger_repo},
};
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};
use rtf_orchestrator_shared::{payload::PreparedPayload, workload_config::WorkloadConfig};
use std::{io::ErrorKind, sync::Arc};
use tokio::fs;
use tracing::error;

const AUTOMATION_PREFIX: &str = "automation:";

#[derive(Debug, Clone)]
pub struct ServerState {
    pub eq_state: EventQueueState,
    pub gcs_client: Arc<GCSClient>,
    pub user_classifier: UserClassifier,
}

impl ServerState {
    pub fn new(
        eq_state: EventQueueState,
        gcs_client: GCSClient,
        user_classifier: UserClassifier,
    ) -> Self {
        Self {
            eq_state,
            gcs_client: Arc::new(gcs_client),
            user_classifier,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct UserClassifier {
    /// Overrides [UserClassifier::admins] with a fixed list for tests, bypassing the admins file
    /// entirely so admin-gated endpoints can be exercised without touching the filesystem.
    /// `None` (the default) falls through to the file-based lookup.
    #[cfg(test)]
    pub test_admins: Option<Vec<String>>,
}

impl UserClassifier {
    /// Load and parse the admins list (one email per line) if the admins file exists.
    /// A missing file is treated as an empty admins list rather than a hard error.
    pub async fn admins(&self) -> Result<Vec<String>> {
        #[cfg(test)]
        if let Some(admins) = &self.test_admins {
            return Ok(admins.clone());
        }

        self.read_admins(&Config::get().admins_path).await
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

    /// Rebuild a [UserType] from the identity string stored in the DB.
    /// Inverse of [UserType::user_identity].
    pub async fn user_type_from_identity(&self, identity: Option<&str>) -> Result<UserType> {
        let Some(id) = identity else {
            return Ok(UserType::Unknown);
        };

        let admins = self.admins().await?;

        if let Some(rest) = id.strip_prefix(AUTOMATION_PREFIX) {
            let mut parts = rest.splitn(3, ':');
            if let (Some(email), Some(org), Some(repo)) = (parts.next(), parts.next(), parts.next())
            {
                return Ok(UserType::Automation {
                    email: email.to_string(),
                    org: org.to_string(),
                    repo: repo.to_string(),
                });
            }
        }

        // Use `admins` to tell a [UserType::Admin] from a [UserType::User] because both are stored as an email.
        // Someone removed from the admins file since the run was triggered is recovered as a
        // [UserType::User] and becomes subject to per-user limits.
        if admins.iter().any(|a| a == id) {
            Ok(UserType::Admin(id.to_string()))
        } else {
            Ok(UserType::User(id.to_string()))
        }
    }

    /// Load and parse the admins list (one email per line) from the file at `path`.
    /// A missing file is treated as an empty admins list rather than a hard error.
    pub async fn read_admins(&self, path: &str) -> Result<Vec<String>> {
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
}

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: PreparedPayload,
    pub requires_dedicated: bool,
    pub workload_config: WorkloadConfig,
    pub initiated_by_user: UserType,
}

#[derive(Debug, Clone, PartialEq)]
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
                Some(format!("{AUTOMATION_PREFIX}{email}:{org}:{repo}"))
            }
            Self::Unknown => None,
        }
    }
}

/// A subset of [UserType] that is used to gate requests that modify details for known test plans.
pub enum KnownUser {
    Admin(String),
    User(String),
}

impl KnownUser {
    pub fn email(&self) -> &str {
        match self {
            Self::Admin(email) | Self::User(email) => email,
        }
    }
}

impl FromRequestParts<ServerState> for KnownUser {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &ServerState) -> Result<Self> {
        match state.user_classifier.identify_user(&parts.headers).await? {
            UserType::Admin(email) => Ok(Self::Admin(email)),
            UserType::User(email) => Ok(Self::User(email)),
            UserType::Automation { .. } | UserType::Unknown => Err(Error::Unauthorized),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UserClassifier, UserType};
    use simple_test_case::test_case;

    #[test_case(UserType::Admin("admin@test.com".into()); "admin")]
    #[test_case(UserType::User("user@test.com".into()); "user")]
    #[test_case(UserType::Automation { email: "bot@test.com".into(), org: "org".into(), repo: "test-repo".into() }; "automation")]
    #[test_case(UserType::Unknown; "unknown")]
    #[tokio::test]
    async fn from_identity_recovers_user_type(user: UserType) {
        let admins = vec!["admin@test.com".to_string()];
        let id = user.user_identity();
        let user_classifier = UserClassifier {
            test_admins: Some(admins),
        };

        let res = user_classifier.user_type_from_identity(id.as_deref()).await;
        assert!(res.is_ok(), "expected ok, got {res:?}");
        let user_type_recovered_from_id = res.unwrap();
        assert_eq!(user_type_recovered_from_id, user);
    }

    #[test_case(Some("admin@test.com"); "admin email")]
    #[test_case(Some("user@test.com"); "user email")]
    #[test_case(Some("automation:bot@test.com:org:test-repo"); "automation")]
    #[test_case(Some("automation:bot@test.com:org:test:repo"); "automation repo contains a colon")]
    #[test_case(Some("automation:malformed"); "malformed automation falls back to user")]
    #[test_case(None; "no identity")]
    #[tokio::test]
    async fn user_identity_returns_the_string_identity_was_built_from(identity: Option<&str>) {
        let admins = vec!["admin@test.com".to_string()];
        let user_classifier = UserClassifier {
            test_admins: Some(admins),
        };
        let user = user_classifier
            .user_type_from_identity(identity)
            .await
            .unwrap();

        assert_eq!(user.user_identity().as_deref(), identity);
    }
}

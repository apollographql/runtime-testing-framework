use crate::error::CliError;
use anyhow::{Context, anyhow};
use rep_orchestrator_shared::{payload::SetStatusPayload, status::Status};
use std::process::ExitStatus;
use tracing::info;
use uuid::Uuid;

pub trait Client: Send + Sync {
    /// Updates the status of the current test execution with context from the provided [CliError].
    async fn update_error_status(&self, error: CliError) -> anyhow::Result<()> {
        self.update_status(
            error.rep_orchestrator_status(),
            error.exit_status(),
            Some(error.source_to_string()),
        )
        .await
    }

    /// Update the [Status] of the current test execution, with an optional `message` to write to the database.
    ///
    /// If there was a subprocess associated with this status update, its [ExitStatus] may be included for additional context.
    async fn update_status(
        &self,
        status: Status,
        exit_status: Option<ExitStatus>,
        message: Option<String>,
    ) -> anyhow::Result<()>;
}

pub struct HttpClient {
    orchestrator_url: String,
    execution_id: Uuid,
}

impl HttpClient {
    /// Creates a new [HttpClient] that will report updates to `orchestrator_url` for the test execution associated
    /// with `execution_id`.
    pub fn new(orchestrator_url: String, execution_id: Uuid) -> Self {
        Self {
            orchestrator_url,
            execution_id,
        }
    }
}

impl Client for HttpClient {
    async fn update_status(
        &self,
        status: Status,
        exit_status: Option<ExitStatus>,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/test-execution/{}/status",
            self.orchestrator_url, self.execution_id
        );
        let payload = SetStatusPayload {
            status,
            exit_code: exit_status
                .and_then(|status| status.code())
                .map(|code| code as u8),
            message,
        };

        info!(id=%self.execution_id, %status, "updating execution status");
        let resp = reqwest::Client::new()
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to send status update request")?;
        let status_code = resp.status();

        if status_code.is_success() {
            info!(id=%self.execution_id, %status, "update successful");

            Ok(())
        } else {
            let msg = match resp.text().await {
                Ok(body) => {
                    format!(
                        "status update to {status} failed for {}; ({status_code}): {body}",
                        self.execution_id
                    )
                }
                Err(err) => format!(
                    "status update to {status} failed for {}; ({status_code}): [body could not be read: {err}]",
                    self.execution_id
                ),
            };

            Err(anyhow!(msg))
        }
    }
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use rep_orchestrator_shared::status::Status;
    use std::process::ExitStatus;
    use std::sync::{RwLock, RwLockReadGuard};

    pub struct StatusUpdateArgs {
        pub status: Status,
        pub exit_status: Option<ExitStatus>,
        pub message: Option<String>,
    }

    #[derive(Default)]
    pub struct MockClient {
        status_updates: RwLock<Vec<StatusUpdateArgs>>,
        update_should_fail: bool,
    }

    impl MockClient {
        pub fn failing() -> Self {
            Self {
                status_updates: RwLock::new(Vec::new()),
                update_should_fail: true,
            }
        }

        pub fn read_updates<F>(&self, closure: F)
        where
            F: FnOnce(RwLockReadGuard<Vec<StatusUpdateArgs>>),
        {
            let updates = self.status_updates.read().unwrap();
            closure(updates)
        }
    }

    impl Client for MockClient {
        async fn update_status(
            &self,
            status: Status,
            exit_status: Option<ExitStatus>,
            message: Option<String>,
        ) -> anyhow::Result<()> {
            if self.update_should_fail {
                return Err(anyhow::anyhow!("mock update failure"));
            }
            self.status_updates.write().unwrap().push(StatusUpdateArgs {
                status,
                exit_status,
                message,
            });

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::mocks::MockClient;

    #[tokio::test]
    async fn update_error_status_delegates_failed_error() {
        use std::os::unix::process::ExitStatusExt;

        let client = MockClient::default();
        let exit_status = ExitStatus::from_raw(1 << 8); // exit code 1
        let error = CliError::failed(exit_status, "test plan failed".to_owned());

        client.update_error_status(error).await.unwrap();

        client.read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].status, Status::Failed);
            assert_eq!(updates[0].exit_status, Some(exit_status));
            assert_eq!(updates[0].message.as_deref(), Some("test plan failed"));
        });
    }

    #[tokio::test]
    async fn update_error_status_delegates_unrunnable_error() {
        let client = MockClient::default();
        let error = CliError::unrunnable(anyhow::anyhow!("setup broke"));

        client.update_error_status(error).await.unwrap();

        client.read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].status, Status::Unrunnable);
            assert_eq!(updates[0].exit_status, None);
            assert_eq!(updates[0].message.as_deref(), Some("setup broke"));
        });
    }
}

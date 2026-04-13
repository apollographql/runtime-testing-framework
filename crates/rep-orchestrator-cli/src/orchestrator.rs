use crate::error::CliError;
use anyhow::{Context, anyhow};
use rep_orchestrator_shared::{payload::SetStatusPayload, status::Status};
use std::{env, process::ExitStatus, str::FromStr};
use tracing::info;
use uuid::Uuid;

pub trait Client: Send + Sync {
    /// Updates the status of the current test execution with context from the provided [CliError].
    async fn update_error_status(&self, error: CliError) -> anyhow::Result<()>;

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
    async fn update_error_status(&self, error: CliError) -> anyhow::Result<()> {
        self.update_status(
            error.rep_orchestrator_status(),
            error.exit_status(),
            Some(error.source_to_string()),
        )
        .await
    }

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

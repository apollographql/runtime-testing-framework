use crate::error::CliError;
use anyhow::{Context, anyhow};
use rep_orchestrator_shared::{
    payload::{GenerateUploadUrlsPayload, SetStatusPayload},
    status::Status,
    upload_urls::UploadUrls,
};
use reqwest::Url;
use std::{path::Path, process::ExitStatus};
use tracing::info;
use uuid::Uuid;

const KUSTOMIZE_PATCH: &str = include_str!("../resources/kustomization.yaml");

pub trait Client: Send + Sync {
    fn kustomize_patch_for_execution(&self, outdir: &Path) -> String;

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

    /// Request signed URLs from the orchestrator for uploading the log file and output zip
    /// associated with the current test execution.
    async fn generate_upload_urls(&self) -> anyhow::Result<UploadUrls>;

    /// PUT `body` to a previously-issued signed upload URL.
    async fn upload_to_signed_url(&self, url: &str, body: Vec<u8>) -> anyhow::Result<()>;

    /// Fetch the resolved environment YAML for the current test execution.
    async fn fetch_environment_config(&self) -> anyhow::Result<Vec<u8>>;

    /// Fetch the resolved scenario YAML for the current test execution.
    async fn fetch_scenario_config(&self) -> anyhow::Result<Vec<u8>>;
}

pub struct HttpClient {
    orchestrator_url: Url,
    execution_id: Uuid,
    execution_token: Uuid,
}

impl HttpClient {
    /// Creates a new [HttpClient] that will report updates to `orchestrator_url` for the test execution associated
    /// with `execution_id`, authenticating requests with `execution_token` as a Bearer token.
    pub fn try_new(
        orchestrator_url: String,
        execution_id: Uuid,
        execution_token: Uuid,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            orchestrator_url: Url::parse(&orchestrator_url)?,
            execution_id,
            execution_token,
        })
    }

    async fn fetch_config(&self, endpoint: &str) -> anyhow::Result<Vec<u8>> {
        let url = self
            .orchestrator_url
            .join(&format!("test-execution/{}/{endpoint}", self.execution_id))?;

        info!(id=%self.execution_id, "fetching {}", endpoint);
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(self.execution_token)
            .send()
            .await
            .context(format!("failed to fetch config from {endpoint}"))?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "{endpoint} fetch failed for {}; ({status_code}): {body}",
                self.execution_id,
            ));
        }

        Ok(resp.bytes().await?.to_vec())
    }
}

impl Client for HttpClient {
    fn kustomize_patch_for_execution(&self, outdir: &Path) -> String {
        KUSTOMIZE_PATCH
            .replace("__ORCHESTRATOR_URL__", self.orchestrator_url.as_str())
            .replace("__EXECUTION_ID__", &self.execution_id.to_string())
            .replace("__EXECUTION_TOKEN__", &self.execution_token.to_string())
            .replace("__OUTDIR__", &outdir.to_string_lossy())
    }

    async fn update_status(
        &self,
        status: Status,
        exit_status: Option<ExitStatus>,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        let url = self
            .orchestrator_url
            .join(&format!("test-execution/{}/status", self.execution_id))?;

        let payload = SetStatusPayload {
            status,
            exit_code: exit_status
                .and_then(|status| status.code())
                .map(|code| code as u8),
            message,
        };

        info!(id=%self.execution_id, %status, "updating execution status");
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(self.execution_token)
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

    async fn generate_upload_urls(&self) -> anyhow::Result<UploadUrls> {
        let url = self.orchestrator_url.join(&format!(
            "test-execution/{}/generate-upload-urls",
            self.execution_id
        ))?;

        info!(id=%self.execution_id, "requesting upload URLs");
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(self.execution_token)
            .json(&GenerateUploadUrlsPayload {})
            .send()
            .await
            .context("failed to request signed URLs for artifact upload")?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "generate-upload-urls failed for {}; ({status_code}): {body}",
                self.execution_id,
            ));
        }

        Ok(resp.json().await?)
    }

    async fn upload_to_signed_url(&self, url: &str, body: Vec<u8>) -> anyhow::Result<()> {
        info!(id=%self.execution_id, "uploading artifact");
        let resp = reqwest::Client::new()
            .put(url)
            .body(body)
            .send()
            .await
            .context("failed to PUT to signed upload URL")?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("signed upload PUT failed ({status_code}): {body}",));
        }

        Ok(())
    }

    async fn fetch_environment_config(&self) -> anyhow::Result<Vec<u8>> {
        self.fetch_config("environment-config").await
    }

    async fn fetch_scenario_config(&self) -> anyhow::Result<Vec<u8>> {
        self.fetch_config("scenario-config").await
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

    pub struct UploadCall {
        pub url: String,
        pub body: Vec<u8>,
    }

    pub struct MockClient {
        status_updates: RwLock<Vec<StatusUpdateArgs>>,
        uploads: RwLock<Vec<UploadCall>>,
        update_should_fail: bool,
        log_file_url: String,
        output_zip_url: String,
    }

    impl Default for MockClient {
        fn default() -> Self {
            Self {
                status_updates: RwLock::new(Vec::new()),
                uploads: RwLock::new(Vec::new()),
                update_should_fail: false,
                log_file_url: "http://mock/log".to_owned(),
                output_zip_url: "http://mock/zip".to_owned(),
            }
        }
    }

    impl MockClient {
        pub fn failing() -> Self {
            Self {
                update_should_fail: true,
                ..Default::default()
            }
        }

        pub fn read_updates<F>(&self, closure: F)
        where
            F: FnOnce(RwLockReadGuard<Vec<StatusUpdateArgs>>),
        {
            let updates = self.status_updates.read().unwrap();
            closure(updates)
        }

        pub fn read_uploads<F>(&self, closure: F)
        where
            F: FnOnce(RwLockReadGuard<Vec<UploadCall>>),
        {
            let uploads = self.uploads.read().unwrap();
            closure(uploads)
        }
    }

    impl Client for MockClient {
        fn kustomize_patch_for_execution(&self, _outdir: &Path) -> String {
            KUSTOMIZE_PATCH.to_string()
        }

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

        async fn generate_upload_urls(&self) -> anyhow::Result<UploadUrls> {
            if self.update_should_fail {
                return Err(anyhow::anyhow!("mock upload urls failure"));
            }
            Ok(UploadUrls {
                log_file_url: self.log_file_url.clone(),
                output_zip_url: self.output_zip_url.clone(),
            })
        }

        async fn upload_to_signed_url(&self, url: &str, body: Vec<u8>) -> anyhow::Result<()> {
            if self.update_should_fail {
                return Err(anyhow::anyhow!("mock upload failure"));
            }
            self.uploads.write().unwrap().push(UploadCall {
                url: url.to_owned(),
                body,
            });
            Ok(())
        }

        async fn fetch_environment_config(&self) -> anyhow::Result<Vec<u8>> {
            if self.update_should_fail {
                return Err(anyhow::anyhow!("mock fetch environment config failure"));
            }
            Ok(Vec::new())
        }

        async fn fetch_scenario_config(&self) -> anyhow::Result<Vec<u8>> {
            if self.update_should_fail {
                return Err(anyhow::anyhow!("mock fetch scenario config failure"));
            }
            Ok(Vec::new())
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

    #[tokio::test]
    async fn mock_generate_upload_urls_returns_canned_urls() {
        let client = MockClient::default();
        let urls = client.generate_upload_urls().await.unwrap();
        assert_eq!(urls.log_file_url, "http://mock/log");
        assert_eq!(urls.output_zip_url, "http://mock/zip");
    }

    #[tokio::test]
    async fn mock_upload_to_signed_url_records_calls() {
        let client = MockClient::default();
        client
            .upload_to_signed_url("http://mock/target", b"payload".to_vec())
            .await
            .unwrap();

        client.read_uploads(|uploads| {
            assert_eq!(uploads.len(), 1);
            assert_eq!(uploads[0].url, "http://mock/target");
            assert_eq!(uploads[0].body, b"payload");
        });
    }

    #[tokio::test]
    async fn mock_failing_client_fails_upload_methods() {
        let client = MockClient::failing();
        assert!(client.generate_upload_urls().await.is_err());
        assert!(
            client
                .upload_to_signed_url("url", Vec::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn mock_failing_client_fails_fetch_methods() {
        let client = MockClient::failing();
        assert!(client.fetch_environment_config().await.is_err());
        assert!(client.fetch_scenario_config().await.is_err());
    }
}

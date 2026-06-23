use rep_orchestrator_shared::{
    FILE_PROVIDERS_LABEL, LOG_COLLECTION_LABEL, OTEL_LABEL, OtelConfig,
    RTF_OTEL_COLLECTOR_GRPC_VAR, RTF_OTEL_COLLECTOR_HTTP_VAR,
    payload::{GenerateUploadUrlsPayload, SetStatusPayload},
    status::Status,
    upload_urls::UploadUrls,
};
use reqwest::Url;
use std::{path::Path, process::ExitStatus};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

/// Errors produced when communicating with the REP orchestrator HTTP API.
#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to fetch {kind} config: {message}")]
    FetchConfig { kind: &'static str, message: String },

    #[error("failed to request artifact upload URLs: {message}")]
    GenerateUploadUrls { message: String },

    #[error("failed to upload artifact: {message}")]
    Upload { message: String },

    #[error("failed to update execution status: {message}")]
    UpdateStatus { message: String },
}

const KUSTOMIZE_PATCH: &str = include_str!("../resources/kustomization.yaml");

pub trait Client: Send + Sync {
    fn kustomize_patch_for_execution(
        &self,
        outdir: &Path,
        toolbox_image_pull_policy: &str,
        otel: &OtelConfig,
    ) -> String;

    /// Update the [Status] of the current test execution, with an optional `message` to write to the database.
    ///
    /// If there was a subprocess associated with this status update, its [ExitStatus] may be included for additional context.
    fn update_status(
        &self,
        status: Status,
        exit_status: Option<ExitStatus>,
        message: Option<String>,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Request signed URLs from the orchestrator for uploading the log file and output zip
    /// associated with the current test execution.
    fn generate_upload_urls(&self) -> impl Future<Output = Result<UploadUrls, Error>> + Send;

    /// PUT `body` to a previously-issued signed upload URL.
    fn upload_to_signed_url(
        &self,
        url: &str,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Fetch the resolved environment YAML for the current test execution.
    fn fetch_environment_config(&self) -> impl Future<Output = Result<Vec<u8>, Error>> + Send;

    /// Fetch the resolved scenario YAML for the current test execution.
    fn fetch_scenario_config(&self) -> impl Future<Output = Result<Vec<u8>, Error>> + Send;
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

    async fn fetch_config(&self, kind: &'static str) -> Result<Vec<u8>, Error> {
        let url = self
            .orchestrator_url
            .join(&format!(
                "test-execution/{}/{kind}-config",
                self.execution_id
            ))
            .map_err(|e| Error::FetchConfig {
                kind,
                message: e.to_string(),
            })?;

        info!(id=%self.execution_id, "fetching {kind} config");
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(self.execution_token)
            .send()
            .await
            .map_err(|e| Error::FetchConfig {
                kind,
                message: e.to_string(),
            })?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::FetchConfig {
                kind,
                message: format!(
                    "{kind} fetch failed for {}; ({status_code}): {body}",
                    self.execution_id,
                ),
            });
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| Error::FetchConfig {
                kind,
                message: e.to_string(),
            })
    }
}

impl Client for HttpClient {
    fn kustomize_patch_for_execution(
        &self,
        outdir: &Path,
        toolbox_image_pull_policy: &str,
        otel: &OtelConfig,
    ) -> String {
        KUSTOMIZE_PATCH
            .replace("__ORCHESTRATOR_URL__", self.orchestrator_url.as_str())
            .replace("__EXECUTION_ID__", &self.execution_id.to_string())
            .replace("__EXECUTION_TOKEN__", &self.execution_token.to_string())
            .replace("__OUTDIR__", &outdir.to_string_lossy())
            .replace("__IMAGE_PULL_POLICY__", toolbox_image_pull_policy)
            .replace("__FILE_PROVIDERS_LABEL__", FILE_PROVIDERS_LABEL)
            .replace("__LOG_COLLECTION_LABEL__", LOG_COLLECTION_LABEL)
            .replace("__OTEL_LABEL__", OTEL_LABEL)
            .replace(
                "__RTF_OTEL_COLLECTOR_GRPC_VAR__",
                RTF_OTEL_COLLECTOR_GRPC_VAR,
            )
            .replace(
                "__RTF_OTEL_COLLECTOR_HTTP_VAR__",
                RTF_OTEL_COLLECTOR_HTTP_VAR,
            )
            .replace("__RTF_OTEL_COLLECTOR_GRPC__", &otel.grpc)
            .replace("__RTF_OTEL_COLLECTOR_HTTP__", &otel.http)
    }

    async fn update_status(
        &self,
        status: Status,
        exit_status: Option<ExitStatus>,
        message: Option<String>,
    ) -> Result<(), Error> {
        let url = self
            .orchestrator_url
            .join(&format!("test-execution/{}/status", self.execution_id))
            .map_err(|e| Error::UpdateStatus {
                message: e.to_string(),
            })?;

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
            .map_err(|e| Error::UpdateStatus {
                message: e.to_string(),
            })?;

        let status_code = resp.status();
        if status_code.is_success() {
            info!(id=%self.execution_id, %status, "update successful");
            Ok(())
        } else {
            let body = match resp.text().await {
                Ok(body) => body,
                Err(err) => format!("[body could not be read: {err}]"),
            };
            Err(Error::UpdateStatus {
                message: format!(
                    "status update to {status} failed for {}; ({status_code}): {body}",
                    self.execution_id
                ),
            })
        }
    }

    async fn generate_upload_urls(&self) -> Result<UploadUrls, Error> {
        let url = self
            .orchestrator_url
            .join(&format!(
                "test-execution/{}/generate-upload-urls",
                self.execution_id
            ))
            .map_err(|e| Error::GenerateUploadUrls {
                message: e.to_string(),
            })?;

        info!(id=%self.execution_id, "requesting upload URLs");
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(self.execution_token)
            .json(&GenerateUploadUrlsPayload {})
            .send()
            .await
            .map_err(|e| Error::GenerateUploadUrls {
                message: e.to_string(),
            })?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::GenerateUploadUrls {
                message: format!(
                    "generate-upload-urls failed for {}; ({status_code}): {body}",
                    self.execution_id,
                ),
            });
        }

        resp.json().await.map_err(|e| Error::GenerateUploadUrls {
            message: e.to_string(),
        })
    }

    async fn upload_to_signed_url(&self, url: &str, body: Vec<u8>) -> Result<(), Error> {
        info!(id=%self.execution_id, "uploading artifact");
        let resp = reqwest::Client::new()
            .put(url)
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Upload {
                message: e.to_string(),
            })?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upload {
                message: format!("signed upload PUT failed ({status_code}): {body}"),
            });
        }

        Ok(())
    }

    async fn fetch_environment_config(&self) -> Result<Vec<u8>, Error> {
        self.fetch_config("environment").await
    }

    async fn fetch_scenario_config(&self) -> Result<Vec<u8>, Error> {
        self.fetch_config("scenario").await
    }
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use rep_orchestrator_shared::status::Status;
    use std::{
        process::ExitStatus,
        sync::{RwLock, RwLockReadGuard},
    };

    pub struct UploadCall {
        pub url: String,
        pub body: Vec<u8>,
    }

    pub struct MockClient {
        status_updates: RwLock<Vec<Status>>,
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
            F: FnOnce(RwLockReadGuard<Vec<Status>>),
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
        fn kustomize_patch_for_execution(
            &self,
            _outdir: &Path,
            _pull_policy: &str,
            _otel: &OtelConfig,
        ) -> String {
            KUSTOMIZE_PATCH.to_string()
        }

        async fn update_status(
            &self,
            status: Status,
            _exit_status: Option<ExitStatus>,
            _message: Option<String>,
        ) -> Result<(), Error> {
            if self.update_should_fail {
                return Err(Error::UpdateStatus {
                    message: "mock update failure".to_owned(),
                });
            }
            self.status_updates.write().unwrap().push(status);

            Ok(())
        }

        async fn generate_upload_urls(&self) -> Result<UploadUrls, Error> {
            if self.update_should_fail {
                return Err(Error::GenerateUploadUrls {
                    message: "mock upload urls failure".to_owned(),
                });
            }

            Ok(UploadUrls {
                log_file_url: self.log_file_url.clone(),
                output_zip_url: self.output_zip_url.clone(),
            })
        }

        async fn upload_to_signed_url(&self, url: &str, body: Vec<u8>) -> Result<(), Error> {
            if self.update_should_fail {
                return Err(Error::Upload {
                    message: "mock upload failure".to_owned(),
                });
            }
            self.uploads.write().unwrap().push(UploadCall {
                url: url.to_owned(),
                body,
            });

            Ok(())
        }

        async fn fetch_environment_config(&self) -> Result<Vec<u8>, Error> {
            if self.update_should_fail {
                return Err(Error::FetchConfig {
                    kind: "environment",
                    message: "mock fetch environment config failure".to_owned(),
                });
            }

            Ok(Vec::new())
        }

        async fn fetch_scenario_config(&self) -> Result<Vec<u8>, Error> {
            if self.update_should_fail {
                return Err(Error::FetchConfig {
                    kind: "scenario",
                    message: "mock fetch scenario config failure".to_owned(),
                });
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

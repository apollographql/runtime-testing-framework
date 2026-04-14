//! Minimal GCS client for generating pre-signed URLs
use crate::config::Config;
use google_cloud_auth::{build_errors, credentials::Builder, signer::Signer};
use google_cloud_storage::{
    builder::storage::SignedUrlBuilder, client::Storage, error::SigningError, http,
};
use std::{future::Future, time::Duration};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to build GCS download client: {0}")]
    DownloadClientInit(Box<dyn std::error::Error + Send>),

    #[error("GCS error: {0}")]
    GCS(#[from] google_cloud_storage::Error),

    #[error("GCS client initialization failed: {0}")]
    Init(#[from] build_errors::Error),

    #[error("failed to generate signed GCS URL: {0}")]
    SignedUrl(#[from] SigningError),
}

/// Trait for a GCS client capable of producing pre-signed upload and download URLs.
pub trait Client: Send + Sync + 'static {
    /// Return a pre-signed URL that allows a PUT of the given object into this client's bucket.
    fn signed_upload_url(&self, object: &str) -> impl Future<Output = Result<String>> + Send;

    /// Return a pre-signed URL that allows a GET of the given object from this client's bucket.
    fn signed_download_url(&self, object: &str) -> impl Future<Output = Result<String>> + Send;

    /// Download the contents of an object as raw bytes.
    fn download_bytes(&self, object: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

#[derive(Debug, Clone)]
pub struct RealClient {
    signer: Signer,
    bucket: String,
    ttl: Duration,
}

impl Client for RealClient {
    async fn signed_upload_url(&self, object: &str) -> Result<String> {
        let url = SignedUrlBuilder::for_object(&self.bucket, object)
            .with_method(http::Method::PUT)
            .with_expiration(self.ttl)
            .sign_with(&self.signer)
            .await?;

        Ok(url)
    }

    async fn signed_download_url(&self, object: &str) -> Result<String> {
        let url = SignedUrlBuilder::for_object(&self.bucket, object)
            .with_method(http::Method::GET)
            .with_expiration(self.ttl)
            .sign_with(&self.signer)
            .await?;

        Ok(url)
    }

    async fn download_bytes(&self, object: &str) -> Result<Vec<u8>> {
        let client = Storage::builder()
            .build()
            .await
            .map_err(|e| Error::DownloadClientInit(Box::new(e)))?;

        let mut resp = client.read_object(&self.bucket, object).send().await?;
        let mut buf = Vec::new();

        while let Some(chunk) = resp.next().await.transpose()? {
            buf.extend_from_slice(&chunk);
        }

        Ok(buf)
    }
}

/// A mock GCS client that returns deterministic URLs based on a base URL.
#[derive(Debug, Clone)]
pub struct MockClient {
    base_url: String,
    bucket: String,
}

impl MockClient {
    pub fn new(base_url: String, bucket: String) -> Self {
        Self { base_url, bucket }
    }

    fn url_for_object(&self, object: &str) -> String {
        format!("{}/{}/{object}", self.base_url, self.bucket)
    }
}

impl Client for MockClient {
    async fn signed_upload_url(&self, object: &str) -> Result<String> {
        Ok(self.url_for_object(object))
    }

    async fn signed_download_url(&self, object: &str) -> Result<String> {
        Ok(self.url_for_object(object))
    }

    async fn download_bytes(&self, object: &str) -> Result<Vec<u8>> {
        // (innes) The timeout error stuff here is a little silly, but it allows us to avoid adding
        // a mock-only variant to Error.

        let resp = reqwest::get(self.url_for_object(object))
            .await
            .map_err(google_cloud_storage::Error::timeout)?;

        let bytes = resp
            .bytes()
            .await
            .map_err(google_cloud_storage::Error::timeout)?;

        Ok(bytes.to_vec())
    }
}

#[derive(Debug, Clone)]
pub enum GCSClient {
    Real(RealClient),
    Mock(MockClient),
}

impl GCSClient {
    /// Initialise the appropriate client from config.
    ///
    /// Uses [MockClient] when `RTF_MOCK_GCS_URL` is set; otherwise attempts to initialise a
    /// [RealClient] using Application Default Credentials.
    pub async fn new_from_config(cfg: &Config) -> Result<Self> {
        let client = match &cfg.mock_gcs_url {
            Some(url) => Self::Mock(MockClient {
                base_url: url.clone(),
                bucket: cfg.gcs_bucket.clone(),
            }),

            None => Self::Real(RealClient {
                signer: Builder::default().build_signer()?,
                bucket: cfg.gcs_bucket.clone(),
                ttl: Duration::from_secs(cfg.gcs_url_ttl_secs),
            }),
        };

        Ok(client)
    }

    /// Construct a [MockClient] with explicit parameters.
    #[cfg(test)]
    pub fn new_mock(base_url: &str, bucket: &str) -> Self {
        Self::Mock(MockClient {
            base_url: base_url.into(),
            bucket: bucket.into(),
        })
    }
}

impl Client for GCSClient {
    async fn signed_upload_url(&self, object: &str) -> Result<String> {
        match self {
            Self::Real(c) => c.signed_upload_url(object).await,
            Self::Mock(c) => c.signed_upload_url(object).await,
        }
    }

    async fn signed_download_url(&self, object: &str) -> Result<String> {
        match self {
            Self::Real(c) => c.signed_download_url(object).await,
            Self::Mock(c) => c.signed_download_url(object).await,
        }
    }

    async fn download_bytes(&self, object: &str) -> Result<Vec<u8>> {
        match self {
            Self::Real(c) => c.download_bytes(object).await,
            Self::Mock(c) => c.download_bytes(object).await,
        }
    }
}

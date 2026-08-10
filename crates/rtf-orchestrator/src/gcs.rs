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
    fn signed_upload_url(&self, object: String) -> impl Future<Output = Result<String>> + Send;

    /// Return a pre-signed URL that allows a GET of the given object from this client's bucket.
    fn signed_download_url(&self, object: String) -> impl Future<Output = Result<String>> + Send;

    /// Download the contents of an object as raw bytes.
    fn download_bytes(&self, object: String) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

#[derive(Debug, Clone)]
pub struct RealClient {
    signer: Signer,
    bucket: String,
    ttl: Duration,
}

impl Client for RealClient {
    async fn signed_upload_url(&self, object: String) -> Result<String> {
        let url = SignedUrlBuilder::for_object(&self.bucket, object)
            .with_method(http::Method::PUT)
            .with_expiration(self.ttl)
            .sign_with(&self.signer)
            .await?;

        Ok(url)
    }

    async fn signed_download_url(&self, object: String) -> Result<String> {
        let url = SignedUrlBuilder::for_object(&self.bucket, object)
            .with_method(http::Method::GET)
            .with_expiration(self.ttl)
            .sign_with(&self.signer)
            .await?;

        Ok(url)
    }

    async fn download_bytes(&self, object: String) -> Result<Vec<u8>> {
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
    internal_url: String,
    public_url: String,
    bucket: String,
    #[cfg(test)]
    canned_response: Option<String>,
}

impl MockClient {
    pub fn new(
        internal_url: impl Into<String>,
        public_url: impl Into<String>,
        bucket: impl Into<String>,
    ) -> Self {
        Self {
            internal_url: internal_url.into(),
            public_url: public_url.into(),
            bucket: bucket.into(),
            #[cfg(test)]
            canned_response: None,
        }
    }

    pub fn internal_url_for_object(&self, object: String) -> String {
        format!("{}/{}/{object}", self.internal_url, self.bucket)
    }

    pub fn public_url_for_object(&self, object: String) -> String {
        format!("{}/{}/{object}", self.public_url, self.bucket)
    }
}

impl Client for MockClient {
    async fn signed_upload_url(&self, object: String) -> Result<String> {
        Ok(self.internal_url_for_object(object))
    }

    async fn signed_download_url(&self, object: String) -> Result<String> {
        Ok(self.public_url_for_object(object))
    }

    async fn download_bytes(&self, object: String) -> Result<Vec<u8>> {
        #[cfg(test)]
        if let Some(resp) = self.canned_response.clone() {
            return Ok(resp.into_bytes());
        }

        // (innes) The timeout error stuff here is a little silly, but it allows us to avoid adding
        // a mock-only variant to Error.

        let resp = reqwest::get(self.internal_url_for_object(object))
            .await
            .map_err(google_cloud_storage::Error::timeout)?;

        if !resp.status().is_success() {
            return Err(Error::GCS(google_cloud_storage::Error::timeout(
                resp.status().to_string(),
            )));
        }

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
        let client = match (&cfg.mock_internal_gcs_url, &cfg.mock_public_gcs_url) {
            (Some(internal), Some(public)) => Self::Mock(MockClient::new(
                internal.clone(),
                public.clone(),
                cfg.gcs_bucket.clone(),
            )),

            (None, None) => Self::Real(RealClient {
                signer: Builder::default().build_signer()?,
                bucket: cfg.gcs_bucket.clone(),
                ttl: Duration::from_secs(cfg.gcs_url_ttl_secs),
            }),

            _ => panic!("when setting mock GCS URLs, both must be set"),
        };

        Ok(client)
    }

    /// Construct a [MockClient] with explicit parameters.
    #[cfg(test)]
    pub fn new_mock(
        internal_url: &str,
        public_url: &str,
        bucket: &str,
        canned_response: Option<String>,
    ) -> Self {
        let mut client = MockClient::new(internal_url, public_url, bucket);
        client.canned_response = canned_response;

        Self::Mock(client)
    }
}

impl Client for GCSClient {
    async fn signed_upload_url(&self, object: String) -> Result<String> {
        match self {
            Self::Real(c) => c.signed_upload_url(object).await,
            Self::Mock(c) => c.signed_upload_url(object).await,
        }
    }

    async fn signed_download_url(&self, object: String) -> Result<String> {
        match self {
            Self::Real(c) => c.signed_download_url(object).await,
            Self::Mock(c) => c.signed_download_url(object).await,
        }
    }

    async fn download_bytes(&self, object: String) -> Result<Vec<u8>> {
        match self {
            Self::Real(c) => c.download_bytes(object).await,
            Self::Mock(c) => c.download_bytes(object).await,
        }
    }
}

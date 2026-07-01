//! A lightweight Prometheus API client
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde_json::Value;

/// Errors that can occur when building or using a [`PrometheusClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An underlying error from the reqwest crate
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    /// Prometheus returned an API-level error response.
    ///
    /// See the [Prometheus HTTP API docs](https://prometheus.io/docs/prometheus/latest/querying/api/#format-overview).
    #[error("prometheus query failed ({status}): {error_type}: {message}")]
    Api {
        /// 400 (bad_data), 422 (execution error), or 503 (timeout/abort)
        status: StatusCode,
        /// Prometheus's `errorType` field, e.g. `bad_data`, `execution`, `timeout`
        error_type: String,
        /// Prometheus's `error` field — human-readable message
        message: String,
    },
}

/// Alias for a [Result][std::result::Result] where the error variant is an [Error].
pub type Result<T> = std::result::Result<T, Error>;

/// A lightweight Prometheus API HTTP client
#[derive(Debug)]
pub struct PrometheusClient {
    url: String,
    http_client: Client,
}

impl PrometheusClient {
    /// Build a new client using the supplied prometheus url
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            http_client: Client::new(),
        }
    }

    /// Executes a Prometheus range query.
    ///
    /// See the [Prometheus HTTP API docs](https://prometheus.io/docs/prometheus/latest/querying/api/#range-queries).
    pub async fn query_range(
        &self,
        query: &str,
        step: &str,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Value> {
        let resp = self
            .http_client
            .get(format!("{}/api/v1/query_range", self.url))
            .query(&[
                ("query", query),
                ("start", &start.timestamp().to_string()),
                ("end", &end.timestamp().to_string()),
                ("step", step),
            ])
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp.json().await?;

        handle_response(status, body)
    }
}

/// Turns a Prometheus HTTP response's status and JSON body into a result, surfacing the
/// `errorType`/`error` fields Prometheus returns on non-2xx responses instead of silently
/// treating them as an empty result.
fn handle_response(status: StatusCode, body: Value) -> Result<Value> {
    if !status.is_success() {
        return Err(Error::Api {
            status,
            error_type: body["errorType"].as_str().unwrap_or("unknown").to_owned(),
            message: body["error"]
                .as_str()
                .unwrap_or("no error message returned")
                .to_owned(),
        });
    }

    Ok(body["data"]["result"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use simple_test_case::test_case;

    #[test_case(
        StatusCode::OK,
        json!({"status": "success", "data": {"resultType": "matrix", "result": ["ok"]}}),
        Ok(json!(["ok"]));
        "200 success returns data.result"
    )]
    #[test_case(
        StatusCode::BAD_REQUEST,
        json!({"status": "error", "errorType": "bad_data", "error": "parse error at char 1"}),
        Err(Error::Api {
            status: StatusCode::BAD_REQUEST,
            error_type: "bad_data".to_owned(),
            message: "parse error at char 1".to_owned(),
        });
        "400 bad_data surfaces errorType and error"
    )]
    #[test_case(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"status": "error", "errorType": "execution", "error": "query timed out in expression evaluation"}),
        Err(Error::Api {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            error_type: "execution".to_owned(),
            message: "query timed out in expression evaluation".to_owned(),
        });
        "422 execution error surfaces errorType and error"
    )]
    #[test_case(
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"status": "error", "errorType": "timeout", "error": "query aborted"}),
        Err(Error::Api {
            status: StatusCode::SERVICE_UNAVAILABLE,
            error_type: "timeout".to_owned(),
            message: "query aborted".to_owned(),
        });
        "503 timeout surfaces errorType and error"
    )]
    #[test_case(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({}),
        Err(Error::Api {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error_type: "unknown".to_owned(),
            message: "no error message returned".to_owned(),
        });
        "non-2xx with no errorType/error falls back to defaults"
    )]
    #[test]
    fn handle_response_cases(status: StatusCode, body: Value, expected: Result<Value>) {
        let actual = handle_response(status, body);

        match (actual, expected) {
            (Ok(a), Ok(e)) => assert_eq!(a, e),
            (Err(a), Err(e)) => assert_eq!(a.to_string(), e.to_string()),
            (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
        }
    }

    // Proves query_range actually attempts a real HTTP call and maps a transport failure to
    // Error::Reqwest, by pointing at an unroutable address — same trick as github.rs's tests.
    #[tokio::test]
    async fn query_range_transport_failure_returns_reqwest_error() {
        let client = PrometheusClient::new("http://127.0.0.1:1");

        let result = client
            .query_range("up", "15s", &Utc::now(), &Utc::now())
            .await;

        assert!(
            matches!(result, Err(Error::Reqwest(_))),
            "expected Reqwest error, got {result:?}"
        );
    }
}

use chrono::{DateTime, Utc};
use rtf_integrations::prometheus::PrometheusClient;
use serde_json::Value;

pub use rtf_integrations::prometheus::Error;

pub trait Client: Send + Sync {
    /// Executes a Prometheus range query.
    fn query_range(
        &self,
        query: &str,
        step: &str,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> impl Future<Output = Result<Value, Error>> + Send;
}

pub struct HttpClient(PrometheusClient);

impl HttpClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self(PrometheusClient::new(url))
    }
}

impl Client for HttpClient {
    async fn query_range(
        &self,
        query: &str,
        step: &str,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Value, Error> {
        self.0.query_range(query, step, start, end).await
    }
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use reqwest::StatusCode;
    use std::sync::{Arc, RwLock};

    #[derive(Debug, Clone, PartialEq)]
    pub struct QueryRangeCall {
        pub query: String,
        pub step: String,
    }

    struct MockState {
        calls: Vec<QueryRangeCall>,
        should_fail: bool,
        result: Value,
    }

    impl Default for MockState {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                should_fail: false,
                result: Value::Array(Vec::new()),
            }
        }
    }

    #[derive(Clone, Default)]
    pub struct MockClient {
        state: Arc<RwLock<MockState>>,
    }

    impl MockClient {
        pub fn failing() -> Self {
            Self {
                state: Arc::new(RwLock::new(MockState {
                    should_fail: true,
                    ..Default::default()
                })),
            }
        }

        fn record_call(&self, call: QueryRangeCall) {
            self.state.write().unwrap().calls.push(call);
        }

        pub fn read_calls<F>(&self, closure: F)
        where
            F: FnOnce(&[QueryRangeCall]),
        {
            let state = self.state.read().unwrap();
            closure(&state.calls)
        }
    }

    impl Client for MockClient {
        async fn query_range(
            &self,
            query: &str,
            step: &str,
            _start: &DateTime<Utc>,
            _end: &DateTime<Utc>,
        ) -> Result<Value, Error> {
            let (should_fail, result) = {
                let state = self.state.read().unwrap();
                (state.should_fail, state.result.clone())
            };

            if should_fail {
                return Err(Error::Api {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    error_type: "mock_failure".to_owned(),
                    message: "mock prometheus failure".to_owned(),
                });
            }

            self.record_call(QueryRangeCall {
                query: query.to_owned(),
                step: step.to_owned(),
            });

            Ok(result)
        }
    }
}

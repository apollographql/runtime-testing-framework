use rtf_config::formats::PrometheusQuery;
use serde::{Deserialize, Serialize};

pub mod payload;
pub mod status;
pub mod summary;
pub mod test_plan;
pub mod upload_urls;

pub const ORCHESTRATOR_URL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_URL";
pub const EXECUTION_ID_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_EXECUTION_ID";
pub const EXECUTION_TOKEN_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN";

pub const EXECUTION_ID_LABEL: &str = "rtf.io/execution-id";

/// Name of the Job the orchestrator creates to run the scenario container. `rep-orchestrator-cli`
/// looks up this same Job by name to read its `.status.startTime`/`.status.completionTime`.
pub const SCENARIO_JOB_NAME: &str = "scenario-execution";

pub const RTF_OTEL_COLLECTOR_GRPC_VAR: &str = "RTF_OTEL_COLLECTOR_GRPC";
pub const RTF_OTEL_COLLECTOR_HTTP_VAR: &str = "RTF_OTEL_COLLECTOR_HTTP";

// Re-exported so other rep crates do not need to depend directly on rtf-config
pub use rtf_config::{FILE_PROVIDERS_LABEL, LOG_COLLECTION_LABEL, OTEL_LABEL};

/// OTEL collector endpoints injected into workload pods by the orchestrator.
#[derive(Clone, Debug)]
pub struct OtelConfig {
    pub grpc: String,
    pub http: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputCollectionResponse {
    pub prometheus: PrometheusQueries,
}

/// Response for the prometheus queries output collection config endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusQueries {
    pub environment: Vec<PrometheusQuery>,
    pub scenario: Vec<PrometheusQuery>,
}

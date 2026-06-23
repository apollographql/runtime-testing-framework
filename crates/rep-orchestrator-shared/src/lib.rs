pub mod payload;
pub mod status;
pub mod summary;
pub mod test_plan;
pub mod upload_urls;

pub const ORCHESTRATOR_URL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_URL";
pub const EXECUTION_ID_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_EXECUTION_ID";
pub const EXECUTION_TOKEN_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN";

pub const EXECUTION_ID_LABEL: &str = "rtf.io/execution-id";

pub const REP_OTEL_COLLECTOR_GRPC_VAR: &str = "REP_OTEL_COLLECTOR_GRPC";
pub const REP_OTEL_COLLECTOR_HTTP_VAR: &str = "REP_OTEL_COLLECTOR_HTTP";

/// OTEL collector endpoints injected into workload pods by the orchestrator.
#[derive(Clone, Debug)]
pub struct OtelConfig {
    pub grpc: String,
    pub http: String,
}

// Re-exported so other rep crates do not need to depend directly on rtf-config
pub use rtf_config::{FILE_PROVIDERS_LABEL, LOG_COLLECTION_LABEL, OTEL_LABEL};

use crate::db::ClusterId;
use rtf_config::context::Context;
use serde::Deserialize;
use std::{collections::HashMap, env, fs, net::SocketAddr, sync::LazyLock};

const APOLLO_KEY_VAR: &str = "RTF_APOLLO_KEY";
const CONFIG_PATH_VAR: &str = "RTF_CONFIG_PATH";
const GH_APP_ID_VAR: &str = "RTF_GITHUB_APP_ID";
const GH_PEM_VAR: &str = "RTF_GITHUB_APP_PRIVATE_KEY_PEM";
const KUBECONFIG_MOUNT_ROOT: &str = "/etc/rtf-orchestrator/kubeconfigs";

// Default values for the per-user execution config options.
//
// These are used in the trigger endpoint to gate trigger requests from users.
const DEFAULT_MAX_CONCURRENT_RUNS: usize = 1;
const DEFAULT_MAX_QUEUED_RUNS: usize = 5;
const DEFAULT_MAX_QUEUED_EXECUTIONS: usize = 100;
const DEFAULT_MAX_RUNS_PER_HOUR: usize = 10;

static CONFIG: LazyLock<Config> = LazyLock::new(|| match Config::try_parse_from_env() {
    Ok(cfg) => cfg,
    Err(e) => panic!("invalid config file: {e}"),
});

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    pub admins_path: String,
    pub db: DbConfig,
    pub server: ServerConfig,
    pub workload_clusters: WorkloadClusters,
    pub toolbox: ToolboxConfig,
    pub otel: OtelConfig,
    pub gcs: GcsConfig,
    // The apollo and github API credentials we hold are passed via environment variables rather
    // than as part of the config file directly (see `try_parse_from_env`).
    #[serde(skip)]
    pub apollo_key: String,
    #[serde(skip)]
    pub github: GithubConfig,
}

impl Config {
    /// Attempt to load and parse the server config file from the path specified by
    /// `CONFIG_PATH_VAR` and combine it with the credentials provided via the following env vars:
    /// - `APOLLO_KEY_VAR`
    /// - `GH_APP_ID_VAR`
    /// - `GH_PEM_VAR`
    pub fn try_parse_from_env() -> Result<Self, serde_yaml::Error> {
        let expect_env = |var| env::var(var).unwrap_or_else(|_| panic!("{var} not set"));

        let mut cfg = match fs::read_to_string(expect_env(CONFIG_PATH_VAR)) {
            Ok(text) => Self::try_parse(&text)?,
            Err(e) => panic!("unable to read config file: {e}"),
        };

        cfg.apollo_key = expect_env(APOLLO_KEY_VAR);
        cfg.github = GithubConfig {
            app_id: expect_env(GH_APP_ID_VAR).parse().unwrap(),
            app_private_key_pem: expect_env(GH_PEM_VAR),
        };

        Ok(cfg)
    }

    fn try_parse(text: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(text)
    }

    pub fn get() -> &'static Self {
        &CONFIG
    }

    pub fn socket_addr(&self) -> SocketAddr {
        match format!("{}:{}", self.server.host, self.server.port).parse() {
            Ok(sa) => sa,
            Err(e) => panic!("invalid socker addr from config: {e}"),
        }
    }

    pub fn toolbox_image(&self) -> String {
        format!(
            "{}:{}",
            self.toolbox.image_repository, self.toolbox.image_tag
        )
    }

    pub fn server_context(&self) -> Context {
        let mut ctx = Context::new();
        // apollo_sudo always true; graphos_staging always false for the orchestrator
        ctx.with_platform_config(&self.apollo_key, false, true);
        ctx.with_github_app_config(self.github.app_id, self.github.app_private_key_pem.clone());

        ctx
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GithubConfig {
    pub app_id: u64,
    pub app_private_key_pem: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DbConfig {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub user: String,
    #[serde(default)]
    pub pass: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub body_limit_mb: usize,
    pub orchestrator_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkloadClusters {
    pub default_cluster: String,
    pub max_queued_executions: usize,
    pub available_clusters: Vec<WorkloadClusterConfig>,
}

impl WorkloadClusters {
    pub fn available_clusters(&self) -> Vec<ClusterId> {
        self.available_clusters
            .iter()
            .map(|c| ClusterId::new(&c.name))
            .collect()
    }

    pub fn default_workload_cluster(&self) -> ClusterId {
        ClusterId::new(&self.default_cluster)
    }

    pub fn per_cluster_config(&self) -> HashMap<ClusterId, WorkloadClusterConfig> {
        self.available_clusters
            .iter()
            .map(|c| (ClusterId::new(&c.name), c.clone()))
            .collect()
    }

    pub fn max_concurrent_executions(&self) -> HashMap<ClusterId, usize> {
        self.available_clusters
            .iter()
            .map(|c| (ClusterId::new(&c.name), c.execution.max_concurrent))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkloadClusterConfig {
    pub name: String,
    pub kubeconfig_secret_name: String,
    pub workload_context: String,
    pub execution: ClusterExecutionConfig,
}

impl WorkloadClusterConfig {
    pub fn kubeconfig_path(&self) -> String {
        format!("{KUBECONFIG_MOUNT_ROOT}/{}/config", self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClusterExecutionConfig {
    pub max_concurrent: usize,
    pub failed_execution_ttl_secs: u64,
    pub retry_window_secs: u64,
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub per_user: PerUserExecutionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PerUserExecutionConfig {
    #[serde(default = "max_concurrent_runs")]
    pub max_concurrent_runs: usize,
    #[serde(default = "max_queued_runs")]
    pub max_queued_runs: usize,
    #[serde(default = "max_queued_executions")]
    pub max_queued_executions: usize,
    #[serde(default = "max_runs_per_hour")]
    pub max_runs_per_hour: usize,
}

impl Default for PerUserExecutionConfig {
    fn default() -> Self {
        Self {
            max_concurrent_runs: max_concurrent_runs(),
            max_queued_runs: max_queued_runs(),
            max_queued_executions: max_queued_executions(),
            max_runs_per_hour: max_runs_per_hour(),
        }
    }
}

fn max_concurrent_runs() -> usize {
    DEFAULT_MAX_CONCURRENT_RUNS
}

fn max_queued_runs() -> usize {
    DEFAULT_MAX_QUEUED_RUNS
}

fn max_queued_executions() -> usize {
    DEFAULT_MAX_QUEUED_EXECUTIONS
}

fn max_runs_per_hour() -> usize {
    DEFAULT_MAX_RUNS_PER_HOUR
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolboxConfig {
    pub pull_policy: String,
    pub image_repository: String,
    pub image_tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OtelConfig {
    pub collector_grpc: String,
    pub collector_http: String,
    pub prometheus_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GcsConfig {
    pub bucket: String,
    pub url_ttl_secs: u64,
    #[serde(default)]
    pub mock_internal_url: Option<String>,
    #[serde(default)]
    pub mock_public_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Config {
        pub fn for_test() -> Self {
            Self {
                admins_path: "dummy".to_string(),
                db: DbConfig {
                    host: "localhost".to_string(),
                    port: 5432,
                    name: "test".to_string(),
                    user: "test".to_string(),
                    pass: Some("test".to_string()),
                },
                server: ServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 8035,
                    body_limit_mb: 50,
                    orchestrator_url: "http://localhost:8035".to_string(),
                },
                workload_clusters: WorkloadClusters::for_test_with_available_clusters(
                    10,
                    "alpha",
                    &["alpha"],
                ),
                toolbox: ToolboxConfig {
                    pull_policy: "IfNotPresent".to_string(),
                    image_repository: "rtf-toolbox".to_string(),
                    image_tag: "edge".to_string(),
                },
                otel: OtelConfig {
                    collector_grpc: "http://otel:4317".to_string(),
                    collector_http: "http://otel:4318".to_string(),
                    prometheus_endpoint: "http://prometheus:9090".to_string(),
                },
                gcs: GcsConfig {
                    bucket: "test-bucket".to_string(),
                    url_ttl_secs: 300,
                    mock_internal_url: Some("http://mock-gcs-internal".to_string()),
                    mock_public_url: Some("http://mock-gcs-public".to_string()),
                },
                apollo_key: "dummy".to_string(),
                github: GithubConfig {
                    app_id: 1,
                    app_private_key_pem: "dummy".to_string(),
                },
            }
        }
    }

    impl WorkloadClusters {
        pub fn for_test() -> Self {
            Self::for_test_with_available_clusters(10, "alpha", &["alpha"])
        }

        pub fn for_test_with_available_clusters(
            max_concurrent: usize,
            default_cluster: &str,
            names: &[&str],
        ) -> Self {
            Self {
                default_cluster: default_cluster.to_string(),
                max_queued_executions: 100,
                available_clusters: names
                    .iter()
                    .map(|name| WorkloadClusterConfig {
                        name: name.to_string(),
                        kubeconfig_secret_name: "workload-kubeconfig".to_string(),
                        workload_context: "dummy".to_string(),
                        execution: ClusterExecutionConfig {
                            max_concurrent,
                            failed_execution_ttl_secs: 600,
                            retry_window_secs: 5 * 60,
                            poll_interval_secs: 10,
                            per_user: Default::default(),
                        },
                    })
                    .collect(),
            }
        }
    }

    #[test]
    fn local_stack_config_parses() {
        let raw = include_str!("../resources/local-config.yaml");

        Config::try_parse(raw).expect("local config used in tests should parse");
    }
}

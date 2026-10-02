use crate::{
    Error,
    db::{ClusterId, PoolId},
};
use rtf_config::context::Context;
use rtf_orchestrator_shared::workload_config::{self, WorkloadConfig};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env, fs, iter,
    net::SocketAddr,
    sync::LazyLock,
};
use thiserror::Error;

pub const DEFAULT_POOL_NAME: &str = "default";
pub const DEFAULT_POOL: PoolId = PoolId::new_static(DEFAULT_POOL_NAME);

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
    Err(errors) => {
        let errors: Vec<_> = errors.iter().map(|e| format!("  - {e}")).collect();
        panic!("invalid config file:\n{}", errors.join("\n"))
    }
});

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid YAML: {inner}")]
    Yaml { inner: String },

    #[error("duplicate cluster name: {name}")]
    DuplicateClusterName { name: String },

    #[error("duplicate pool name: {name}")]
    DuplicatePoolName { name: String },

    #[error("empty pool: {pool}")]
    EmptyPool { pool: String },

    #[error("pool {pool} contains unknown cluster: {cluster}")]
    UnknownCluster { pool: String, cluster: String },

    #[error("cluster {cluster} appears in multiple pools: {first}, {second}")]
    ClusterInMultiplePools {
        cluster: String,
        first: String,
        second: String,
    },

    #[error("pool {pool} has invalid workload config: {reason}")]
    InvalidWorkloadConfig {
        pool: String,
        reason: workload_config::Error,
    },

    #[error("pool {pool} sets node_label_weights but does not support dedicated clusters")]
    NodeLabelsWithoutDedicated { pool: String },
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    pub admins_path: String,
    pub automation_users: Vec<String>,
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
    pub fn try_parse_from_env() -> Result<Self, Vec<ConfigError>> {
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

    fn try_parse(text: &str) -> Result<Self, Vec<ConfigError>> {
        let cfg: Self = serde_yaml::from_str(text).map_err(|e| {
            vec![ConfigError::Yaml {
                inner: e.to_string(),
            }]
        })?;
        cfg.workload_clusters.validate()?;

        Ok(cfg)
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

    pub fn per_user_execution_config(
        &self,
        pool: &PoolId,
    ) -> Result<PerUserExecutionConfig, Error> {
        self.workload_clusters
            .cluster_pools
            .pool_config(pool)
            .map(|p| p.per_user_limits.clone())
            .ok_or_else(|| Error::UnknownWorkloadPool {
                pool: pool.to_string(),
            })
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
    pub max_queued_executions: usize,
    pub cluster_roles: ClusterRoles,
    pub available_clusters: Vec<WorkloadClusterConfig>,
    pub cluster_pools: ClusterPools,
}

impl WorkloadClusters {
    pub fn validate(&self) -> Result<(), Vec<ConfigError>> {
        let mut errors = Vec::new();
        let mut cluster_names = HashSet::new();
        let mut pool_names = HashSet::new();
        let mut pool_members: HashMap<&str, &str> = HashMap::new();

        // Ensure that cluster names are unique
        for c in self.available_clusters.iter() {
            if !cluster_names.insert(c.name.as_str()) {
                errors.push(ConfigError::DuplicateClusterName {
                    name: c.name.clone(),
                });
            }
        }

        for (pool_name, pool) in self.cluster_pools.iter() {
            // Ensure that pool names are unique
            if !pool_names.insert(pool_name) {
                errors.push(ConfigError::DuplicatePoolName {
                    name: pool_name.to_string(),
                });
            }
            // Ensure that pools have at least one cluster
            if pool.available_clusters.is_empty() {
                errors.push(ConfigError::EmptyPool {
                    pool: pool_name.to_string(),
                });
            }

            // Ensure that the workload config is valid and that nodes are only labelled in pools
            // that can hand out dedicated clusters
            if let Err(reason) = pool.workload_config.validate() {
                errors.push(ConfigError::InvalidWorkloadConfig {
                    pool: pool_name.to_string(),
                    reason,
                });
            }
            if !pool.workload_config.node_label_weights.is_empty() && !pool.supports_dedicated {
                errors.push(ConfigError::NodeLabelsWithoutDedicated {
                    pool: pool_name.to_string(),
                });
            }

            for cluster in pool.available_clusters.iter() {
                // Ensure that pools only contain known clusters
                if !cluster_names.contains(cluster.as_str()) {
                    errors.push(ConfigError::UnknownCluster {
                        pool: pool_name.to_string(),
                        cluster: cluster.to_string(),
                    });
                    continue;
                }

                // Ensure clusters are only in a single pool
                if let Some(other) = pool_members.insert(cluster, pool_name) {
                    errors.push(ConfigError::ClusterInMultiplePools {
                        cluster: cluster.to_string(),
                        first: other.to_string(),
                        second: pool_name.to_string(),
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn available_clusters(&self) -> Vec<ClusterId> {
        self.available_clusters
            .iter()
            .map(|c| ClusterId::new(&c.name))
            .collect()
    }

    pub fn available_pools(&self) -> Vec<PoolId> {
        self.cluster_pools
            .iter()
            .map(|(name, _)| PoolId::new(name))
            .collect()
    }

    pub fn dedicated_pools(&self) -> HashSet<PoolId> {
        self.cluster_pools
            .iter()
            .filter(|(_, pool)| pool.supports_dedicated)
            .map(|(name, _)| PoolId::new(name))
            .collect()
    }

    pub fn pool_workload_config(&self, pool: &PoolId) -> Option<&WorkloadConfig> {
        self.cluster_pools
            .pool_config(pool)
            .map(|pool| &pool.workload_config)
    }

    pub fn pool_clusters(&self) -> HashMap<PoolId, Vec<ClusterId>> {
        self.cluster_pools
            .iter()
            .map(|(name, pool)| {
                let clusters = pool.available_clusters.iter().map(ClusterId::new).collect();
                (PoolId::new(name), clusters)
            })
            .collect()
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

/// Names of the cluster roles that we bind to the scenario service account.
///
/// See `management-cluster/workload-rbac/` in kanaveral for the production details of these.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClusterRoles {
    pub cluster_read: String,
    pub namespace_read: String,
    pub namespace_write: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClusterPools {
    pub default: PoolConfig,
    #[serde(default)]
    pub additional: Vec<NamedPoolConfig>,
}

impl ClusterPools {
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PoolConfig)> {
        iter::once((DEFAULT_POOL_NAME, &self.default))
            .chain(self.additional.iter().map(|p| (p.name.as_str(), &p.config)))
    }

    pub fn pool_config(&self, pool: &PoolId) -> Option<&PoolConfig> {
        self.iter()
            .find(|(name, _)| *name == pool.as_str())
            .map(|(_, config)| config)
    }

    pub fn pool_config_for_cluster(&self, cluster: &str) -> Option<&PoolConfig> {
        self.iter()
            .map(|(_, pool)| pool)
            .find(|pool| pool.available_clusters.iter().any(|c| c == cluster))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NamedPoolConfig {
    pub name: String,
    #[serde(flatten)]
    pub config: PoolConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PoolConfig {
    #[serde(default)]
    pub supports_dedicated: bool,
    pub available_clusters: Vec<String>,
    #[serde(default)]
    pub per_user_limits: PerUserExecutionConfig,
    #[serde(flatten)]
    pub workload_config: WorkloadConfig,
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
    /// Give each execution a node to itself. Requires `max_concurrent` to be no greater than
    /// the cluster's schedulable node count - pods that cannot find a free node are not
    /// queued, they fail on the deploy-environment timeout.
    #[serde(default)]
    pub exclusive_nodes: bool,
    /// Node labels the scenario pod must match. Empty means no constraint, and the scenario
    /// pod schedules normally alongside the environment.
    #[serde(default)]
    pub scenario_node_selector: BTreeMap<String, String>,
    /// How long to wait, after cleaning up a finished execution's namespace, for its pods to
    /// be removed before freeing its concurrency slot.
    #[serde(default = "namespace_cleanup_timeout_secs")]
    pub namespace_cleanup_timeout_secs: u64,
}

fn namespace_cleanup_timeout_secs() -> u64 {
    90
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
    use simple_test_case::test_case;

    impl Config {
        pub fn for_test() -> Self {
            Self {
                admins_path: "dummy".to_string(),
                automation_users: Vec::new(),
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
                cluster_pools: ClusterPools {
                    default: PoolConfig {
                        supports_dedicated: false,
                        available_clusters: vec![default_cluster.to_string()],
                        per_user_limits: Default::default(),
                        workload_config: Default::default(),
                    },
                    additional: names
                        .iter()
                        .filter(|name| **name != default_cluster)
                        .map(|name| NamedPoolConfig {
                            name: name.to_string(),
                            config: PoolConfig {
                                supports_dedicated: false,
                                available_clusters: vec![name.to_string()],
                                per_user_limits: Default::default(),
                                workload_config: Default::default(),
                            },
                        })
                        .collect(),
                },
                max_queued_executions: 100,
                cluster_roles: ClusterRoles {
                    cluster_read: "scenario-cluster-read".into(),
                    namespace_read: "scenario-namespace-read".into(),
                    namespace_write: "scenario-namespace-write".into(),
                },
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
                            exclusive_nodes: false,
                            scenario_node_selector: BTreeMap::new(),
                            namespace_cleanup_timeout_secs: 90,
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

    fn pool(clusters: &[&str]) -> PoolConfig {
        PoolConfig {
            supports_dedicated: false,
            available_clusters: clusters.iter().map(|c| c.to_string()).collect(),
            per_user_limits: Default::default(),
            workload_config: Default::default(),
        }
    }

    fn named_pool(name: &str, clusters: &[&str]) -> NamedPoolConfig {
        NamedPoolConfig {
            name: name.to_string(),
            config: pool(clusters),
        }
    }

    fn with_pools(
        clusters: &[&str],
        default: PoolConfig,
        additional: Vec<NamedPoolConfig>,
    ) -> WorkloadClusters {
        let mut wc = WorkloadClusters::for_test_with_available_clusters(1, clusters[0], clusters);
        wc.cluster_pools = ClusterPools {
            default,
            additional,
        };

        wc
    }

    #[test]
    fn validate_accepts_valid_config() {
        let wc = with_pools(
            &["alpha", "beta", "gamma"],
            pool(&["alpha"]),
            vec![named_pool("perf", &["beta", "gamma"])],
        );

        assert!(wc.validate().is_ok());
    }

    #[test_case(
        with_pools(&["a", "a"], pool(&["a"]), vec![]),
        vec![ConfigError::DuplicateClusterName { name: "a".into() }];
        "duplicate cluster names"
    )]
    #[test_case(
        with_pools(&["a", "b"], pool(&["a"]), vec![named_pool("default", &["b"])]),
        vec![ConfigError::DuplicatePoolName { name: "default".into() }];
        "additional pool named default"
    )]
    #[test_case(
        with_pools(&["a", "b", "c"], pool(&["a"]), vec![named_pool("X", &["b"]), named_pool("X", &["c"])]),
        vec![ConfigError::DuplicatePoolName { name: "X".into() }];
        "duplicate pool names"
    )]
    #[test_case(
        with_pools(&["a"], pool(&["a"]), vec![named_pool("X", &[])]),
        vec![ConfigError::EmptyPool { pool: "X".into() }];
        "empty pool"
    )]
    #[test_case(
        with_pools(&["a"], pool(&["a", "b"]), vec![]),
        vec![ConfigError::UnknownCluster { pool: "default".into(), cluster: "b".into() }];
        "unknown cluster"
    )]
    #[test_case(
        with_pools(&["a"], pool(&["a"]), vec![named_pool("X", &["a"])]),
        vec![ConfigError::ClusterInMultiplePools { cluster: "a".into(), first: "default".into(), second: "X".into() }];
        "cluster in multiple pools"
    )]
    #[test_case(
        with_pools(&["a"], pool(&["a", "b"]), vec![named_pool("X", &[])]),
        vec![
            ConfigError::UnknownCluster { pool: "default".into(), cluster: "b".into() },
            ConfigError::EmptyPool { pool: "X".into() }
        ];
        "multiple errors are returned together"
    )]
    #[test]
    fn validate_rejects_invalid_config(wc: WorkloadClusters, expected: Vec<ConfigError>) {
        assert_eq!(wc.validate().unwrap_err(), expected);
    }

    fn dedicated_pool_with_labels(clusters: &[&str], weights: &[(&str, u32)]) -> PoolConfig {
        PoolConfig {
            supports_dedicated: true,
            workload_config: WorkloadConfig {
                node_label_weights: weights.iter().map(|(v, w)| (v.to_string(), *w)).collect(),
                ..Default::default()
            },
            ..pool(clusters)
        }
    }

    #[test_case(
        with_pools(&["a"], dedicated_pool_with_labels(&["a"], &[("x", 1), ("y", 2)]), vec![]),
        Ok(());
        "labels in a dedicated pool"
    )]
    #[test_case(
        with_pools(&["a"], PoolConfig { supports_dedicated: false, ..dedicated_pool_with_labels(&["a"], &[("x", 1)]) }, vec![]),
        Err(vec![ConfigError::NodeLabelsWithoutDedicated { pool: "default".into() }]);
        "labels in a pool without dedicated support"
    )]
    #[test_case(
        with_pools(&["a"], dedicated_pool_with_labels(&["a"], &[("x", 0)]), vec![]),
        Err(vec![ConfigError::InvalidWorkloadConfig {
            pool: "default".into(),
            reason: workload_config::Error::ZeroWeights,
        }]);
        "all zero weights"
    )]
    #[test]
    fn validate_node_label_weights(wc: WorkloadClusters, expected: Result<(), Vec<ConfigError>>) {
        assert_eq!(wc.validate(), expected);
    }
}

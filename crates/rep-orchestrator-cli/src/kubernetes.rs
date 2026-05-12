use crate::error::{CliError, CliResult};
use anyhow::Context;
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Namespace, ServiceAccount},
};
use kube::{
    Api, Config,
    api::{ObjectMeta, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig},
};
use std::{collections::BTreeMap, path::Path};

const MANAGER_NAME: &str = "rep-orchestrator-cli";
const SA_NAME: &str = "results-writer";
const SA_PREFIX: &str = "iam.gke.io/gcp-service-account";
const WIF_ANNOTATION: &str = "results-writer@runtime-testing-framework.iam.gserviceaccount.com";

pub struct DeploymentStatus {
    pub total: usize,
    pub not_ready: Vec<DeploymentInfo>,
}

pub struct DeploymentInfo {
    pub name: String,
    pub last_condition_status: String,
}

pub trait Client: Send + Sync + Clone {
    fn create_namespace(&self, name: &str) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn create_results_writer_service_account(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn check_deployment_status(
        &self,
        namespace: &str,
    ) -> impl Future<Output = anyhow::Result<DeploymentStatus>> + Send;
}

#[derive(Clone)]
pub struct HttpClient {
    client: kube::Client,
}

impl HttpClient {
    pub async fn from_kubeconfig(kubeconfig_path: Option<&Path>) -> CliResult<Self> {
        let client = match kubeconfig_path {
            Some(path) => {
                let kfg = Kubeconfig::read_from(path)
                    .with_context(|| format!("failed to read kubeconfig from {}", path.display()))
                    .map_err(CliError::unrunnable)?;

                let config = Config::from_custom_kubeconfig(kfg, &KubeConfigOptions::default())
                    .await
                    .context("failed to build kube config")
                    .map_err(CliError::unrunnable)?;

                kube::Client::try_from(config)
            }
            None => kube::Client::try_default().await,
        }
        .context("failed to create kube client from in-cluster config")
        .map_err(CliError::unrunnable)?;

        Ok(Self { client })
    }
}

impl Client for HttpClient {
    async fn create_namespace(&self, name: &str) -> anyhow::Result<()> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        api.patch(name, &PatchParams::apply(MANAGER_NAME), &Patch::Apply(&ns))
            .await?;

        Ok(())
    }

    async fn create_results_writer_service_account(&self, namespace: &str) -> anyhow::Result<()> {
        let sa = ServiceAccount {
            metadata: ObjectMeta {
                name: Some(SA_NAME.to_owned()),
                namespace: Some(namespace.to_owned()),
                annotations: Some(BTreeMap::from([(
                    SA_PREFIX.to_owned(),
                    WIF_ANNOTATION.to_owned(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };

        let api: Api<ServiceAccount> = Api::namespaced(self.client.clone(), namespace);
        api.patch(
            SA_NAME,
            &PatchParams::apply(MANAGER_NAME),
            &Patch::Apply(&sa),
        )
        .await?;

        Ok(())
    }

    async fn check_deployment_status(&self, namespace: &str) -> anyhow::Result<DeploymentStatus> {
        let api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        let deployments = api.list(&Default::default()).await?;

        let not_ready: Vec<DeploymentInfo> = deployments
            .items
            .iter()
            .filter(|d| {
                !d.status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .is_some_and(|conditions| {
                        conditions
                            .iter()
                            .any(|c| c.type_ == "Available" && c.status == "True")
                    })
            })
            .filter_map(|d| {
                d.metadata.name.as_ref().map(|name| DeploymentInfo {
                    name: name.clone(),
                    last_condition_status: d
                        .status
                        .as_ref()
                        .and_then(|s| s.conditions.as_ref())
                        .and_then(|c| c.last())
                        .map(|c| c.status.clone())
                        .unwrap_or_else(|| "unknown".to_owned()),
                })
            })
            .collect();

        Ok(DeploymentStatus {
            total: deployments.items.len(),
            not_ready,
        })
    }
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[derive(Debug, Clone, PartialEq)]
    pub enum KubeCall {
        ApplyNamespace { name: String },
        ApplyResultsWriterServiceAccount { namespace: String },
        CheckDeploymentsAvailable { namespace: String },
    }

    struct MockState {
        calls: Vec<KubeCall>,
        should_fail: bool,
        deployments_available: bool,
    }

    impl Default for MockState {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                should_fail: false,
                deployments_available: true,
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

        fn record(&self, call: KubeCall) -> anyhow::Result<()> {
            let mut state = self.state.write().unwrap();
            if state.should_fail {
                return Err(anyhow::anyhow!("mock kube failure"));
            }
            state.calls.push(call);

            Ok(())
        }

        pub fn read_calls<F>(&self, closure: F)
        where
            F: FnOnce(&[KubeCall]),
        {
            let state = self.state.read().unwrap();

            closure(&state.calls)
        }
    }

    impl Client for MockClient {
        async fn create_namespace(&self, name: &str) -> anyhow::Result<()> {
            self.record(KubeCall::ApplyNamespace {
                name: name.to_owned(),
            })
        }

        async fn create_results_writer_service_account(
            &self,
            namespace: &str,
        ) -> anyhow::Result<()> {
            self.record(KubeCall::ApplyResultsWriterServiceAccount {
                namespace: namespace.to_owned(),
            })
        }

        async fn check_deployment_status(
            &self,
            namespace: &str,
        ) -> anyhow::Result<DeploymentStatus> {
            self.record(KubeCall::CheckDeploymentsAvailable {
                namespace: namespace.to_owned(),
            })?;
            let state = self.state.read().unwrap();

            if state.deployments_available {
                Ok(DeploymentStatus {
                    total: 1,
                    not_ready: vec![],
                })
            } else {
                Ok(DeploymentStatus {
                    total: 1,
                    not_ready: vec![DeploymentInfo {
                        name: "mock-deployment".to_owned(),
                        last_condition_status: "False".to_owned(),
                    }],
                })
            }
        }
    }
}

use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Namespace, ServiceAccount},
};
use kube::{
    Api, Config,
    api::{ObjectMeta, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig, KubeconfigError},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const MANAGER_NAME: &str = "rep-orchestrator-cli";
const SA_NAME: &str = "results-writer";
const SA_PREFIX: &str = "iam.gke.io/gcp-service-account";
const WIF_ANNOTATION: &str = "results-writer@runtime-testing-framework.iam.gserviceaccount.com";

/// Errors produced when communicating with the Kubernetes API or reading kubeconfig.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read kubeconfig from {path}")]
    ReadKubeconfig {
        path: PathBuf,
        #[source]
        source: KubeconfigError,
    },

    #[error("failed to build kube config")]
    BuildConfig(#[source] KubeconfigError),

    #[error("failed to create kubernetes client")]
    CreateClient(#[source] kube::Error),

    #[error("failed to create namespace")]
    CreateNamespace(#[source] kube::Error),

    #[error("failed to create results-writer service account")]
    CreateServiceAccount(#[source] kube::Error),

    #[error("failed to list deployments in namespace")]
    ListDeployments(#[source] kube::Error),

    #[error("timed out after {timeout_secs}s waiting for deployments: {names}")]
    DeploymentTimeout { timeout_secs: u64, names: String },
}

pub struct DeploymentStatus {
    pub total: usize,
    pub not_ready: Vec<DeploymentInfo>,
}

pub struct DeploymentInfo {
    pub name: String,
    pub last_condition_status: String,
}

pub trait Client: Send + Sync + Clone {
    fn create_namespace(&self, name: &str) -> impl Future<Output = Result<(), Error>> + Send;

    fn create_results_writer_service_account(
        &self,
        namespace: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    fn check_deployment_status(
        &self,
        namespace: &str,
    ) -> impl Future<Output = Result<DeploymentStatus, Error>> + Send;
}

#[derive(Clone)]
pub struct HttpClient {
    client: kube::Client,
}

impl HttpClient {
    pub async fn from_kubeconfig(kubeconfig_path: Option<&Path>) -> crate::Result<Self> {
        let client = match kubeconfig_path {
            Some(path) => {
                let kfg = Kubeconfig::read_from(path).map_err(|source| Error::ReadKubeconfig {
                    path: path.to_owned(),
                    source,
                })?;

                let config = Config::from_custom_kubeconfig(kfg, &KubeConfigOptions::default())
                    .await
                    .map_err(Error::BuildConfig)?;

                kube::Client::try_from(config)
            }
            None => kube::Client::try_default().await,
        }
        .map_err(Error::CreateClient)?;

        Ok(Self { client })
    }
}

impl Client for HttpClient {
    async fn create_namespace(&self, name: &str) -> Result<(), Error> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        api.patch(name, &PatchParams::apply(MANAGER_NAME), &Patch::Apply(&ns))
            .await
            .map_err(Error::CreateNamespace)?;

        Ok(())
    }

    async fn create_results_writer_service_account(&self, namespace: &str) -> Result<(), Error> {
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
        .await
        .map_err(Error::CreateServiceAccount)?;

        Ok(())
    }

    async fn check_deployment_status(&self, namespace: &str) -> Result<DeploymentStatus, Error> {
        let api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        let deployments = api
            .list(&Default::default())
            .await
            .map_err(Error::ListDeployments)?;

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

        fn should_fail(&self) -> bool {
            self.state.read().unwrap().should_fail
        }

        fn record_call(&self, call: KubeCall) {
            self.state.write().unwrap().calls.push(call);
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
        async fn create_namespace(&self, name: &str) -> Result<(), Error> {
            if self.should_fail() {
                return Err(Error::CreateNamespace(kube::Error::Api(
                    kube::core::Status::failure("mock kube failure", "MockFailure").boxed(),
                )));
            }
            self.record_call(KubeCall::ApplyNamespace {
                name: name.to_owned(),
            });

            Ok(())
        }

        async fn create_results_writer_service_account(
            &self,
            namespace: &str,
        ) -> Result<(), Error> {
            if self.should_fail() {
                return Err(Error::CreateServiceAccount(kube::Error::Api(
                    kube::core::Status::failure("mock kube failure", "MockFailure").boxed(),
                )));
            }
            self.record_call(KubeCall::ApplyResultsWriterServiceAccount {
                namespace: namespace.to_owned(),
            });

            Ok(())
        }

        async fn check_deployment_status(
            &self,
            namespace: &str,
        ) -> Result<DeploymentStatus, Error> {
            if self.should_fail() {
                return Err(Error::ListDeployments(kube::Error::Api(
                    kube::core::Status::failure("mock kube failure", "MockFailure").boxed(),
                )));
            }
            self.record_call(KubeCall::CheckDeploymentsAvailable {
                namespace: namespace.to_owned(),
            });
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

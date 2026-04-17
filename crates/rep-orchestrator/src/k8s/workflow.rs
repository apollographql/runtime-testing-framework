use crate::{
    db::TestExecution,
    k8s::{TOOLBOX_IMAGE, env_configmap_name},
};
use k8s_openapi::api::core::v1::{
    ConfigMapVolumeSource, Container, EnvVar, KeyToPath, SecretVolumeSource, Volume, VolumeMount,
};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const TTL_SECONDS_AFTER_FINISHED: i32 = 3600; // cleanup after 1h

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct WorkflowStatus {
    pub phase: Option<String>, // Pending | Running | Succeeded | Failed | Error
    pub message: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TtlStrategy {
    pub seconds_after_completion: Option<i32>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodGC {
    pub strategy: String,
}

#[derive(CustomResource, Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "argoproj.io",
    version = "v1alpha1",
    kind = "Workflow",
    namespaced,
    status = "WorkflowStatus",
    derive = "Default"
)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSpec {
    pub service_account_name: String,
    pub entrypoint: String,
    pub on_exit: String,
    pub templates: Vec<TemplateDef>,
    pub volumes: Vec<Volume>,
    pub ttl_strategy: Option<TtlStrategy>,
    pub pod_g_c: Option<PodGC>,
}

impl WorkflowSpec {
    // TODO: This needs to be rewritten to use the toolbox image commands instead of inline shell
    // scripts!
    // The current helper methods on the nested structs are aimed at creating tasks that run shell
    // inline shell scripts. These will need to be updated to simply call the corresponding
    // subcommands from the CLI.
    pub fn for_execution(ex: &TestExecution, orchestrator_url: &str) -> Self {
        let execution_id = ex.uuid();
        let configmap_name = env_configmap_name(&execution_id);
        let env_vars = ex.toolbox_env_vars(orchestrator_url);
        let namespace = execution_id.to_string();

        Self {
            service_account_name: "argo-workflow".to_owned(),
            entrypoint: "main".to_owned(),
            on_exit: "cleanup".to_owned(),
            templates: vec![
                TemplateDef::Main(MainTemplate::new()),
                TemplateDef::Task(create_namespace(&namespace, env_vars.clone())),
                TemplateDef::Task(create_pull_secret(&namespace, env_vars.clone())),
                TemplateDef::Task(deploy_environment(&configmap_name, &namespace, env_vars)),
                TemplateDef::Task(cleanup(&configmap_name)),
            ],
            volumes: vec![Volume {
                name: "kubeconfig".into(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some("workload-kubeconfig".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ttl_strategy: Some(TtlStrategy {
                seconds_after_completion: Some(TTL_SECONDS_AFTER_FINISHED),
            }),
            // Delete the workflow pods if the workflow succeeds
            // Retains failed pods for debugging
            pod_g_c: Some(PodGC {
                strategy: "OnWorkflowSuccess".to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[allow(clippy::large_enum_variant)]
#[serde(untagged)]
pub enum TemplateDef {
    Main(MainTemplate),
    Task(TaskTemplate),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MainTemplate {
    pub name: String,
    pub dag: Dag,
}

impl MainTemplate {
    #[expect(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            name: "main".into(),
            dag: Dag {
                tasks: vec![
                    TaskSpec::new("create-namespace", &[]),
                    TaskSpec::new("create-pull-secret", &["create-namespace"]),
                    TaskSpec::new("deploy-environment", &["create-pull-secret"]),
                ],
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Dag {
    pub tasks: Vec<TaskSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TaskSpec {
    pub name: String,
    pub template: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

impl TaskSpec {
    fn new(name: &str, deps: &[&str]) -> Self {
        Self {
            name: name.into(),
            template: name.into(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TaskTemplate {
    pub name: String,
    pub container: Container,
    pub volumes: Option<Vec<Volume>>,
}

impl TaskTemplate {
    fn new(
        name: &str,
        arg: String,
        volume_mounts: Vec<VolumeMount>,
        volumes: Option<Vec<Volume>>,
        env: Vec<EnvVar>,
    ) -> Self {
        Self {
            name: name.into(),
            container: Container {
                name: name.into(),
                image: Some(TOOLBOX_IMAGE.into()),
                command: Some(vec!["/bin/sh".to_owned(), "-c".to_owned()]),
                args: Some(vec![arg]),
                volume_mounts: Some(volume_mounts),
                env: Some(env),
                ..Default::default()
            },
            volumes,
        }
    }
}

const CREATE_NAMESPACE_SCRIPT: &str = r#"
set -e
curl -X POST \
  "$APOLLO_REP_ORCHESTRATOR_URL/test-execution/$APOLLO_REP_ORCHESTRATOR_EXECUTION_ID/status" \
  -H "Bearer: $APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"status":"PROVISIONING","message":"creating namespace"}'

echo "Creating namespace '__NAMESPACE__' in workload cluster..."
kubectl \
  --kubeconfig=/kubeconfig/value \
  create namespace __NAMESPACE__ \
  --dry-run=client \
  -o yaml |
    kubectl --kubeconfig=/kubeconfig/value apply -f -

echo "Namespace created successfully."

curl -X POST \
  "$APOLLO_REP_ORCHESTRATOR_URL/test-execution/$APOLLO_REP_ORCHESTRATOR_EXECUTION_ID/status" \
  -H "Bearer: $APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"status":"PROVISIONING","message":"namespace created"}'
"#;

fn create_namespace(namespace: &str, env: Vec<EnvVar>) -> TaskTemplate {
    TaskTemplate::new(
        "create-namespace",
        CREATE_NAMESPACE_SCRIPT.replace("__NAMESPACE__", namespace),
        vec![VolumeMount {
            name: "kubeconfig".into(),
            mount_path: "/kubeconfig".into(),
            read_only: Some(true),
            ..Default::default()
        }],
        None,
        env,
    )
}

const CREATE_PULL_SECRET_SCRIPT: &str = r#"
set -e
curl -X POST \
  "$APOLLO_REP_ORCHESTRATOR_URL/test-execution/$APOLLO_REP_ORCHESTRATOR_EXECUTION_ID/status" \
  -H "Bearer: $APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"status":"PROVISIONING","message":"creating pull secret"}'

echo "Creating image pull secret in namespace '__NAMESPACE__'..."
kubectl --kubeconfig=/kubeconfig/value \
  create secret docker-registry gcr-secret \
  --namespace=__NAMESPACE__ \
  --from-file=.dockerconfigjson=/gcr-secret/config.json \
  --dry-run=client -o yaml |
    kubectl --kubeconfig=/kubeconfig/value apply -f -

echo "Patching default service account..."
kubectl --kubeconfig=/kubeconfig/value \
  patch serviceaccount default \
  --namespace=__NAMESPACE__ \
  -p '{"imagePullSecrets": [{"name": "gcr-secret"}]}'

echo "Pull secret created successfully."
"#;

fn create_pull_secret(namespace: &str, env: Vec<EnvVar>) -> TaskTemplate {
    TaskTemplate::new(
        "create-pull-secret",
        CREATE_PULL_SECRET_SCRIPT.replace("__NAMESPACE__", namespace),
        vec![
            VolumeMount {
                name: "kubeconfig".into(),
                mount_path: "/kubeconfig".into(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: "gcr-secret".into(),
                mount_path: "/gcr-secret".into(),
                read_only: Some(true),
                ..Default::default()
            },
        ],
        Some(vec![Volume {
            name: "gcr-secret".into(),
            secret: Some(SecretVolumeSource {
                secret_name: Some("gcr-secret".into()),
                items: Some(vec![KeyToPath {
                    key: ".dockerconfigjson".into(),
                    path: "config.json".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        env,
    )
}

const DEPLOY_ENV_SCRIPT: &str = r#"
set -e
curl -X POST \
  "$APOLLO_REP_ORCHESTRATOR_URL/test-execution/$APOLLO_REP_ORCHESTRATOR_EXECUTION_ID/status" \
  -H "Bearer: $APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"status":"PROVISIONING","message":"deploying environment"}'

WORKDIR=/tmp/rtf-work
mkdir -p $WORKDIR
mkdir -p $WORKDIR/k8s

echo "Resolving environment docker-compose files..."
rtf resolve environment /environment/environment.yaml --outdir $WORKDIR/output

# Source RTF env vars
. $WORKDIR/output/setup/setup.env
echo "Converting to kubernetes manifests..."
KOMPOSE_ARGS=""
while IFS= read -r f || [ -n "$f" ]; do
  [ -n "$f" ] && KOMPOSE_ARGS="$KOMPOSE_ARGS -f $f"
done < "$COMPOSE_FILES"
kompose convert $KOMPOSE_ARGS -o $WORKDIR/k8s/

echo "Applying manifests to namespace '__NAMESPACE__'..."
kubectl --kubeconfig=/kubeconfig/value apply \
  -n __NAMESPACE__ \
  -f $WORKDIR/k8s/

echo "Waiting for deployments to become available..."
kubectl --kubeconfig=/kubeconfig/value wait \
  --for=condition=available \
  deployment \
  --all \
  -n __NAMESPACE__ \
  --timeout=300s

echo "Environment deployed successfully."
curl -X POST \
  "$APOLLO_REP_ORCHESTRATOR_URL/test-execution/$APOLLO_REP_ORCHESTRATOR_EXECUTION_ID/status" \
  -H "Bearer: $APOLLO_REP_ORCHESTRATOR_EXECUTION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"status":"PROVISIONING","message":"environment deployed successfully"}'
"#;

fn deploy_environment(configmap_name: &str, namespace: &str, env: Vec<EnvVar>) -> TaskTemplate {
    TaskTemplate::new(
        "deploy-environment",
        DEPLOY_ENV_SCRIPT.replace("__NAMESPACE__", namespace),
        vec![
            VolumeMount {
                name: "kubeconfig".into(),
                mount_path: "/kubeconfig".into(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: "environment".into(),
                mount_path: "/environment".into(),
                read_only: Some(true),
                ..Default::default()
            },
        ],
        Some(vec![Volume {
            name: "environment".to_owned(),
            config_map: Some(ConfigMapVolumeSource {
                name: configmap_name.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        env,
    )
}

const CLEANUP_SCRIPT: &str = r#"
set -e
echo "Cleaning up ConfigMap '__CONFIGMAP__'..."

kubectl delete configmap __CONFIGMAP__ \
  -n cluster-api \
  --ignore-not-found

echo "Cleanup complete."
"#;

fn cleanup(configmap_name: &str) -> TaskTemplate {
    TaskTemplate::new(
        "cleanup",
        CLEANUP_SCRIPT.replace("__CONFIGMAP__", configmap_name),
        vec![],
        None,
        vec![],
    )
}

//! Helper trait for building Argo workflows from RTF environment variants
use crate::k8s::{
    TaskSpec, TaskTemplate, TemplateDef,
    workflow::{EnvironmentTaskSettings, KUBECONFIG_PATH, kubeconfig_volume_mount},
};
use k8s_openapi::api::core::v1::EnvVar;
use rtf_config::formats::{DockerComposeEnvironment, K8sEnvironment, NullEnvironment};
use rtf_orchestrator_shared::test_plan::OrchestratorEnvironment;

pub const DEPLOY_ENVIRONMENT: &str = "deploy-environment";

/// Helper trait for adding tasks to the Argo workflow DAG used for deploying per-execution
/// environments.
pub trait AsWorkflowTasks {
    fn specs(&self, parent: &str) -> Vec<TaskSpec>;

    fn templates(
        &self,
        namespace: &str,
        settings: &EnvironmentTaskSettings<'_>,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate>;

    fn tasks(
        &self,
        namespace: &str,
        settings: &EnvironmentTaskSettings<'_>,
        env: Vec<EnvVar>,
    ) -> impl Iterator<Item = TemplateDef> {
        self.templates(namespace, settings, env)
            .into_iter()
            .map(TemplateDef::Task)
    }
}

impl AsWorkflowTasks for OrchestratorEnvironment {
    fn specs(&self, parent: &str) -> Vec<TaskSpec> {
        match self {
            Self::DockerCompose(inner) => inner.specs(parent),
            Self::Null(inner) => inner.specs(parent),
            Self::K8s(inner) => inner.specs(parent),
        }
    }

    fn templates(
        &self,
        namespace: &str,
        settings: &EnvironmentTaskSettings<'_>,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        match self {
            Self::Null(inner) => inner.templates(namespace, settings, env),
            Self::DockerCompose(inner) => inner.templates(namespace, settings, env),
            Self::K8s(inner) => inner.templates(namespace, settings, env),
        }
    }
}

fn deploy_env_template(
    namespace: &str,
    settings: &EnvironmentTaskSettings<'_>,
    native_k8s: bool,
    env: Vec<EnvVar>,
) -> TaskTemplate {
    let toolbox = settings.toolbox;
    let mut args = vec![
        "deploy-environment".into(),
        "--namespace".into(),
        namespace.into(),
        "--kubeconfig".into(),
        KUBECONFIG_PATH.into(),
        "--toolbox-pull-policy".into(),
        toolbox.pull_policy.into(),
        "--toolbox-image".into(),
        toolbox.image.into(),
        "--provider-dir".into(),
        "/providers".into(),
        "--otel-collector-grpc".into(),
        toolbox.otel.grpc.clone(),
        "--otel-collector-http".into(),
        toolbox.otel.http.clone(),
    ];

    if native_k8s {
        args.push("--native-k8s".into());
    }

    if settings.exclusive_nodes {
        args.push("--exclusive-nodes".into());
    }

    TaskTemplate::new(
        DEPLOY_ENVIRONMENT,
        toolbox.pull_policy,
        toolbox.image,
        args,
        vec![kubeconfig_volume_mount()],
        None,
        env,
    )
}

impl AsWorkflowTasks for DockerComposeEnvironment {
    fn specs(&self, parent: &str) -> Vec<TaskSpec> {
        vec![TaskSpec::new(DEPLOY_ENVIRONMENT, &[parent])]
    }

    fn templates(
        &self,
        namespace: &str,
        settings: &EnvironmentTaskSettings<'_>,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        vec![deploy_env_template(namespace, settings, false, env)]
    }
}

impl AsWorkflowTasks for K8sEnvironment {
    fn specs(&self, parent: &str) -> Vec<TaskSpec> {
        vec![TaskSpec::new(DEPLOY_ENVIRONMENT, &[parent])]
    }

    fn templates(
        &self,
        namespace: &str,
        settings: &EnvironmentTaskSettings<'_>,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        vec![deploy_env_template(namespace, settings, true, env)]
    }
}

impl AsWorkflowTasks for NullEnvironment {
    fn specs(&self, _parent: &str) -> Vec<TaskSpec> {
        Vec::new()
    }

    fn templates(
        &self,
        _namespace: &str,
        _settings: &EnvironmentTaskSettings<'_>,
        _env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::workflow::{CREATE_NAMESPACE, WorkflowToolboxSettings};
    use rtf_config::formats::{ComposeResources, K8sResources};
    use rtf_orchestrator_shared::OtelConfig;

    fn otel() -> OtelConfig {
        OtelConfig {
            grpc: "http://otel:4317".to_string(),
            http: "http://otel:4318".to_string(),
        }
    }

    fn toolbox(otel: &OtelConfig) -> WorkflowToolboxSettings<'_> {
        WorkflowToolboxSettings {
            orchestrator_url: "http://localhost:8035",
            pull_policy: "IfNotPresent",
            image: "rtf-toolbox:edge",
            otel,
        }
    }

    fn docker_compose_env() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: vec![],
            },
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        }
    }

    #[test]
    fn docker_compose_specs_depend_on_correct_parent() {
        let specs = docker_compose_env().specs(CREATE_NAMESPACE);

        assert_eq!(specs.len(), 1, "expected exactly one task spec");
        assert_eq!(specs[0].name, DEPLOY_ENVIRONMENT);
        assert_eq!(specs[0].dependencies, vec![CREATE_NAMESPACE.to_string()]);
    }

    #[test]
    fn docker_compose_templates_build_deploy_environment_container() {
        let otel = otel();
        let settings = EnvironmentTaskSettings {
            toolbox: &toolbox(&otel),
            exclusive_nodes: false,
        };
        let templates = docker_compose_env().templates("ns", &settings, vec![]);

        assert_eq!(templates.len(), 1, "expected exactly one task template");
        assert_eq!(templates[0].name, DEPLOY_ENVIRONMENT);
        assert_eq!(
            templates[0].container.image.as_deref(),
            Some("rtf-toolbox:edge")
        );

        let args = templates[0]
            .container
            .args
            .as_ref()
            .expect("deploy-environment container should have args");

        assert!(args.contains(&"deploy-environment".to_string()));
        assert!(args.contains(&"ns".to_string()));
        assert!(args.contains(&"IfNotPresent".to_string()));
        assert!(args.contains(&"rtf-toolbox:edge".to_string()));
        assert!(args.contains(&"http://otel:4317".to_string()));
        assert!(args.contains(&"http://otel:4318".to_string()));
        assert!(!args.contains(&"--native-k8s".to_string()));
        assert!(!args.contains(&"--exclusive-nodes".to_string()));
    }

    #[test]
    fn docker_compose_templates_push_exclusive_nodes_flag_when_enabled() {
        let otel = otel();
        let settings = EnvironmentTaskSettings {
            toolbox: &toolbox(&otel),
            exclusive_nodes: true,
        };
        let templates = docker_compose_env().templates("ns", &settings, vec![]);

        let args = templates[0]
            .container
            .args
            .as_ref()
            .expect("deploy-environment container should have args");

        assert!(args.contains(&"--exclusive-nodes".to_string()));
    }

    fn k8s_env() -> K8sEnvironment {
        K8sEnvironment {
            resources: K8sResources { resources: vec![] },
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        }
    }

    #[test]
    fn k8s_specs_depend_on_correct_parent() {
        let specs = k8s_env().specs(CREATE_NAMESPACE);

        assert_eq!(specs.len(), 1, "expected exactly one task spec");
        assert_eq!(specs[0].name, DEPLOY_ENVIRONMENT);
        assert_eq!(specs[0].dependencies, vec![CREATE_NAMESPACE.to_string()]);
    }

    #[test]
    fn k8s_templates_build_deploy_environment_container() {
        let otel = otel();
        let settings = EnvironmentTaskSettings {
            toolbox: &toolbox(&otel),
            exclusive_nodes: false,
        };
        let templates = k8s_env().templates("ns", &settings, vec![]);

        assert_eq!(templates.len(), 1, "expected exactly one task template");
        assert_eq!(templates[0].name, DEPLOY_ENVIRONMENT);
        assert_eq!(
            templates[0].container.image.as_deref(),
            Some("rtf-toolbox:edge")
        );

        let args = templates[0]
            .container
            .args
            .as_ref()
            .expect("deploy-environment container should have args");

        assert!(args.contains(&"deploy-environment".to_string()));
        assert!(args.contains(&"ns".to_string()));
        assert!(args.contains(&"IfNotPresent".to_string()));
        assert!(args.contains(&"rtf-toolbox:edge".to_string()));
        assert!(args.contains(&"http://otel:4317".to_string()));
        assert!(args.contains(&"http://otel:4318".to_string()));
        assert!(args.contains(&"--native-k8s".to_string()));
        assert!(!args.contains(&"--exclusive-nodes".to_string()));
    }

    #[test]
    fn k8s_templates_push_exclusive_nodes_flag_when_enabled() {
        let otel = otel();
        let settings = EnvironmentTaskSettings {
            toolbox: &toolbox(&otel),
            exclusive_nodes: true,
        };
        let templates = k8s_env().templates("ns", &settings, vec![]);

        let args = templates[0]
            .container
            .args
            .as_ref()
            .expect("deploy-environment container should have args");

        assert!(args.contains(&"--exclusive-nodes".to_string()));
    }

    #[test]
    fn null_environment_contributes_no_specs_or_templates() {
        let env = NullEnvironment { skip: true };
        let otel = otel();
        let settings = EnvironmentTaskSettings {
            toolbox: &toolbox(&otel),
            exclusive_nodes: false,
        };

        assert!(env.specs(CREATE_NAMESPACE).is_empty());
        assert!(env.templates("ns", &settings, vec![]).is_empty());
    }
}

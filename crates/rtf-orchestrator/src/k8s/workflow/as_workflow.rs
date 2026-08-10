//! Helper trait for building Argo workflows from RTF environment variants
use crate::k8s::{
    TaskSpec, TaskTemplate, TemplateDef,
    workflow::{KUBECONFIG_PATH, kubeconfig_volume_mount},
};
use k8s_openapi::api::core::v1::EnvVar;
use rtf_config::formats::{DockerComposeEnvironment, NullEnvironment};
use rtf_orchestrator_shared::{OtelConfig, test_plan::OrchestratorEnvironment};

pub const DEPLOY_ENVIRONMENT: &str = "deploy-environment";

/// Helper trait for adding tasks to the Argo workflow DAG used for deploying per-execution
/// environments.
pub trait AsWorkflowTasks {
    fn specs(&self, parent: &str) -> Vec<TaskSpec>;

    fn templates(
        &self,
        namespace: &str,
        toolbox_pull_policy: &str,
        otel: &OtelConfig,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate>;

    fn tasks(
        &self,
        namespace: &str,
        toolbox_pull_policy: &str,
        otel: &OtelConfig,
        env: Vec<EnvVar>,
    ) -> impl Iterator<Item = TemplateDef> {
        self.templates(namespace, toolbox_pull_policy, otel, env)
            .into_iter()
            .map(TemplateDef::Task)
    }
}

impl AsWorkflowTasks for OrchestratorEnvironment {
    fn specs(&self, parent: &str) -> Vec<TaskSpec> {
        match self {
            Self::DockerCompose(inner) => inner.specs(parent),
            Self::Null(inner) => inner.specs(parent),
        }
    }

    fn templates(
        &self,
        namespace: &str,
        toolbox_pull_policy: &str,
        otel: &OtelConfig,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        match self {
            Self::DockerCompose(inner) => {
                inner.templates(namespace, toolbox_pull_policy, otel, env)
            }
            Self::Null(inner) => inner.templates(namespace, toolbox_pull_policy, otel, env),
        }
    }
}

impl AsWorkflowTasks for DockerComposeEnvironment {
    fn specs(&self, parent: &str) -> Vec<TaskSpec> {
        vec![TaskSpec::new(DEPLOY_ENVIRONMENT, &[parent])]
    }

    fn templates(
        &self,
        namespace: &str,
        toolbox_pull_policy: &str,
        otel: &OtelConfig,
        env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        vec![TaskTemplate::new(
            DEPLOY_ENVIRONMENT,
            toolbox_pull_policy,
            vec![
                "deploy-environment".into(),
                "--namespace".into(),
                namespace.into(),
                "--kubeconfig".into(),
                KUBECONFIG_PATH.into(),
                "--toolbox-pull-policy".into(),
                toolbox_pull_policy.into(),
                "--provider-dir".into(),
                "/providers".into(),
                "--otel-collector-grpc".into(),
                otel.grpc.clone(),
                "--otel-collector-http".into(),
                otel.http.clone(),
            ],
            vec![kubeconfig_volume_mount()],
            None,
            env,
        )]
    }
}

impl AsWorkflowTasks for NullEnvironment {
    fn specs(&self, _parent: &str) -> Vec<TaskSpec> {
        Vec::new()
    }

    fn templates(
        &self,
        _namespace: &str,
        _toolbox_pull_policy: &str,
        _otel: &OtelConfig,
        _env: Vec<EnvVar>,
    ) -> Vec<TaskTemplate> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::workflow::CREATE_SERVICE_ACCOUNT;

    fn otel() -> OtelConfig {
        OtelConfig {
            grpc: "http://otel:4317".to_string(),
            http: "http://otel:4318".to_string(),
        }
    }

    fn docker_compose_env() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        }
    }

    #[test]
    fn docker_compose_specs_depend_on_parent() {
        let specs = docker_compose_env().specs(CREATE_SERVICE_ACCOUNT);

        assert_eq!(specs.len(), 1, "expected exactly one task spec");
        assert_eq!(specs[0].name, DEPLOY_ENVIRONMENT);
        assert_eq!(
            specs[0].dependencies,
            vec!["create-service-account".to_string()]
        );
    }

    #[test]
    fn docker_compose_templates_build_deploy_environment_container() {
        let templates = docker_compose_env().templates("ns", "IfNotPresent", &otel(), vec![]);

        assert_eq!(templates.len(), 1, "expected exactly one task template");
        assert_eq!(templates[0].name, DEPLOY_ENVIRONMENT);

        let args = templates[0]
            .container
            .args
            .as_ref()
            .expect("deploy-environment container should have args");

        assert!(args.contains(&"deploy-environment".to_string()));
        assert!(args.contains(&"ns".to_string()));
        assert!(args.contains(&"IfNotPresent".to_string()));
        assert!(args.contains(&"http://otel:4317".to_string()));
        assert!(args.contains(&"http://otel:4318".to_string()));
    }

    #[test]
    fn null_environment_contributes_no_specs_or_templates() {
        let env = NullEnvironment { skip: true };

        assert!(env.specs(CREATE_SERVICE_ACCOUNT).is_empty());
        assert!(
            env.templates("ns", "IfNotPresent", &otel(), vec![])
                .is_empty()
        );
    }
}

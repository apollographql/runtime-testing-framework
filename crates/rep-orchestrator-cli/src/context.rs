use crate::orchestrator;
use anyhow::Context;
use rep_orchestrator_shared::{EXECUTION_ID_ENV_VAR, ORCHESTRATOR_URL_ENV_VAR};
use std::{env, str::FromStr};
use uuid::Uuid;

pub trait CliContext {
    type OrchestratorClient: orchestrator::Client;

    #[expect(unused)]
    fn orchestrator_client(&self) -> &Self::OrchestratorClient;
}

pub struct EnvironmentContext {
    orchestrator_client: orchestrator::HttpClient,
}

impl EnvironmentContext {
    pub fn from_environment() -> anyhow::Result<Self> {
        let orchestrator_url = env::var(ORCHESTRATOR_URL_ENV_VAR)
            .context(format!("{ORCHESTRATOR_URL_ENV_VAR} must be set"))?;

        let execution_id = env::var(EXECUTION_ID_ENV_VAR)
            .context(format!("{EXECUTION_ID_ENV_VAR} must be set"))
            .and_then(|id_var| {
                Uuid::from_str(&id_var)
                    .context(format!("{EXECUTION_ID_ENV_VAR} must be a valid UUID"))
            })?;

        let orchestrator_client = orchestrator::HttpClient::new(orchestrator_url, execution_id);

        Ok(Self {
            orchestrator_client,
        })
    }
}

impl CliContext for EnvironmentContext {
    type OrchestratorClient = orchestrator::HttpClient;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient {
        &self.orchestrator_client
    }
}

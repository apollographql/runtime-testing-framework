use crate::{
    context::OrchestratorContext,
    db::TestExecution,
    resolver::{self, ResolverError},
};
use rtf_orchestrator_shared::{
    OutputCollectionResponse,
    test_plan::{OrchestratorEnvironment, OrchestratorTestPlan},
    workload_config::WorkloadConfig,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct Shared {
    /// Metadata for every run with at least one in-flight execution, keyed by TestRun uuid.
    pub(super) runs: HashMap<Uuid, RunState>,
    /// Metadata for every in-flight TestExecution, keyed by its uuid.
    pub(super) executions: HashMap<Uuid, ExecutionState>,
    /// Maximum number of pending executions waiting for a namespace
    pub(super) max_queued_executions: usize,
    /// The number of currently queued executions
    pub(super) n_queued: usize,
}

impl Shared {
    pub(super) fn run_and_workload_config(
        &self,
        ex_id: Uuid,
    ) -> resolver::Result<(Uuid, WorkloadConfig)> {
        let run_uuid = self
            .executions
            .get(&ex_id)
            .ok_or(ResolverError::UnknownExecution(ex_id))?
            .run_uuid;

        let run = self
            .runs
            .get(&run_uuid)
            .ok_or(ResolverError::UnknownRun(run_uuid))?;

        Ok((run_uuid, run.workload_config.clone()))
    }

    pub(super) fn register_execution(&mut self, ex_uuid: Uuid, run_uuid: Uuid) {
        self.executions.insert(
            ex_uuid,
            ExecutionState {
                run_uuid,
                resolved_config: None,
            },
        );

        if let Some(run) = self.runs.get_mut(&run_uuid) {
            run.executions.insert(ex_uuid);
        }
    }

    pub(super) fn variant_with_context(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<(OrchestratorTestPlan, Arc<OrchestratorContext>)> {
        let run_uuid = match self.executions.get(&ex.uuid()) {
            Some(exec) => exec.run_uuid,
            None => return Err(ResolverError::UnknownExecution(ex.uuid())),
        };
        let index = ex.test_plan_index();

        match self.runs.get(&run_uuid) {
            Some(run) => match run.test_plan.try_expand_variant(index)? {
                Some((_, variant)) => Ok((variant, Arc::clone(&run.ctx))),
                None => Err(ResolverError::UnknownExecution(ex.uuid())),
            },
            None => Err(ResolverError::UnknownRun(run_uuid)),
        }
    }

    pub(super) fn snapshot_state(&self) -> SharedSnapshotState {
        SharedSnapshotState {
            cached_run_payloads: self.runs.keys().cloned().collect(),
            active_run_executions: self
                .runs
                .iter()
                .map(|(run_uuid, run)| (*run_uuid, run.executions.clone().into_iter().collect()))
                .collect(),
            resolved_execution_cache: self
                .executions
                .iter()
                .filter(|(_, exec)| exec.resolved_config.is_some())
                .map(|(ex_uuid, _)| *ex_uuid)
                .collect(),
        }
    }
}

#[derive(Debug)]
pub(super) struct RunState {
    pub(super) ctx: Arc<OrchestratorContext>,
    pub(super) test_plan: OrchestratorTestPlan,
    pub(super) executions: HashSet<Uuid>,
    pub(super) initiated_by: Option<String>,
    pub(super) workload_config: WorkloadConfig,
    pub(super) requires_dedicated: bool,
}

#[derive(Debug, PartialEq)]
pub(super) struct ExecutionState {
    pub(super) run_uuid: Uuid,
    pub(super) resolved_config: Option<ResolvedExecutionConfig>,
}

/// All config resolved for a single execution ahead of time, kept together since it's always
/// inserted and evicted as a unit.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ResolvedExecutionConfig {
    pub(super) env_yaml: String,
    pub(super) scenario_yaml: String,
    pub(super) output_collection: OutputCollectionResponse,
    pub(super) docker_image: String,
    pub(super) docker_command: String,
    pub(super) environment: OrchestratorEnvironment,
}

pub(super) struct SharedSnapshotState {
    pub(super) cached_run_payloads: Vec<Uuid>,
    pub(super) active_run_executions: BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub(super) resolved_execution_cache: Vec<Uuid>,
}

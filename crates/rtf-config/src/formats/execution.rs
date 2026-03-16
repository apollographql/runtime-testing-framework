use serde::{Deserialize, Serialize};

use crate::{
    formats::{DockerComposeEnvironment, DockerScenario, EnvironmentExecution, ScenarioExecution},
    run::{RunEnvironment, RunScenario},
};

/// Associates a concrete (Scenario, Environment) execution type pair.
pub trait Execution {
    type Scenario: RunScenario;
    type Environment: RunEnvironment;
}

/// Marker for the generic (multi-execution-type) test plan variant.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Generic;

impl Execution for Generic {
    type Scenario = ScenarioExecution;
    type Environment = EnvironmentExecution;
}

/// Marker for the REP (Runtime Execution Platform) test plan variant.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Rep;

impl Execution for Rep {
    type Scenario = DockerScenario;
    type Environment = DockerComposeEnvironment;
}

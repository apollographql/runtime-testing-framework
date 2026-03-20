use crate::{
    formats::{EnvironmentExecution, ScenarioExecution},
    run::{RunEnvironment, RunScenario},
};
use serde::{Serialize, de::DeserializeOwned};

/// Associates a concrete (Scenario, Environment) execution type pair.
pub trait Execution: Clone {
    type Scenario: RunScenario + DeserializeOwned + Serialize;
    type Environment: RunEnvironment + DeserializeOwned + Serialize;
}

/// Marker for the generic (multi-execution-type) test plan variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Generic;

impl Execution for Generic {
    type Scenario = ScenarioExecution;
    type Environment = EnvironmentExecution;
}

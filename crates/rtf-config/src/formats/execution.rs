use crate::{
    formats::{EnvironmentExecution, ScenarioExecution},
    run::{RunEnvironment, RunScenario, ValidateEnvironment, ValidateScenario},
};
use serde::{Serialize, de::DeserializeOwned};

pub trait Prepare: Clone {
    type Scenario: ValidateScenario + DeserializeOwned + Serialize;
    type Environment: ValidateEnvironment + DeserializeOwned + Serialize;
}

pub trait Run: Prepare
where
    Self::Scenario: RunScenario,
    Self::Environment: RunEnvironment,
{
}

/// Marker for the generic (multi-execution-type) test plan variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Generic;

impl Prepare for Generic {
    type Scenario = ScenarioExecution;
    type Environment = EnvironmentExecution;
}

impl Run for Generic {}

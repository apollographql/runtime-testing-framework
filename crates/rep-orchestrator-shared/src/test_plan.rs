//! A REP-orchestrator specific test plan implementation that only allows docker based scenarios
//! and environments.
use rtf_config::{
    Execution,
    formats::{DockerComposeEnvironment, DockerScenario, TestPlan},
};

/// Test plan restricted to Docker scenario + DockerCompose environment.
pub type RepTestPlan = TestPlan<Rep>;

/// Marker for the REP (Runtime Environment Provisioner) test plan variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Rep;

impl Execution for Rep {
    type Scenario = DockerScenario;
    type Environment = DockerComposeEnvironment;
}

use clap::{ArgAction, Parser, Subcommand};
use std::path::{Path, PathBuf};

/// CLI for the REP (Runtime Environment Provisioner) orchestrator.
///
/// Provides first-class commands for provisioning and tearing down
/// RTF environments on Kubernetes.
#[derive(Debug, Parser)]
#[clap(about, name = "rep-orchestrator-cli", long_about = None)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Command,

    /// Flag to control logging verbosity. Default level is `info`.
    /// `-v` sets logging level to `debug`, `-vv` to `trace`.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a namespace in the workload cluster
    CreateNamespace {
        /// Target namespace name
        #[arg(long)]
        namespace: String,

        /// Path to the kubeconfig file for the workload cluster
        #[arg(long)]
        kubeconfig: PathBuf,
    },

    /// Create the results-writer service account for Workload Identity Federation
    CreateServiceAccount {
        /// Target namespace name
        #[arg(long)]
        namespace: String,

        /// Path to the kubeconfig file for the workload cluster
        #[arg(long)]
        kubeconfig: PathBuf,
    },

    /// Resolve an RTF environment, convert the resolved compose files to Kubernetes manifests, and deploy
    DeployEnvironment {
        /// Target namespace for deployment
        #[arg(long)]
        namespace: String,

        /// Path to the kubeconfig file for the workload cluster
        #[arg(long)]
        kubeconfig: PathBuf,

        /// Timeout in seconds for waiting on deployments to become available
        #[arg(long, default_value = "300")]
        timeout: u64,

        /// The directory to write file provider output to
        #[arg(long, default_value = "/providers")]
        provider_dir: PathBuf,

        /// The image pull policy to set for toolbox init containers
        #[arg(long, default_value = "Always")]
        toolbox_pull_policy: String,
    },

    /// Resolve an RTF environment
    ResolveEnvironment {
        /// Path to write the resolved output to
        #[arg(long)]
        outdir: String,
    },

    /// Resolve the scenario config and write a `run.sh` wrapper into the shared volume for the
    /// scenario-runner container to execute.
    PrepareScenario {
        /// Path to the shared volume mounted by the scenario-runner and output-collector containers
        #[arg(long)]
        shared_dir: PathBuf,

        /// User-provided shell command to run as the scenario body
        #[arg(long)]
        command: String,
    },

    /// Upload the scenario's log file and zipped output directory, then report the terminal
    /// execution status based on the scenario's exit code.
    CollectOutput {
        /// Path to the shared volume containing the scenario's artifacts
        #[arg(long)]
        shared_dir: PathBuf,
    },
}

impl Command {
    pub fn kubeconfig(&self) -> Option<&Path> {
        match self {
            Self::CreateNamespace { kubeconfig, .. }
            | Self::CreateServiceAccount { kubeconfig, .. }
            | Self::DeployEnvironment { kubeconfig, .. } => Some(kubeconfig),
            Self::ResolveEnvironment { .. }
            | Self::PrepareScenario { .. }
            | Self::CollectOutput { .. } => None,
        }
    }
}

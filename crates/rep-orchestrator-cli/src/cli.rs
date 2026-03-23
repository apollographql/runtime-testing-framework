use clap::{ArgAction, Parser, Subcommand};
use std::path::PathBuf;

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

    /// Create an image pull secret and patch the default service account
    CreatePullSecret {
        /// Target namespace name
        #[arg(long)]
        namespace: String,

        /// Path to the kubeconfig file for the workload cluster
        #[arg(long)]
        kubeconfig: PathBuf,

        /// Path to the Docker config JSON file for registry authentication
        #[arg(long)]
        docker_config: PathBuf,
    },

    /// Resolve an RTF environment, convert the resolved compose files to Kubernetes manifests, and deploy
    DeployEnvironment {
        /// Target namespace for deployment
        #[arg(long)]
        namespace: String,

        /// Path to the kubeconfig file for the workload cluster
        #[arg(long)]
        kubeconfig: PathBuf,

        /// Path to the environment.yaml configuration file
        #[arg(long)]
        environment: PathBuf,

        /// Timeout in seconds for waiting on deployments to become available
        #[arg(long, default_value = "300")]
        timeout: u64,
    },

    /// Delete a ConfigMap used for environment configuration
    Cleanup {
        /// Name of the ConfigMap to delete
        #[arg(long)]
        configmap: String,

        /// Namespace where the ConfigMap resides
        #[arg(long)]
        namespace: String,

        /// Path to the kubeconfig file for the workload cluster
        #[arg(long)]
        kubeconfig: PathBuf,
    },
}

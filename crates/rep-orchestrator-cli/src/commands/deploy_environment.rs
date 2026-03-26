use crate::commands::{client_from_kubeconfig, run_shell};
use anyhow::{Context, anyhow, bail};
use k8s_openapi::api::apps::v1::Deployment;
use kube::{Api, Client};
use std::{collections::HashMap, env::temp_dir, fs, path::Path, process::Command, time::Duration};
use tokio::time::{Instant, sleep};
use tracing::info;

const DUPLICATE_ERROR: &str =
    "Encountered a duplicate environment entry in the resolved RTF environment";
const MALFORMED_ERROR: &str = "Encountered a malformed line in the resolved RTF environment";
const MISSING_EXPORT_ERROR: &str = "Expected leading 'export ' prefix to env file line";

pub async fn deploy_environment(
    namespace: &str,
    kubeconfig_path: &Path,
    environment_path: &Path,
    timeout: u64,
) -> anyhow::Result<()> {
    let workdir_path = temp_dir().join("rtf-work");
    let k8s_dir_path = workdir_path.join("k8s");
    fs::create_dir_all(&k8s_dir_path).context("Failed to create k8s working directory")?;

    info!("Resolving environment docker-compose files...");
    let outdir = workdir_path.join("output");
    run_shell(
        Command::new("rtf")
            .args(["resolve", "environment"])
            .arg(environment_path)
            .args(["--outdir", &outdir.to_string_lossy()]),
    )?;

    setup_env(&k8s_dir_path, &outdir)?;

    info!("Applying manifests to namespace '{namespace}'...");
    run_shell(
        Command::new("kubectl")
            .args(["--kubeconfig", &kubeconfig_path.to_string_lossy()])
            .args([
                "apply",
                "-n",
                namespace,
                "-f",
                &k8s_dir_path.to_string_lossy(),
            ]),
    )?;

    let client = client_from_kubeconfig(Some(kubeconfig_path)).await?;
    wait_for_deployments(&client, namespace, timeout).await?;
    info!("Environment deployed successfully.");

    Ok(())
}

/// Sources COMPOSE_FILES from the resolved RTF environment and converts them to k8s manifests
fn setup_env(k8s_dir_path: &Path, outdir_path: &Path) -> anyhow::Result<()> {
    let setup_env = outdir_path.join("setup/setup.env");
    let env_contents = fs::read_to_string(&setup_env).context("Failed to read setup.env file")?;

    let env_vars = parse_env_file(&env_contents)?;

    let compose_files_path = env_vars.get("COMPOSE_FILES").context(
        "COMPOSE_FILES was not set by setup.env -- make sure environment.yaml is well-formed",
    )?;

    let compose_files_content = fs::read_to_string(compose_files_path)
        .context("Failed to read COMPOSE_FILES file provider output")?;

    info!("Converting to kubernetes manifests...");
    let mut kompose = build_kompose_command(&compose_files_content, k8s_dir_path, &env_vars);

    run_shell(&mut kompose)
}

/// Parse a `.env` file into a map of key-value pairs.
///
/// Expects all lines in the file to be of the form "export $key=$value"
///
/// Assumes clean output from RTF and returns an [anyhow::Error] if the file was in any way malformed.
fn parse_env_file(contents: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut rtf_env = HashMap::new();

    for line in contents.lines() {
        let kv = line
            .strip_prefix("export ")
            .ok_or(anyhow!("{MISSING_EXPORT_ERROR}: {line:?}"))?;

        match kv.split_once('=') {
            Some((key, value)) => {
                if let Some(_existing) = rtf_env.insert(key.to_owned(), value.to_owned()) {
                    return Err(anyhow!("{DUPLICATE_ERROR}: {line:?}"));
                }
            }
            _ => {
                return Err(anyhow!("{MALFORMED_ERROR}: {line:?}"));
            }
        }
    }

    Ok(rtf_env)
}

/// Build the `kompose convert` command from the compose file listing and environment variables.
fn build_kompose_command(
    compose_files_content: &str,
    k8s_dir_path: &Path,
    env_vars: &HashMap<String, String>,
) -> Command {
    let mut kompose = Command::new("kompose");
    kompose.arg("convert");
    for line in compose_files_content.lines() {
        kompose.args(["-f", line]);
    }
    kompose.args(["-o", &k8s_dir_path.to_string_lossy()]);
    kompose.envs(env_vars);

    kompose
}

/// Wait for all Deployments in a namespace to have the Available condition.
async fn wait_for_deployments(
    client: &Client,
    namespace: &str,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    info!("Waiting for deployments to become available...");
    loop {
        let deployments = api
            .list(&Default::default())
            .await
            .context("Failed to list deployments")?;

        let all_available = deployments.items.iter().all(|deployment| {
            deployment
                .status
                .as_ref()
                .and_then(|status| status.conditions.as_ref())
                .is_some_and(|conditions| {
                    conditions.iter().any(|condition| {
                        condition.type_ == "Available" && condition.status == "True"
                    })
                })
        });

        if all_available && !deployments.items.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let not_ready: Vec<_> = deployments
                .items
                .iter()
                .filter(|deployment| {
                    !deployment
                        .status
                        .as_ref()
                        .and_then(|status| status.conditions.as_ref())
                        .is_some_and(|conditions| {
                            conditions.iter().any(|condition| {
                                condition.type_ == "Available" && condition.status == "True"
                            })
                        })
                })
                .filter_map(|deployment| {
                    deployment.metadata.name.as_ref().map(|name| {
                        format!(
                            "{}: {}",
                            name,
                            deployment
                                .status
                                .as_ref()
                                .and_then(|status| status.conditions.as_ref())
                                .and_then(|conditions| conditions.last())
                                .map(|condition| condition.status.as_str())
                                .unwrap_or("unknown")
                        )
                    })
                })
                .collect();
            bail!(
                "Timed out after {timeout_secs}s waiting for deployments: {}",
                not_ready.join("\n")
            );
        }

        sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    #[test]
    fn parse_env_preserves_values_containing_equals() -> anyhow::Result<()> {
        let vars = parse_env_file("export FOO=bar=baz=qux")?;
        assert_eq!(vars.get("FOO").unwrap(), "bar=baz=qux");

        Ok(())
    }

    #[test_case("foo=bar", MISSING_EXPORT_ERROR; "no export prefix")]
    #[test_case("export no_equals_here", MALFORMED_ERROR; "no equals")]
    #[test_case("export foo=1\nexport foo=2", DUPLICATE_ERROR; "duplicate keys")]
    #[test]
    fn parse_env_errors_on_malformed_input(lines: &str, error_prefix: &str) {
        match parse_env_file(lines) {
            Ok(_) => panic!("Expected parsing to fail"),
            Err(e) => assert!(e.to_string().starts_with(error_prefix), "{e}"),
        }
    }

    fn extract_kompose_args(cmd: &Command) -> Vec<String> {
        // Debug format of Command includes the program and args
        let debug = format!("{cmd:?}");
        // Parse out the arguments by splitting on quotes
        debug
            .split('"')
            .enumerate()
            .filter(|(i, _)| i % 2 == 1) // odd indices are inside quotes
            .map(|(_, s)| s.to_owned())
            .collect()
    }

    #[test]
    fn build_kompose_command_builds_correct_args_for_single_compose_file() {
        let env_vars = HashMap::new();
        let k8s_dir = PathBuf::from("/tmp/k8s");
        let cmd = build_kompose_command("/path/to/docker-compose.yaml", &k8s_dir, &env_vars);

        let args = extract_kompose_args(&cmd);
        assert_eq!(args[0], "kompose");
        assert!(args.contains(&"convert".to_owned()));
        assert!(args.contains(&"-f".to_owned()));
        assert!(args.contains(&"/path/to/docker-compose.yaml".to_owned()));
        assert!(args.contains(&"-o".to_owned()));
        assert!(args.contains(&"/tmp/k8s".to_owned()));
    }

    #[test]
    fn build_kompose_command_builds_correct_args_for_multiple_compose_files() {
        let env_vars = HashMap::new();
        let k8s_dir = PathBuf::from("/tmp/k8s");
        let compose = r#"
                /path/to/a.yaml
                /path/to/b.yaml
                /path/to/c.yaml
            "#
        .trim();
        let cmd = build_kompose_command(compose, &k8s_dir, &env_vars);

        let args = extract_kompose_args(&cmd);
        let f_count = args.iter().filter(|a| a.as_str() == "-f").count();
        assert_eq!(f_count, 3);
    }
}

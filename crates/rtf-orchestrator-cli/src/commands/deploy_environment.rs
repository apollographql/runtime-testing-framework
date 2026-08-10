use crate::{
    context::{CliContext, FsError, FsErrorKind},
    info_status,
    kubernetes::Client,
    kubernetes::Error,
    orchestrator::Client as OrchestratorClient,
};
use anyhow::anyhow;
use rtf_config::formats::PullPolicyServices;
use rtf_orchestrator_shared::{OtelConfig, status::Status};
use std::{
    collections::HashMap, env::temp_dir, fs, io, path::Path, process::Command, time::Duration,
};
use tokio::time::{Instant, sleep};
use tracing::{info, warn};

const DUPLICATE_ERROR: &str =
    "Encountered a duplicate environment entry in the resolved RTF environment";
const MALFORMED_ERROR: &str = "Encountered a malformed line in the resolved RTF environment";
const MISSING_EXPORT_ERROR: &str = "Expected leading 'export ' prefix to env file line";

pub async fn deploy_environment(
    namespace: &str,
    kubeconfig_path: &Path,
    provider_dir_path: &Path,
    toolbox_pull_policy: &str,
    otel: &OtelConfig,
    timeout: u64,
    ctx: &impl CliContext,
) -> crate::Result<()> {
    let workdir_path = temp_dir().join("rtf-work");
    let k8s_dir_path = workdir_path.join("k8s");

    fs::create_dir_all(&k8s_dir_path).map_err(|source| FsError {
        kind: FsErrorKind::CreateDir,
        path: k8s_dir_path.clone(),
        source,
    })?;

    fs::create_dir_all(provider_dir_path).map_err(|source| FsError {
        kind: FsErrorKind::CreateDir,
        path: provider_dir_path.to_owned(),
        source,
    })?;

    info_status!(ctx, Status::Provisioning, "deploying environment")?;

    let cfg_bytes = ctx.orchestrator_client().fetch_environment_config().await?;

    let cfg_path = temp_dir().join("environment.yaml");
    ctx.write_file(&cfg_path, &cfg_bytes)?;

    info!("resolving environment docker-compose files");

    ctx.run_shell(Command::new("rtf").args([
        "resolve",
        "environment",
        &cfg_path.to_string_lossy(),
        "--outdir",
        &provider_dir_path.to_string_lossy(),
    ]))
    .await?;

    setup_env(
        &k8s_dir_path,
        provider_dir_path,
        toolbox_pull_policy,
        otel,
        ctx,
    )
    .await?;

    info!("applying manifests to namespace '{namespace}'");
    ctx.run_shell(Command::new("kubectl").args([
        "--kubeconfig",
        &kubeconfig_path.to_string_lossy(),
        "apply",
        "-n",
        namespace,
        "-f",
        &k8s_dir_path.join("out.yaml").to_string_lossy(),
    ]))
    .await?;

    wait_for_deployments(namespace, timeout, ctx).await?;
    info_status!(
        ctx,
        Status::Provisioning,
        "environment deployed successfully"
    )?;

    Ok(())
}

/// Sources COMPOSE_FILES from the resolved RTF environment and converts them to k8s manifests
async fn setup_env(
    k8s_dir_path: &Path,
    provider_dir_path: &Path,
    toolbox_pull_policy: &str,
    otel: &OtelConfig,
    ctx: &impl CliContext,
) -> crate::Result<()> {
    let setup_env = provider_dir_path.join("setup/setup.env");
    let env_contents = fs::read_to_string(&setup_env).map_err(|source| FsError {
        kind: FsErrorKind::Read,
        path: setup_env.clone(),
        source,
    })?;

    let env_vars = parse_env_file(&env_contents).map_err(|e| FsError {
        kind: FsErrorKind::Read,
        path: setup_env.clone(),
        source: io::Error::new(io::ErrorKind::InvalidData, e.to_string()),
    })?;

    let compose_files_path = env_vars.get("COMPOSE_FILES").ok_or_else(|| FsError {
        kind: FsErrorKind::Read,
        path: setup_env.clone(),
        source: io::Error::new(
            io::ErrorKind::InvalidData,
            "COMPOSE_FILES was not set by setup.env -- make sure environment.yaml is well-formed",
        ),
    })?;

    let compose_files_content =
        fs::read_to_string(compose_files_path).map_err(|source| FsError {
            kind: FsErrorKind::Read,
            path: compose_files_path.into(),
            source,
        })?;

    let compose_contents: Vec<String> = compose_files_content
        .lines()
        .filter_map(|path| fs::read_to_string(path).ok())
        .collect();

    let mut kompose_input = compose_files_content.clone();

    // kompose doesn't read compose's native `pull_policy` field (see
    // https://github.com/kubernetes/kompose/issues/1923), so translate it into the
    // `kompose.image-pull-policy` label it does read via an extra compose overlay file.
    if let Some(overlay) =
        PullPolicyServices::overlay_from_compose_files(compose_contents.iter().map(String::as_str))
    {
        let overlay_path = k8s_dir_path.join("pull-policy-overlay.yaml");
        ctx.write_file(&overlay_path, overlay.as_bytes())?;

        kompose_input.push('\n');
        kompose_input.push_str(&overlay_path.to_string_lossy());
    }

    info!("converting to kubernetes manifests");
    let mut kompose = build_kompose_command(
        &kompose_input,
        &k8s_dir_path.join("kompose-output.yaml"),
        &env_vars,
    );

    ctx.run_shell(&mut kompose).await?;

    ctx.write_file(
        &k8s_dir_path.join("kustomization.yaml"),
        ctx.orchestrator_client()
            .kustomize_patch_for_execution(provider_dir_path, toolbox_pull_policy, otel)
            .as_bytes(),
    )?;

    ctx.run_shell(Command::new("kubectl").args([
        "kustomize",
        k8s_dir_path.to_string_lossy().as_ref(),
        "-o",
        &k8s_dir_path.join("out.yaml").to_string_lossy(),
    ]))
    .await
}

/// Parse a `.env` file into a map of key-value pairs.
///
/// Expects all lines in the file to be of the form `export $key="$value"` — the quoted form
/// produced by `rtf resolve environment`. The surrounding double quotes are stripped so the
/// value behaves the way it would after `source`-ing the file in a shell.
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
                let unquoted = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                if let Some(_existing) = rtf_env.insert(key.to_owned(), unquoted.to_owned()) {
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
    kompose.args([
        "-o",
        &k8s_dir_path.to_string_lossy(),
        "--volumes",
        "emptyDir",
    ]);
    kompose.envs(env_vars);

    kompose
}

/// Wait for all Deployments in a namespace to have the Available condition.
async fn wait_for_deployments(
    namespace: &str,
    timeout_secs: u64,
    ctx: &impl CliContext,
) -> crate::Result<()> {
    let kube = ctx.kube_client();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    info_status!(
        ctx,
        Status::Provisioning,
        "waiting for deployments to become available"
    )?;

    loop {
        let status = kube.check_deployment_status(namespace).await?;
        if status.total > 0 && status.not_ready.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let not_ready: Vec<_> = status
                .not_ready
                .iter()
                .map(|d| format!("{}: {}", d.name, d.last_condition_status))
                .collect();

            warn!(
                "timed out after {timeout_secs}s; not-ready deployments: {}",
                not_ready.join(", ")
            );

            return Err(Error::DeploymentTimeout {
                timeout_secs,
                names: not_ready.join("\n"),
            }
            .into());
        }

        sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::mocks::MockContext;
    use assert_fs::{TempDir, prelude::*};
    use indoc::indoc;
    use serde::Deserialize;
    use serde_yaml::{Deserializer, Value};
    use simple_test_case::test_case;
    use std::{iter::once, path::PathBuf};

    #[test]
    fn parse_env_preserves_values_containing_equals() -> anyhow::Result<()> {
        let vars = parse_env_file("export FOO=bar=baz=qux")?;
        assert_eq!(vars.get("FOO").unwrap(), "bar=baz=qux");

        Ok(())
    }

    #[test]
    fn parse_env_strips_surrounding_double_quotes() -> anyhow::Result<()> {
        // `rtf resolve environment` emits `export KEY="value"`; the quotes would be stripped
        // by `source`, and we need to mirror that behaviour so values like file paths don't
        // carry stray quote characters.
        let vars = parse_env_file(
            "export COMPOSE_FILES=\"/tmp/rtf-work/output/setup/compose-files.txt\"",
        )?;
        assert_eq!(
            vars.get("COMPOSE_FILES").unwrap(),
            "/tmp/rtf-work/output/setup/compose-files.txt"
        );

        Ok(())
    }

    #[test]
    fn parse_env_leaves_unquoted_values_intact() -> anyhow::Result<()> {
        let vars = parse_env_file("export FOO=bar")?;
        assert_eq!(vars.get("FOO").unwrap(), "bar");

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

    #[test]
    #[ignore = "requires kompose on the PATH"]
    fn kompose_convert_applies_translated_pull_policy_label() {
        let tmp = TempDir::new().unwrap();

        let compose_content = indoc! {r#"
            services:
              web:
                image: nginx:alpine
                pull_policy: always
            "#};

        let compose_file = tmp.child("compose.yaml");
        compose_file.write_str(compose_content).unwrap();

        let overlay = PullPolicyServices::overlay_from_compose_files(once(compose_content))
            .expect("expected a pull-policy overlay for a service with pull_policy set");

        let overlay_file = tmp.child("pull-policy-overlay.yaml");
        overlay_file.write_str(&overlay).unwrap();

        let compose_files_content = format!(
            "{}\n{}",
            compose_file.path().display(),
            overlay_file.path().display()
        );

        let output_file = tmp.child("kompose-output.yaml");
        let mut kompose =
            build_kompose_command(&compose_files_content, output_file.path(), &HashMap::new());
        let status = kompose
            .status()
            .expect("failed to run kompose - is it on the PATH?");
        assert!(status.success(), "kompose convert failed");

        let output = fs::read_to_string(output_file.path()).unwrap();

        let deployment = Deserializer::from_str(&output)
            .map(|doc| Value::deserialize(doc).expect("valid yaml document"))
            .find(|doc| doc.get("kind").and_then(|k| k.as_str()) == Some("Deployment"))
            .expect("expected kompose to produce a Deployment");

        let image_pull_policy = deployment
            .get("spec")
            .and_then(|v| v.get("template"))
            .and_then(|v| v.get("spec"))
            .and_then(|v| v.get("containers"))
            .and_then(|v| v.as_sequence())
            .and_then(|containers| containers.first())
            .and_then(|c| c.get("imagePullPolicy"))
            .and_then(|v| v.as_str());

        assert_eq!(image_pull_policy, Some("Always"));
    }

    #[tokio::test]
    async fn wait_for_deployments_returns_ok_when_all_available() {
        let ctx = MockContext::default();
        let result = wait_for_deployments("test-ns", 10, &ctx).await;
        assert!(result.is_ok());
    }
}

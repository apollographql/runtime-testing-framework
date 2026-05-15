use crate::{
    context::{CliContext, FsError, FsErrorKind},
    info_status,
    orchestrator::Client as _,
};
use http::Request;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, api::LogParams};
use rep_orchestrator_shared::{EXECUTION_ID_ENV_VAR, status::Status};
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{self, Cursor, Write},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};
use tokio::{fs, time::sleep};
use tracing::{info, warn};
use zip::{ZipWriter, write::SimpleFileOptions};

const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Unrunnable(#[from] crate::Error),

    #[error("scenario exited with non-zero exit code {exit_code}")]
    ScenarioFailed { exit_code: u8 },
}

impl Error {
    pub fn status_and_exit_status(&self) -> (Status, Option<ExitStatus>) {
        match self {
            Error::Unrunnable(_) => (Status::Unrunnable, None),
            Error::ScenarioFailed { exit_code } => (
                Status::Failed,
                Some(ExitStatus::from_raw((*exit_code as i32) << 8)),
            ),
        }
    }
}

/// Upload the scenario's log file and zipped output directory, then report the terminal
/// execution status derived from the scenario's exit code.
///
/// Runs alongside the scenario-runner container, waiting for it to touch the exit sentinel
/// file before reading artifacts off the shared volume.
pub async fn collect_output(shared_dir: &Path, ctx: &impl CliContext) -> Result<(), Error> {
    let paths = SharedPaths::new(shared_dir);

    collect_output_inner(&paths, ctx)
        .await
        .map_err(Error::Unrunnable)?;

    let exit_code = read_exit_code(ctx, &paths.exit_status_file)?;

    if exit_code == 0 {
        info_status!(ctx, Status::Successful, "scenario completed successfully")?;

        Ok(())
    } else {
        Err(Error::ScenarioFailed { exit_code })
    }
}

async fn collect_output_inner(paths: &SharedPaths, ctx: &impl CliContext) -> crate::Result<()> {
    info_status!(ctx, Status::Running, "waiting for scenario to complete")?;
    wait_for_sentinel(ctx, &paths.exit_sentinel, POLL_INTERVAL).await;

    info!("requesting upload URLs");
    let urls = ctx.orchestrator_client().generate_upload_urls().await?;

    info!("uploading log file");
    let log_bytes = ctx.read_file(&paths.output_log)?;

    ctx.orchestrator_client()
        .upload_to_signed_url(&urls.log_file_url, log_bytes)
        .await?;

    info!("collecting execution namespace artifacts");
    match (
        env::var(EXECUTION_ID_ENV_VAR).ok(),
        kube::Client::try_default().await,
    ) {
        (Some(ns), Ok(client)) => {
            let output_dir = paths.base.join("output");
            collect_container_logs(client.clone(), &ns, &output_dir).await;
            collect_namespace_events(client.clone(), &ns, &output_dir).await;
            collect_resource_metrics(client, &ns, &output_dir).await;
        }
        (None, _) => warn!("execution ID env var not set, skipping artifact collection"),
        (_, Err(e)) => warn!("failed to build kube client, skipping artifact collection: {e}"),
    }

    info!("building output zip");
    build_output_zip(ctx, &paths.base, &paths.output_zip)?;
    let zip_bytes = ctx.read_file(&paths.output_zip)?;

    info!("uploading output zip");
    ctx.orchestrator_client()
        .upload_to_signed_url(&urls.output_zip_url, zip_bytes)
        .await?;

    Ok(())
}

struct SharedPaths {
    base: PathBuf,
    output_log: PathBuf,
    output_zip: PathBuf,
    exit_sentinel: PathBuf,
    exit_status_file: PathBuf,
}

impl SharedPaths {
    fn new(shared_dir: &Path) -> Self {
        Self {
            base: shared_dir.into(),
            output_log: shared_dir.join("output").join("output.log"),
            output_zip: shared_dir.join("output.zip"),
            exit_sentinel: shared_dir.join("scenario-exited"),
            exit_status_file: shared_dir.join("scenario_exit_status"),
        }
    }
}

async fn wait_for_sentinel(ctx: &impl CliContext, sentinel: &Path, poll_interval: Duration) {
    while !ctx.path_exists(sentinel) {
        sleep(poll_interval).await;
    }
}

fn read_exit_code(ctx: &impl CliContext, exit_status_file: &Path) -> crate::Result<u8> {
    let contents = ctx.read_file_to_string(exit_status_file)?;

    contents.trim().parse::<u8>().map_err(|_| {
        FsError {
            path: exit_status_file.to_owned(),
            kind: FsErrorKind::Read,
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected exit code content in {}: {:?}",
                    exit_status_file.display(),
                    contents.trim()
                ),
            ),
        }
        .into()
    })
}

/// Produce a zip archive of `<shared_dir>/output/` in-process, writing it to `zip_path`.
/// Archive entries are rooted at `output/` (i.e. relative to `shared_dir`), mirroring
/// `cd <shared_dir> && zip -r <zip_path> output`.
fn build_output_zip(
    ctx: &impl CliContext,
    shared_dir: &Path,
    zip_path: &Path,
) -> crate::Result<()> {
    let output_dir = shared_dir.join("output");
    let files = ctx.list_files_under(&output_dir)?;

    let mut buf = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut buf));
        let options = SimpleFileOptions::default();

        for file_path in files {
            let archive_name = file_path.strip_prefix(shared_dir).map_err(|_| FsError {
                path: file_path.clone(),
                kind: FsErrorKind::Read,
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "file {} is not under shared dir {}",
                        file_path.display(),
                        shared_dir.display()
                    ),
                ),
            })?;

            writer
                .start_file(archive_name.to_string_lossy(), options)
                .map_err(|e| FsError {
                    path: zip_path.to_owned(),
                    kind: FsErrorKind::Write,
                    source: io::Error::other(e),
                })?;
            let bytes = ctx.read_file(&file_path)?;
            writer.write_all(&bytes).map_err(|source| FsError {
                path: zip_path.to_owned(),
                kind: FsErrorKind::Write,
                source,
            })?;
        }

        writer.finish().map_err(|e| FsError {
            path: zip_path.to_owned(),
            kind: FsErrorKind::Write,
            source: io::Error::other(e),
        })?;
    }

    ctx.write_file(zip_path, &buf)?;

    Ok(())
}

async fn collect_container_logs(client: kube::Client, namespace: &str, output_dir: &Path) {
    let pod_api: Api<Pod> = Api::namespaced(client, namespace);

    let pods = match pod_api.list(&Default::default()).await {
        Ok(p) => p,
        Err(e) => {
            warn!("failed to list pods for log collection: {e}");
            return;
        }
    };

    for pod in pods.items {
        let Some(pod_name) = pod.metadata.name.as_deref() else {
            continue;
        };

        let containers = pod
            .spec
            .iter()
            .flat_map(|s| {
                s.containers
                    .iter()
                    .chain(s.init_containers.iter().flatten())
            })
            .map(|c| c.name.clone())
            .collect::<Vec<_>>();

        for container_name in containers {
            let params = LogParams {
                container: Some(container_name.clone()),
                tail_lines: Some(10_000),
                ..Default::default()
            };

            match pod_api.logs(pod_name, &params).await {
                Ok(logs) => {
                    let log_path = output_dir
                        .join("logs")
                        .join(pod_name)
                        .join(format!("{container_name}.txt"));

                    if let Some(parent) = log_path.parent()
                        && let Err(e) = fs::create_dir_all(parent).await
                    {
                        warn!("failed to create log dir {}: {e}", parent.display());
                        continue;
                    }

                    if let Err(e) = fs::write(&log_path, logs.as_bytes()).await {
                        warn!("failed to write logs for {pod_name}/{container_name}: {e}");
                    }
                }
                Err(e) => warn!("failed to fetch logs for {pod_name}/{container_name}: {e}"),
            }
        }
    }
}

async fn collect_namespace_events(client: kube::Client, namespace: &str, output_dir: &Path) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/namespaces/{namespace}/events"))
        .body(vec![])
        .expect("events request URI is always valid");

    let body = match client.request_text(req).await {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to fetch namespace events: {e}");
            return;
        }
    };

    let path = output_dir.join("events.json");
    if let Err(e) = fs::write(&path, body.as_bytes()).await {
        warn!("failed to write events.json: {e}");
    }
}

async fn collect_resource_metrics(client: kube::Client, namespace: &str, output_dir: &Path) {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);

    let pods = match pod_api.list(&Default::default()).await {
        Ok(p) => p,
        Err(e) => {
            warn!("failed to list pods for metric collection: {e}");
            return;
        }
    };

    let nodes: std::collections::BTreeSet<String> = pods
        .items
        .iter()
        .filter_map(|p| p.spec.as_ref()?.node_name.clone())
        .collect();

    let mut all_pods: Vec<PodStats> = Vec::new();

    for node in &nodes {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/nodes/{node}/proxy/stats/summary"))
            .body(vec![])
            .expect("stats/summary request URI is always valid");

        let text = match client.request_text(req).await {
            Ok(t) => t,
            Err(e) => {
                warn!("kubelet proxy unavailable for node {node}: {e}");
                continue;
            }
        };

        let summary: NodeSummary = match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to deserialize stats/summary for node {node}: {e}");
                continue;
            }
        };

        all_pods.extend(
            summary
                .pods
                .into_iter()
                .filter(|p| p.pod_ref.namespace == namespace),
        );
    }

    let json = match serde_json::to_string_pretty(&PodResourceReport { pods: all_pods }) {
        Ok(j) => j,
        Err(e) => {
            warn!("failed to serialize resource metrics: {e}");
            return;
        }
    };

    let path = output_dir.join("resource-metrics.json");
    if let Err(e) = fs::write(&path, json.as_bytes()).await {
        warn!("failed to write resource-metrics.json: {e}");
    }
}

#[derive(Debug, Deserialize)]
struct NodeSummary {
    pods: Vec<PodStats>,
}

#[derive(Debug, Serialize)]
struct PodResourceReport {
    pods: Vec<PodStats>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PodStats {
    pod_ref: PodRef,
    containers: Vec<ContainerStats>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PodRef {
    name: String,
    namespace: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerStats {
    name: String,
    start_time: Option<String>,
    cpu: Option<CpuStats>,
    memory: Option<MemoryStats>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CpuStats {
    usage_nano_cores: Option<u64>,
    usage_core_nano_seconds: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryStats {
    usage_bytes: Option<u64>,
    working_set_bytes: Option<u64>,
    rss_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::mocks::MockContext;
    use std::io::Read;
    use zip::ZipArchive;

    const SHARED_DIR: &str = "/shared";
    const EXIT_STATUS_FILE: &str = "/shared/scenario_exit_status";
    const SENTINEL: &str = "/shared/scenario-exited";

    #[test]
    fn read_exit_code_parses_valid_u8() {
        let ctx = MockContext::default();
        ctx.write_file(Path::new(EXIT_STATUS_FILE), b"42\n")
            .unwrap();

        assert_eq!(
            read_exit_code(&ctx, Path::new(EXIT_STATUS_FILE)).unwrap(),
            42
        );
    }

    #[test]
    fn read_exit_code_errors_on_missing_file() {
        let res = read_exit_code(&MockContext::default(), Path::new(EXIT_STATUS_FILE));

        assert!(res.is_err(), "{res:?}");
    }

    #[test]
    fn read_exit_code_errors_on_garbage() {
        let ctx = MockContext::default();
        ctx.write_file(Path::new(EXIT_STATUS_FILE), b"not-a-number")
            .unwrap();
        let res = read_exit_code(&ctx, Path::new(EXIT_STATUS_FILE));

        assert!(res.is_err(), "{res:?}");
    }

    #[tokio::test]
    async fn wait_for_sentinel_returns_immediately_when_already_present() {
        let ctx = MockContext::default();
        ctx.write_file(Path::new(SENTINEL), b"").unwrap();

        wait_for_sentinel(&ctx, Path::new(SENTINEL), Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn wait_for_sentinel_returns_once_sentinel_appears() {
        let ctx = MockContext::default();
        let fs = ctx.fs.clone();

        let writer = tokio::spawn(async move {
            sleep(Duration::from_millis(30)).await;
            fs.files
                .write()
                .unwrap()
                .insert(PathBuf::from(SENTINEL), Vec::new());
        });

        wait_for_sentinel(&ctx, Path::new(SENTINEL), Duration::from_millis(10)).await;
        writer.await.unwrap();
    }

    #[test]
    fn build_output_zip_produces_entries_rooted_at_output() {
        let ctx = MockContext::default();
        ctx.write_file(Path::new("/shared/output/RTF_OUTPUT"), b"hello")
            .unwrap();
        ctx.write_file(Path::new("/shared/output/sub/nested.txt"), b"n")
            .unwrap();

        let zip_path = Path::new("/shared/output.zip");
        build_output_zip(&ctx, Path::new(SHARED_DIR), zip_path).expect("zip should succeed");

        let bytes = ctx.read_file(zip_path).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();

        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_owned())
            .collect();
        assert!(
            names.iter().all(|n| n.starts_with("output/")),
            "entries should be rooted at 'output/' — got: {names:?}"
        );
        assert!(names.iter().any(|n| n == "output/RTF_OUTPUT"));
        assert!(names.iter().any(|n| n == "output/sub/nested.txt"));

        let mut content = String::new();
        zip.by_name("output/RTF_OUTPUT")
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        assert_eq!(content, "hello");
    }
}

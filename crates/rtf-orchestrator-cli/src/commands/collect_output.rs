use crate::{
    context::{CliContext, FsError, FsErrorKind},
    info_status,
    kubernetes::Client as KubeClient,
    orchestrator::Client as _,
};
use chrono::{DateTime, Utc};
use rtf_config::formats::PrometheusQuery;
use rtf_integrations::prometheus::{Client as PrometheusClientTrait, PrometheusClient};
use rtf_orchestrator_shared::{PrometheusQueries, status::Status};
use std::{
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
pub async fn collect_output(
    shared_dir: &Path,
    prometheus_endpoint: &str,
    ctx: &impl CliContext,
) -> Result<(), Error> {
    let paths = SharedPaths::new(shared_dir);

    collect_output_inner(&paths, prometheus_endpoint, ctx)
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

async fn collect_output_inner(
    paths: &SharedPaths,
    prometheus_endpoint: &str,
    ctx: &impl CliContext,
) -> crate::Result<()> {
    info_status!(ctx, Status::Running, "waiting for scenario to complete")?;
    wait_for_sentinel(ctx, &paths.exit_sentinel, POLL_INTERVAL).await;

    info!("requesting upload URLs");
    let urls = ctx.orchestrator_client().generate_upload_urls().await?;

    info!("uploading log file");
    let log_bytes = ctx.read_file(&paths.output_log)?;

    ctx.orchestrator_client()
        .upload_to_signed_url(&urls.log_file_url, log_bytes)
        .await?;

    let ns = ctx.execution_namespace();
    let output_dir = paths.base.join("output");

    let execution_variables = match ctx.orchestrator_client().fetch_output_collection().await {
        Ok(output) => {
            let prometheus_client = PrometheusClient::new(prometheus_endpoint);
            collect_prometheus_metrics(
                &output.prometheus,
                ns,
                &output_dir,
                &prometheus_client,
                ctx,
            )
            .await;

            output.execution_variables
        }

        Err(e) => {
            let path = output_dir.join("prometheus.txt");
            let err_str = format!(
                "failed to fetch output collection config, skipping metric collection: {e}"
            );
            warn!("{err_str}");

            if let Err(e) = fs::write(&path, &err_str).await {
                warn!("failed to write {}: {e}", path.display());
            }

            "{}".to_string()
        }
    };

    let path = output_dir.join("variables.json");
    if let Err(e) = fs::write(&path, &execution_variables).await {
        warn!("failed to write {}: {e}", path.display());
    }

    info!("collecting execution namespace artifacts");
    ctx.kube_client()
        .collect_container_logs(ns, &output_dir)
        .await;
    ctx.kube_client()
        .collect_namespace_events(ns, &output_dir)
        .await;
    ctx.kube_client()
        .collect_resource_metrics(ns, &output_dir)
        .await;

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

/// Fetches the execution's prometheus queries and, if any are configured, executes them and
/// writes results under `output_dir`. This is a soft failure end-to-end: any error here is
/// logged and swallowed rather than propagated, since a Prometheus outage or misconfiguration
/// must not fail the scenario execution — the scenario runner's exit code is what determines
/// the execution's outcome, never metric collection.
async fn collect_prometheus_metrics(
    prometheus_queries: &PrometheusQueries,
    namespace: &str,
    output_dir: &Path,
    prometheus_client: &impl PrometheusClientTrait,
    ctx: &impl CliContext,
) {
    if prometheus_queries.environment.is_empty() && prometheus_queries.scenario.is_empty() {
        return;
    }

    info!("getting scenario start and end time");
    let (start, end) = match ctx.kube_client().get_scenario_job_window(namespace).await {
        Ok(window) => window,
        Err(e) => {
            let path = output_dir.join("prometheus.txt");
            let err_str = format!(
                "failed to get scenario job window, skipping prometheus metric collection: {e}"
            );
            warn!("{err_str}");

            if let Err(e) = fs::write(&path, &err_str).await {
                warn!("failed to write {}: {e}", path.display());
            }

            return;
        }
    };

    execute_prometheus_queries(
        &prometheus_queries.environment,
        "environment",
        start,
        end,
        output_dir,
        prometheus_client,
    )
    .await;
    execute_prometheus_queries(
        &prometheus_queries.scenario,
        "scenario",
        start,
        end,
        output_dir,
        prometheus_client,
    )
    .await;
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

/// Executes prometheus queries and writes its result
async fn execute_prometheus_queries(
    queries: &[PrometheusQuery],
    source: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    output_dir: &Path,
    client: &impl PrometheusClientTrait,
) {
    if !queries.is_empty() {
        info!("executing {source} prometheus queries");
        for query in queries.iter() {
            let outcome = client
                .query_range(&query.query, &query.step, start, end)
                .await;

            let envelope = match &outcome {
                Ok(result) => prometheus_result_envelope(start, end, Ok(result)),
                Err(e) => {
                    warn!("prometheus query {} failed: {e}", query.name);
                    prometheus_result_envelope(start, end, Err(&e.to_string()))
                }
            };

            write_prometheus_result(output_dir, source, &query.name, &envelope).await;
        }
    }
}

/// Builds the JSON written for a single prometheus query's result: the time window it was
/// queried over, plus either the raw `result` data or an `error` message.
fn prometheus_result_envelope(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    outcome: Result<&serde_json::Value, &str>,
) -> serde_json::Value {
    let window = serde_json::json!({ "start": start, "end": end });

    match outcome {
        Ok(result) => serde_json::json!({ "window": window, "result": result }),
        Err(message) => serde_json::json!({ "window": window, "error": message }),
    }
}

/// Writes a prometheus query's result to
/// `<output_dir>/prometheus/{environment|scenario}/{name}.json`, creating any missing parent
/// directories first. Failures are logged and swallowed — one query's write failure shouldn't
/// prevent the rest of output collection from proceeding.
async fn write_prometheus_result(
    output_dir: &Path,
    source: &str,
    name: &str,
    result: &serde_json::Value,
) {
    let path = output_dir
        .join("prometheus")
        .join(source)
        .join(format!("{name}.json"));

    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent).await
    {
        warn!("failed to create directory {}: {e}", parent.display());
        return;
    }

    let json = serde_json::to_string_pretty(result).expect("Value always serializes");
    if let Err(e) = fs::write(&path, json.as_bytes()).await {
        warn!("failed to write {}: {e}", path.display());
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::mocks::MockContext, kubernetes::mocks::KubeCall,
        orchestrator::mocks::MockClient as MockOrchestrator,
    };
    use assert_fs::{TempDir, prelude::*};
    use predicates::path::{exists, missing};
    use reqwest::StatusCode;
    use rtf_integrations::prometheus;
    use serde_json::Value;
    use simple_test_case::test_case;
    use std::{
        io::Read,
        sync::{Arc, RwLock},
    };
    use zip::ZipArchive;

    const SHARED_DIR: &str = "/shared";
    const EXIT_STATUS_FILE: &str = "/shared/scenario_exit_status";
    const SENTINEL: &str = "/shared/scenario-exited";

    #[derive(Debug, Clone, PartialEq)]
    pub struct QueryRangeCall {
        pub query: String,
        pub step: String,
    }

    struct MockState {
        calls: Vec<QueryRangeCall>,
        should_fail: bool,
        result: Value,
    }

    impl Default for MockState {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                should_fail: false,
                result: Value::Array(Vec::new()),
            }
        }
    }

    #[derive(Clone, Default)]
    pub struct MockPrometheus {
        state: Arc<RwLock<MockState>>,
    }

    impl MockPrometheus {
        pub fn failing() -> Self {
            Self {
                state: Arc::new(RwLock::new(MockState {
                    should_fail: true,
                    ..Default::default()
                })),
            }
        }

        fn record_call(&self, call: QueryRangeCall) {
            self.state.write().unwrap().calls.push(call);
        }

        pub fn read_calls<F>(&self, closure: F)
        where
            F: FnOnce(&[QueryRangeCall]),
        {
            let state = self.state.read().unwrap();
            closure(&state.calls)
        }
    }

    impl PrometheusClientTrait for MockPrometheus {
        async fn query_range(
            &self,
            query: &str,
            step: &str,
            _start: DateTime<Utc>,
            _end: DateTime<Utc>,
        ) -> prometheus::Result<Value> {
            let (should_fail, result) = {
                let state = self.state.read().unwrap();
                (state.should_fail, state.result.clone())
            };

            if should_fail {
                return Err(prometheus::Error::Api {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    error_type: "mock_failure".to_owned(),
                    message: "mock prometheus failure".to_owned(),
                });
            }

            self.record_call(QueryRangeCall {
                query: query.to_owned(),
                step: step.to_owned(),
            });

            Ok(result)
        }
    }

    fn write_sentinel_and_log(ctx: &MockContext, shared_dir: &Path) {
        ctx.write_file(&shared_dir.join("scenario-exited"), b"")
            .unwrap();
        ctx.write_file(
            &shared_dir.join("output").join("output.log"),
            b"log contents",
        )
        .unwrap();
    }

    fn prometheus_query(name: &str) -> PrometheusQuery {
        PrometheusQuery {
            name: name.to_owned(),
            step: "15s".to_owned(),
            query: "up".to_owned(),
        }
    }

    fn prometheus_queries(environment: &[&str], scenario: &[&str]) -> PrometheusQueries {
        PrometheusQueries {
            environment: environment.iter().map(|n| prometheus_query(n)).collect(),
            scenario: scenario.iter().map(|n| prometheus_query(n)).collect(),
        }
    }

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        (
            DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            DateTime::parse_from_rfc3339("2024-01-01T01:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

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

    #[tokio::test]
    async fn collect_output_inner_succeeds_with_clean_kube_client() {
        let ctx = MockContext::default();
        write_sentinel_and_log(&ctx, Path::new(SHARED_DIR));

        let paths = SharedPaths::new(Path::new(SHARED_DIR));
        collect_output_inner(&paths, "dummy endpoint", &ctx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn collect_output_inner_writes_error_file_when_output_collection_fetch_fails() {
        let shared_dir = TempDir::new().unwrap();
        fs::create_dir_all(shared_dir.child("output").path())
            .await
            .unwrap();

        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::with_failing_output_collection(),
            ..Default::default()
        };
        write_sentinel_and_log(&ctx, shared_dir.path());

        let paths = SharedPaths::new(shared_dir.path());

        // Fetching the output collection config is a soft failure: it must not fail the
        // scenario, only skip metric collection and leave a record of why.
        let res = collect_output_inner(&paths, "dummy endpoint", &ctx).await;
        assert!(
            res.is_ok(),
            "expected collect_output_inner to succeed, got {res:?}"
        );

        let error_file = shared_dir.child("output/prometheus.txt");
        error_file.assert(exists());

        let contents = fs::read_to_string(error_file.path()).await.unwrap();
        assert!(
            contents
                .contains("failed to fetch output collection config, skipping metric collection"),
            "unexpected contents: {contents}"
        );
    }

    #[test]
    fn prometheus_result_envelope_wraps_a_successful_result() {
        let (start, end) = window();
        let result = serde_json::json!(["ok"]);

        let envelope = prometheus_result_envelope(start, end, Ok(&result));

        assert_eq!(
            envelope,
            serde_json::json!({
                "window": { "start": "2024-01-01T00:00:00Z", "end": "2024-01-01T01:00:00Z" },
                "result": ["ok"]
            })
        );
    }

    #[test]
    fn prometheus_result_envelope_captures_an_error_message() {
        let (start, end) = window();

        let envelope = prometheus_result_envelope(start, end, Err("query timed out"));

        assert_eq!(
            envelope,
            serde_json::json!({
                "window": { "start": "2024-01-01T00:00:00Z", "end": "2024-01-01T01:00:00Z" },
                "error": "query timed out"
            })
        );
    }

    #[test_case(
        prometheus_queries(&[], &[]),
        &[],
        false;
        "no queries means no metrics written"
    )]
    #[test_case(
        prometheus_queries(&["up"], &[]),
        &["prometheus/environment/up.json"],
        true;
        "single environment query"
    )]
    #[test_case(
        prometheus_queries(&["up", "latency"], &[]),
        &["prometheus/environment/up.json", "prometheus/environment/latency.json"],
        true;
        "multiple environment queries"
    )]
    #[test_case(
        prometheus_queries(&[], &["up"]),
        &["prometheus/scenario/up.json"],
        true;
        "single scenario query"
    )]
    #[test_case(
        prometheus_queries(&[], &["up", "latency"]),
        &["prometheus/scenario/up.json", "prometheus/scenario/latency.json"],
        true;
        "multiple scenario queries"
    )]
    #[test_case(
        prometheus_queries(&["up"], &["latency"]),
        &["prometheus/environment/up.json", "prometheus/scenario/latency.json"],
        true;
        "mixture of environment and scenario queries"
    )]
    #[tokio::test]
    async fn collect_prometheus_metrics_writes_expected_files(
        prom_queries: PrometheusQueries,
        expected_paths: &[&str],
        expect_window_call: bool,
    ) {
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::with_prometheus_queries(prom_queries.clone()),
            ..Default::default()
        };
        let output_dir = TempDir::new().unwrap();
        let prometheus_client = MockPrometheus::default();

        collect_prometheus_metrics(
            &prom_queries,
            "test-namespace",
            output_dir.path(),
            &prometheus_client,
            &ctx,
        )
        .await;

        ctx.kube_client.read_calls(|calls| {
            assert_eq!(
                calls
                    .iter()
                    .any(|c| matches!(c, KubeCall::GetScenarioJobWindow { .. })),
                expect_window_call,
                "unexpected get_scenario_job_window call state, got: {calls:?}"
            );
        });

        prometheus_client.read_calls(|calls| {
            assert_eq!(
                calls.len(),
                expected_paths.len(),
                "expected one prometheus query per expected file, got: {calls:?}"
            );
        });

        for path in expected_paths {
            output_dir.child(path).assert(exists());
        }

        if expected_paths.is_empty() {
            output_dir.child("prometheus").assert(missing());
        }
        if !expected_paths
            .iter()
            .any(|p| p.starts_with("prometheus/environment"))
        {
            output_dir.child("prometheus/environment").assert(missing());
        }
        if !expected_paths
            .iter()
            .any(|p| p.starts_with("prometheus/scenario"))
        {
            output_dir.child("prometheus/scenario").assert(missing());
        }
    }

    #[tokio::test]
    async fn collect_prometheus_metrics_still_writes_file_when_query_fails() {
        let queries = prometheus_queries(&["up"], &[]);
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::with_prometheus_queries(queries.clone()),
            ..Default::default()
        };
        let output_dir = TempDir::new().unwrap();

        // A failing prometheus client is captured as a per-query error, not something
        // collect_prometheus_metrics needs to handle specially — the file is still written.
        collect_prometheus_metrics(
            &queries,
            "test-namespace",
            output_dir.path(),
            &MockPrometheus::failing(),
            &ctx,
        )
        .await;

        output_dir
            .child("prometheus/environment/up.json")
            .assert(exists());
    }

    #[tokio::test]
    async fn collect_prometheus_metrics_skips_query_execution_when_job_window_fails() {
        let queries = prometheus_queries(&["up"], &[]);
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::with_prometheus_queries(queries.clone()),
            kube_client: crate::kubernetes::mocks::MockClient::failing(),
            ..Default::default()
        };
        let output_dir = TempDir::new().unwrap();

        collect_prometheus_metrics(
            &queries,
            "test-namespace",
            output_dir.path(),
            &MockPrometheus::default(),
            &ctx,
        )
        .await;

        output_dir.child("prometheus").assert(missing());

        let error_file = output_dir.child("prometheus.txt");
        error_file.assert(exists());

        let contents = fs::read_to_string(error_file.path()).await.unwrap();
        assert!(
            contents.contains(
                "failed to get scenario job window, skipping prometheus metric collection"
            ),
            "unexpected contents: {contents}"
        );
    }
}

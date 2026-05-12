use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    info_status,
    orchestrator::Client as _,
};
use anyhow::Context;
use rep_orchestrator_shared::status::Status;
use std::{
    io::{Cursor, Write},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};
use tokio::time::sleep;
use tracing::info;
use zip::{ZipWriter, write::SimpleFileOptions};

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Upload the scenario's log file and zipped output directory, then report the terminal
/// execution status derived from the scenario's exit code.
///
/// Runs alongside the scenario-runner container, waiting for it to touch the exit sentinel
/// file before reading artifacts off the shared volume.
pub async fn collect_output(shared_dir: &Path, ctx: &impl CliContext) -> CliResult<()> {
    let paths = SharedPaths::new(shared_dir);

    info_status!(ctx, Status::Running, "waiting for scenario to complete")?;
    wait_for_sentinel(ctx, &paths.exit_sentinel, POLL_INTERVAL).await;

    info!("requesting upload URLs");
    let urls = ctx
        .orchestrator_client()
        .generate_upload_urls()
        .await
        .map_err(CliError::unrunnable)?;

    info!("uploading log file");
    let log_bytes = ctx.read_file(&paths.output_log)?;
    ctx.orchestrator_client()
        .upload_to_signed_url(&urls.log_file_url, log_bytes)
        .await
        .map_err(CliError::unrunnable)?;

    info!("building output zip");
    build_output_zip(ctx, shared_dir, &paths.output_zip)?;
    let zip_bytes = ctx.read_file(&paths.output_zip)?;

    info!("uploading output zip");
    ctx.orchestrator_client()
        .upload_to_signed_url(&urls.output_zip_url, zip_bytes)
        .await
        .map_err(CliError::unrunnable)?;

    let exit_code = read_exit_code(ctx, &paths.exit_status_file)?;
    if exit_code == 0 {
        info_status!(ctx, Status::Successful, "scenario completed successfully")?;

        Ok(())
    } else {
        Err(CliError::failed(
            exit_status_from_code(exit_code),
            format!("Scenario exited with non-zero exit code {exit_code}"),
        ))
    }
}

struct SharedPaths {
    output_log: PathBuf,
    output_zip: PathBuf,
    exit_sentinel: PathBuf,
    exit_status_file: PathBuf,
}

impl SharedPaths {
    fn new(shared_dir: &Path) -> Self {
        Self {
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

fn read_exit_code(ctx: &impl CliContext, exit_status_file: &Path) -> CliResult<u8> {
    let contents = ctx.read_file_to_string(exit_status_file)?;

    contents
        .trim()
        .parse::<u8>()
        .with_context(|| {
            format!(
                "Unexpected exit code in {}: {contents:?}",
                exit_status_file.display()
            )
        })
        .map_err(CliError::unrunnable)
}

/// Produce a zip archive of `<shared_dir>/output/` in-process, writing it to `zip_path`.
/// Archive entries are rooted at `output/` (i.e. relative to `shared_dir`), mirroring
/// `cd <shared_dir> && zip -r <zip_path> output`.
fn build_output_zip(ctx: &impl CliContext, shared_dir: &Path, zip_path: &Path) -> CliResult<()> {
    let output_dir = shared_dir.join("output");
    let files = ctx.list_files_under(&output_dir)?;

    let mut buf = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut buf));
        let options = SimpleFileOptions::default();

        for file_path in files {
            let archive_name = file_path
                .strip_prefix(shared_dir)
                .with_context(|| {
                    format!(
                        "File {} is not under shared dir {}",
                        file_path.display(),
                        shared_dir.display()
                    )
                })
                .map_err(CliError::unrunnable)?;

            writer
                .start_file(archive_name.to_string_lossy(), options)
                .with_context(|| format!("Failed to add {} to zip", archive_name.display()))
                .map_err(CliError::unrunnable)?;
            let bytes = ctx.read_file(&file_path)?;
            writer
                .write_all(&bytes)
                .with_context(|| format!("Failed to write {} to zip", archive_name.display()))
                .map_err(CliError::unrunnable)?;
        }

        writer
            .finish()
            .with_context(|| format!("Failed to finalise zip at {}", zip_path.display()))
            .map_err(CliError::unrunnable)?;
    }

    ctx.write_file(zip_path, &buf)
}

fn exit_status_from_code(code: u8) -> ExitStatus {
    ExitStatus::from_raw((code as i32) << 8)
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
        let err = read_exit_code(&MockContext::default(), Path::new(EXIT_STATUS_FILE)).unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }

    #[test]
    fn read_exit_code_errors_on_garbage() {
        let ctx = MockContext::default();
        ctx.write_file(Path::new(EXIT_STATUS_FILE), b"not-a-number")
            .unwrap();
        let err = read_exit_code(&ctx, Path::new(EXIT_STATUS_FILE)).unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }

    #[test]
    fn exit_status_from_code_round_trips() {
        let status = exit_status_from_code(7);
        assert_eq!(status.code(), Some(7));
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

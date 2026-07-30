use crate::commands::get_context_and_check_outdir;
use futures::future::try_join_all;
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
use reqwest::{Method, StatusCode};
use rtf_config::context::ResolutionContext;
use rtf_integrations::orchestrator::OrchestratorClient;
use std::path::Path;
use tracing::{info, warn};
use uuid::Uuid;

const N_PARALLEL_FETCH: usize = 20;

pub async fn pull_run_output(run_id: Uuid, out_dir: &str, force: bool) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let client = OrchestratorClient::new_from_env().await?;

    let tr: TestRunSummary = client
        .get_json(&format!("test-run/{run_id}/status"))
        .await?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    ctx.write(
        out_dir.join("run-summary.json"),
        serde_json::to_string_pretty(&tr)?,
    )?;

    let out_dir = ctx.canonicalize_path(&out_dir)?;

    for chunk in tr.executions.chunks(N_PARALLEL_FETCH) {
        let sub_dirs = chunk
            .iter()
            .map(|ex| {
                let sub_dir = out_dir.join(&ex.name);
                ctx.create_dir_all(&sub_dir)?;
                Ok(sub_dir)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        try_join_all(
            chunk
                .iter()
                .zip(sub_dirs.iter())
                .map(|(ex, sub_dir)| write_output_for_execution(ex, sub_dir, &client, &ctx)),
        )
        .await?;
    }

    Ok(())
}

pub async fn pull_execution_output(ex_id: Uuid, out_dir: &str, force: bool) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let client = OrchestratorClient::new_from_env().await?;

    let ex: TestExecutionSummary = client
        .get_json(&format!("test-execution/{ex_id}/status"))
        .await?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    ctx.write(
        out_dir.join("execution-summary.json"),
        serde_json::to_string_pretty(&ex)?,
    )?;

    let out_dir = ctx.canonicalize_path(&out_dir)?;

    write_output_for_execution(&ex, &out_dir, &client, &ctx).await
}

async fn write_output_for_execution(
    ex: &TestExecutionSummary,
    out_dir: &Path,
    client: &OrchestratorClient,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("pulling execution log for {}", ex.id);
    fetch_execution_file(client, ex.id, "log.txt", out_dir, ctx).await?;

    info!("pulling execution output for {}", ex.id);
    fetch_execution_file(client, ex.id, "output.zip", out_dir, ctx).await?;

    Ok(())
}

/// Fetch a single execution file from the Orchestrator and write it to `out_dir/file_name`.
///
/// If the file cannot be found (404), the command shouldn't abort the whole pull -- instead
/// write the error to `out_dir/<file_name>.txt` (appending `.txt` only if `file_name` doesn't
/// already have it) so the rest of the executions still get pulled.
async fn fetch_execution_file(
    client: &OrchestratorClient,
    ex_id: Uuid,
    file_name: &str,
    out_dir: &Path,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    let endpoint = format!("test-execution/{ex_id}/{file_name}");
    let resp = client.request(Method::GET, &endpoint).await?.send().await?;

    if resp.status() == StatusCode::NOT_FOUND {
        let body = resp.text().await?;
        return write_not_found_placeholder(ex_id, file_name, &body, out_dir, ctx);
    }

    let bytes = resp.error_for_status()?.bytes().await?;
    ctx.write(out_dir.join(file_name), bytes)?;

    Ok(())
}

/// Record that `file_name` could not be found for execution `ex_id`, instead of failing the
/// whole pull.
///
/// The error is written to `out_dir/<file_name>.txt` (appending `.txt` only if `file_name`
/// doesn't already have it) so the rest of the executions still get pulled.
fn write_not_found_placeholder(
    ex_id: Uuid,
    file_name: &str,
    body: &str,
    out_dir: &Path,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    let error_file_name = if file_name.ends_with(".txt") {
        file_name.to_owned()
    } else {
        format!("{file_name}.txt")
    };

    warn!("{file_name} not found for execution {ex_id}, writing error to {error_file_name}");
    ctx.write(
        out_dir.join(error_file_name),
        format!("failed to fetch test-execution/{ex_id}/{file_name}: 404 Not Found\n\n{body}"),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::{TempDir, prelude::*};
    use predicates::{prelude::*, str::contains};
    use rtf_config::context::Context;
    use simple_test_case::test_case;

    const EX_ID: Uuid = Uuid::nil();

    #[test_case("log.txt", "log.txt"; "already_txt")]
    #[test_case("output.zip", "output.zip.txt"; "non_txt_gets_txt_appended")]
    #[test]
    fn not_found_placeholder_only_appends_txt_when_needed(
        file_name: &str,
        expected_error_file: &str,
    ) {
        let out_dir = TempDir::new().unwrap();
        let ctx = Context::new();

        write_not_found_placeholder(
            EX_ID,
            file_name,
            "execution output has expired",
            out_dir.path(),
            &ctx,
        )
        .unwrap();

        out_dir
            .child(expected_error_file)
            .assert(contains("execution output has expired").and(contains(EX_ID.to_string())));

        // The real (non-placeholder) file should never exist unless it's the same
        // path as the error placeholder (i.e. file_name already ended in .txt).
        if file_name != expected_error_file {
            out_dir.child(file_name).assert(predicate::path::missing());
        }
    }
}

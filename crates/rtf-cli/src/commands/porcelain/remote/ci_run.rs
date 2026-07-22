use crate::commands::plumbing::prepare_remote_trigger_payload;
use rep_orchestrator_shared::{
    status::Status,
    summary::{TestExecutionSummary, TestRunSummary},
};
use rtf_core::variables::Variables;
use rtf_integrations::orchestrator::OrchestratorClient;
use std::{process::exit, time::Duration};
use tabled::{Table, Tabled, settings::Style};
use tokio::time::sleep;
use tracing::warn;
use uuid::Uuid;

pub async fn ci_run(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    poll_interval_seconds: u64,
    variables: Variables,
) -> anyhow::Result<()> {
    println!("Preparing trigger payload for {test_plan_path}...");
    match (github, git_ref.as_ref()) {
        (true, Some(s)) => println!("  pulling from GitHub using git ref {s:?}"),
        (true, None) => println!("  pulling from GitHub"),
        _ => (),
    }

    let payload =
        prepare_remote_trigger_payload(test_plan_path, github, git_ref, variables).await?;
    let client = OrchestratorClient::new().await?;
    let poll_interval = Duration::from_secs(poll_interval_seconds);

    println!("Triggering test run...\n");
    let mut summary: TestRunSummary = client.post_json("test-run/trigger", &payload).await?;
    let id = summary.id;
    println!("Test run id: {id}\n");

    // Print the table headers and initial line
    let mut lines = vec![UpdateLine::from_summary(&mut summary)];
    let mut table = Table::new(&lines);
    table.with(Style::empty());
    println!("{table}");

    // Fetch and print the next line of the table at our poll interval
    while any_execution_ongoing(&summary) {
        match client.get_json(&format!("test-run/{id}/status")).await {
            Ok(new) => {
                summary = new;
                lines.push(UpdateLine::from_summary(&mut summary));

                let mut table = Table::new(&lines);
                table.with(Style::empty());
                let s = table.to_string();
                println!("{}", s.lines().last().unwrap());

                if summary.current_status.is_terminal() {
                    break;
                }
            }

            Err(e) => warn!("{e}"),
        }

        sleep(poll_interval).await;
    }

    let (s, exit_code) = match summary.current_status {
        Status::Successful => ("successful", 0),
        Status::Failed => ("failed", 1),
        _ => ("unrunnable", 2),
    };

    println!("\n\nTest run complete. Final status: {s}\n");
    println!("Run 'rtf remote request test-run/{id}/status' to view the summary for this run\n");
    println!("Run the following to fetch the status, log or output.zip for an execution:");
    println!("   rtf remote request test-execution/$ID/status");
    println!("   rtf remote request test-execution/$ID/log.txt");
    println!("   rtf remote request test-execution/$ID/output.zip > output.zip");

    if exit_code != 0 {
        println!(
            "\nFailed executions:\n\n{}",
            failed_execution_report(summary.executions)
        );
        exit(exit_code);
    }

    Ok(())
}

fn any_execution_ongoing(trs: &TestRunSummary) -> bool {
    !trs.current_status.is_terminal()
        || trs
            .executions
            .iter()
            .any(|ex| !ex.current_status.is_terminal())
}

#[derive(Debug, Default, Clone, Tabled)]
struct UpdateLine {
    status: String,
    running: usize,
    successful: usize,
    failed: usize,
    unrunnable: usize,
}

impl UpdateLine {
    fn from_summary(summary: &mut TestRunSummary) -> Self {
        summary.executions.sort_by_key(|s| s.updated_at);

        let mut line = UpdateLine::default();
        for ex in summary.executions.iter() {
            match ex.current_status {
                Status::Successful => line.successful += 1,
                Status::Failed => line.failed += 1,
                Status::Unrunnable => line.unrunnable += 1,
                _ => line.running += 1,
            }
        }

        // Maximum length of a Status string repr. We pad to ensure that the column size of our
        // table repr remainins fixed.
        line.status = format!("{:<12}", summary.current_status);

        line
    }
}

#[derive(Debug, Default, Clone, Tabled)]
struct FailedExecution {
    id: Uuid,
    name: String,
    execution_time: String,
    status: Status,
}

impl From<TestExecutionSummary> for FailedExecution {
    fn from(ex: TestExecutionSummary) -> Self {
        // We drop our time delta to seconds in terms of precision to prevent humantime from going
        // overboard on reporting things to an unnecessary level of detail.
        let delta = ex.completed_at.unwrap_or(ex.updated_at) - ex.started_at;
        let seconds = Duration::from_secs(delta.num_seconds().unsigned_abs());

        Self {
            id: ex.id,
            name: ex.name,
            execution_time: humantime::format_duration(seconds).to_string(),
            status: ex.current_status,
        }
    }
}

fn failed_execution_report(executions: Vec<TestExecutionSummary>) -> Table {
    let mut failed: Vec<FailedExecution> = executions
        .into_iter()
        .filter_map(|ex| {
            if [Status::Unrunnable, Status::Failed].contains(&ex.current_status) {
                Some(FailedExecution::from(ex))
            } else {
                None
            }
        })
        .collect();

    failed.sort_by_key(|f| f.name.clone());

    let mut table = Table::new(&failed);
    table.with(Style::empty());

    table
}

use crate::filter::Filter;
use ad_client::{MiniBufferSelection, tokio::BufferClient};
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::{
    payload::TriggerPayload,
    status::Status,
    summary::{TestExecutionSummary, TestRunSummary},
};
use reqwest::Method;
use rtf_integrations::orchestrator::OrchestratorClient;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Command, exit},
    time::Instant,
};
use tabled::{Table, Tabled, settings::Style};
use tokio::task::{self, JoinHandle};
use uuid::Uuid;

const HEADER: &str = "+rep-watch :: dashboard
You can right click on 'TR-uuid' and 'EX-uuid' to view the raw summary for that item\n\n";
const MAX_MESSAGE_CHARS: usize = 60;

pub struct Runner {
    pub start: Instant,
    orchestrator_client: OrchestratorClient,
    buffer_client: BufferClient,
}

impl Runner {
    pub async fn try_new() -> anyhow::Result<Self> {
        let orchestrator_client = OrchestratorClient::new().await?;
        let client = match ad_client::tokio::Client::new().await {
            Ok(client) => client
                .open_virtual("+rtf", "")
                .await
                .context("unable to create +rtf window")?,
            Err(e) => {
                eprintln!("unable to connect to ad\n{e}");
                exit(1);
            }
        };

        Ok(Self {
            start: Instant::now(),
            orchestrator_client,
            buffer_client: client,
        })
    }

    pub async fn get_initial_summary(&mut self) -> anyhow::Result<(String, TestRunSummary)> {
        match self
            .buffer_client
            .minibuffer_select("Select mode >", &["Test Run ID", "Test Plan Path"])
            .await?
        {
            MiniBufferSelection::Line { index: 0, .. } => {
                let s = match self
                    .buffer_client
                    .minibuffer_prompt("Test Run ID: ")
                    .await?
                {
                    Some(s) => s,
                    None => bail!("no ID provided, exiting"),
                };
                let id = Uuid::try_parse(&s)?;
                let tr_summary = self.get_run_status(id).await?;

                Ok(("unknown".into(), tr_summary))
            }

            MiniBufferSelection::Line { index: 1, .. } => {
                let tp_path = match self
                    .buffer_client
                    .minibuffer_prompt("Test plan path: ")
                    .await?
                {
                    Some(path) => path,
                    None => bail!("no path provided, exiting"),
                };

                let tr_summary = match self.trigger_run(&tp_path).await {
                    Ok(trs) => trs,
                    Err(e) => {
                        println!("Failed to trigger run: {e}");
                        return Err(e);
                    }
                };

                let tp_path = PathBuf::from(tp_path)
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();

                Ok((tp_path, tr_summary))
            }

            _ => bail!("invalid mode"),
        }
    }

    pub async fn spawn_event_filter(&self) -> anyhow::Result<JoinHandle<()>> {
        let client = self.buffer_client.clone();
        let filter = Filter::try_new().await?;

        Ok(task::spawn(async move {
            let res = client.run_event_filter(filter).await;

            if let Err(e) = res {
                println!("event filter failed: {e}");
            } else {
                println!("event filter exiting");
            }
        }))
    }

    pub async fn set_buffer_content(&mut self, content: impl AsRef<str>) {
        let addr = self.buffer_client.read_addr().await.unwrap_or("1:1".into());
        _ = self.buffer_client.write_xaddr(",").await;
        _ = self
            .buffer_client
            .write_xdot(&format!("{HEADER}{}", content.as_ref()))
            .await;
        _ = self.buffer_client.write_addr(&addr).await;
        _ = self.buffer_client.ctl("viewport-center", "").await;
    }

    pub async fn append_buffer_content(&mut self, content: impl AsRef<str>) {
        _ = self.buffer_client.append_to_body(content.as_ref()).await;
    }

    pub async fn trigger_run(&mut self, tp_path: &str) -> anyhow::Result<TestRunSummary> {
        self.set_buffer_content(format!("Triggering test run for {tp_path}"))
            .await;

        let payload_bytes = Command::new("rtf")
            .args(["rep", "prepare", tp_path])
            .output()?
            .stdout;

        let payload: TriggerPayload =
            serde_json::from_slice(&payload_bytes).context("failed to parse RTF output")?;

        let resp = self
            .orchestrator_client
            .request(Method::POST, "test-run/trigger")
            .await?
            .json(&payload)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            let raw: serde_json::Value = resp.json().await?;
            let s = serde_json::to_string_pretty(&raw)?;

            bail!("Failed to trigger:\n{s}")
        }
    }

    pub async fn get_run_status(&self, id: Uuid) -> anyhow::Result<TestRunSummary> {
        let resp = self
            .orchestrator_client
            .request(Method::GET, &format!("test-run/{id}/status"))
            .await?
            .send()
            .await?;

        Ok(resp.json().await?)
    }

    pub async fn render_run_details(
        &mut self,
        tr_summary: &mut TestRunSummary,
        tp_path: &str,
        footer: &str,
    ) {
        tr_summary.executions.sort_by_key(|s| s.name.clone());

        let mut counts: HashMap<Status, usize> = HashMap::new();
        for ex in tr_summary.executions.iter() {
            *counts.entry(ex.current_status).or_default() += 1;
        }

        let mut status_counts: Vec<_> = counts
            .into_iter()
            .map(|(status, count)| StatusCount { status, count })
            .collect();

        status_counts.sort_by_key(|s| s.status);

        let mut counts_table = Table::new(status_counts);
        counts_table.with(Style::sharp());

        let mut table = Table::new(tr_summary.executions.iter().map(ExLine::from));
        table.with(Style::sharp());

        let overview = format!(
            "Test Run details\nID: (TR-{})\nTest Plan: {tp_path}\nStarted at: {}\nLast updated at: {}\nCurrent status: {}",
            tr_summary.id,
            tr_summary.started_at,
            Utc::now(),
            tr_summary.current_status
        );

        self.set_buffer_content(format!("{overview}\n\n{counts_table}\n\n{table}\n{footer}",))
            .await;
    }
}

#[derive(Debug, Tabled)]
struct StatusCount {
    status: Status,
    count: usize,
}

#[derive(Debug, Tabled)]
struct ExLine {
    name: String,
    status: Status,
    message: String,
    updated_at: DateTime<Utc>,
    id: String,
}

impl From<&TestExecutionSummary> for ExLine {
    fn from(s: &TestExecutionSummary) -> Self {
        let mut message: String = s.status_history[0]
            .message
            .clone()
            .unwrap_or_default()
            .chars()
            .take(MAX_MESSAGE_CHARS)
            .collect();

        if message.chars().count() < MAX_MESSAGE_CHARS {
            message.extend(vec![' '; MAX_MESSAGE_CHARS - message.len()]);
        }

        Self {
            name: s.name.clone(),
            status: s.status_history[0].status,
            message,
            updated_at: s.updated_at,
            id: format!("(EX-{})", s.id),
        }
    }
}

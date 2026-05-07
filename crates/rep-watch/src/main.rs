use ad_client::{
    EventOutcome, MiniBufferSelection, Source,
    tokio::{AsyncEventFilter, Client as AdClient},
};
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::{
    payload::TriggerPayload,
    status::Status,
    summary::{TestExecutionSummary, TestRunSummary},
};
use reqwest::Client;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Command, exit},
    time::{Duration, Instant},
};
use tabled::{Table, Tabled, settings::Style};
use tokio::{
    task::{self, JoinHandle},
    time::sleep,
};
use uuid::Uuid;

const POLL_SECONDS: u64 = 5;
const MAX_MESSAGE_CHARS: usize = 60;
const HEADER: &str = "+rep-watch :: dashboard
You can right click on 'TR-uuid' and 'EX-uuid' to view the raw summary for that item\n\n";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut runner = Runner::new().await?;
    let mut footer = String::new();

    let (tp_path, mut trs) = runner.get_initial_summary().await?;
    runner.open_rtf_buffer().await?;

    let handle = runner.spawn_event_filter().await?;
    runner.render_run_details(&mut trs, &tp_path, &footer).await;

    while any_execution_ongoing(&trs) {
        match runner.get_run_status(trs.id).await {
            Ok(new) => trs = new,
            Err(e) => footer = format!("failed to pull run update: {e}"),
        }

        runner.render_run_details(&mut trs, &tp_path, &footer).await;
        footer.clear();
        sleep(Duration::from_secs(POLL_SECONDS)).await;
    }

    let t = Instant::now().duration_since(runner.start).as_secs();

    runner
        .append_buffer_content(format!("\nRun completed in {t}s"))
        .await;

    // wait for the +rtf buffer to close so we still run the event filter
    _ = handle.await;

    println!("exiting");

    Ok(())
}

struct Runner {
    start: Instant,
    bufid: usize,
    client: Client,
    ad_client: AdClient,
}

impl Runner {
    async fn new() -> anyhow::Result<Self> {
        let client = Client::new();
        let ad_client = match AdClient::new().await {
            Ok(client) => client,
            Err(e) => {
                eprintln!("unable to connect to ad\n{e}");
                exit(1);
            }
        };

        Ok(Self {
            start: Instant::now(),
            bufid: 0,
            client,
            ad_client,
        })
    }

    async fn get_initial_summary(&mut self) -> anyhow::Result<(String, TestRunSummary)> {
        match self
            .ad_client
            .minibuffer_select("Select mode >", &["Test Run ID", "Test Plan Path"])
            .await?
        {
            MiniBufferSelection::Line { index: 0, .. } => {
                let s = match self.ad_client.minibuffer_prompt("Test Run ID: ").await? {
                    Some(s) => s,
                    None => bail!("no ID provided, exiting"),
                };
                let id = Uuid::try_parse(&s)?;
                let tr_summary = self.get_run_status(id).await?;

                Ok(("unknown".into(), tr_summary))
            }

            MiniBufferSelection::Line { index: 1, .. } => {
                let tp_path = match self.ad_client.minibuffer_prompt("Test plan path: ").await? {
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

    async fn open_rtf_buffer(&mut self) -> anyhow::Result<()> {
        self.bufid = self
            .ad_client
            .open_virtual("+rtf", "")
            .await
            .context("unable to create +rtf window")?;

        Ok(())
    }

    async fn spawn_event_filter(&self) -> anyhow::Result<JoinHandle<()>> {
        let mut client = AdClient::new().await?;
        let bufid = self.bufid;

        Ok(task::spawn(async move {
            let res = client
                .run_event_filter(
                    bufid,
                    Filter {
                        client: Client::new(),
                    },
                )
                .await;

            if let Err(e) = res {
                println!("event filter failed: {e}");
            } else {
                println!("event filter exiting");
            }
        }))
    }

    async fn set_buffer_content(&mut self, content: impl AsRef<str>) {
        let addr = self
            .ad_client
            .read_addr(self.bufid)
            .await
            .unwrap_or("1:1".into());
        _ = self.ad_client.write_xaddr(self.bufid, ",").await;
        _ = self
            .ad_client
            .write_xdot(self.bufid, &format!("{HEADER}{}", content.as_ref()))
            .await;
        _ = self.ad_client.write_addr(self.bufid, &addr).await;
        _ = self.ad_client.ctl("viewport-center", "").await;
    }

    async fn append_buffer_content(&mut self, content: impl AsRef<str>) {
        _ = self
            .ad_client
            .append_to_body(self.bufid, content.as_ref())
            .await;
    }

    async fn trigger_run(&mut self, tp_path: &str) -> anyhow::Result<TestRunSummary> {
        self.set_buffer_content(format!("Triggering test run for {tp_path}"))
            .await;

        let payload_bytes = Command::new("rtf")
            .args(["rep", "prepare", tp_path])
            .output()?
            .stdout;

        let _payload: TriggerPayload =
            serde_json::from_slice(&payload_bytes).context("failed to parse RTF output")?;

        let resp = self
            .client
            .post("http://localhost:8035/test-run/trigger")
            .body(payload_bytes)
            .header("Content-Type", "application/json")
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

    async fn get_run_status(&self, id: Uuid) -> anyhow::Result<TestRunSummary> {
        let resp = self
            .client
            .get(format!("http://localhost:8035/test-run/{id}/status"))
            .send()
            .await?;

        Ok(resp.json().await?)
    }

    async fn render_run_details(
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

fn any_execution_ongoing(trs: &TestRunSummary) -> bool {
    !trs.current_status.is_terminal()
        || trs
            .executions
            .iter()
            .any(|ex| !ex.current_status.is_terminal())
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
        let message: String = s.status_history[0]
            .message
            .clone()
            .unwrap_or_default()
            .chars()
            .take(MAX_MESSAGE_CHARS)
            .collect();

        Self {
            name: s.name.clone(),
            status: s.status_history[0].status,
            message,
            updated_at: s.updated_at,
            id: format!("(EX-{})", s.id),
        }
    }
}

struct Filter {
    client: Client,
}

impl Filter {
    async fn try_show_summary(
        &self,
        kind: &str,
        s: &str,
        ad_client: &mut AdClient,
    ) -> anyhow::Result<EventOutcome> {
        let id = match Uuid::try_parse(s) {
            Ok(id) => id,
            Err(_) => return Ok(EventOutcome::Passthrough),
        };

        let data: serde_json::Value = self
            .client
            .get(format!("http://localhost:8035/{kind}/{id}/status"))
            .send()
            .await?
            .json()
            .await?;

        let s = serde_json::to_string_pretty(&data)?;
        ad_client
            .open_virtual_in_new_window(format!("+{kind}-summary/{id}.json"), s)
            .await?;

        Ok(EventOutcome::Handled)
    }

    async fn try_show_log(
        &self,
        s: &str,
        ad_client: &mut AdClient,
    ) -> anyhow::Result<EventOutcome> {
        let id = match Uuid::try_parse(s) {
            Ok(id) => id,
            Err(_) => return Ok(EventOutcome::Passthrough),
        };

        let txt = self
            .client
            .get(format!("http://localhost:8035/test-execution/{id}/log.txt"))
            .send()
            .await?
            .text()
            .await?;

        ad_client
            .open_virtual_in_new_window(format!("+test-execution-log/{id}"), txt)
            .await?;

        Ok(EventOutcome::Handled)
    }
}

impl AsyncEventFilter for Filter {
    async fn handle_load(
        &mut self,
        _src: Source,
        _from: usize,
        _to: usize,
        txt: &str,
        client: &mut AdClient,
    ) -> ad_client::Result<EventOutcome> {
        if let Some(s) = txt.strip_prefix("TR-") {
            return self
                .try_show_summary("test-run", s, client)
                .await
                .map_err(|e| ad_client::Error::Rerror {
                    ename: e.to_string(),
                });
        } else if let Some(s) = txt.strip_prefix("EX-") {
            return self
                .try_show_summary("test-execution", s, client)
                .await
                .map_err(|e| ad_client::Error::Rerror {
                    ename: e.to_string(),
                });
        }

        Ok(EventOutcome::Passthrough)
    }

    async fn handle_execute(
        &mut self,
        _src: Source,
        _from: usize,
        _to: usize,
        txt: &str,
        client: &mut AdClient,
    ) -> ad_client::Result<EventOutcome> {
        if let Some(s) = txt.strip_prefix("EX-") {
            return self
                .try_show_log(s, client)
                .await
                .map_err(|e| ad_client::Error::Rerror {
                    ename: e.to_string(),
                });
        }

        Ok(EventOutcome::Passthrough)
    }
}

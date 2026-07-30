use reqwest::Method;
use rtf_integrations::orchestrator::OrchestratorClient;
use std::io::{Write, stdout};

pub async fn execute_remote_request(
    path: &str,
    method: Method,
    data: Option<&str>,
) -> anyhow::Result<()> {
    let client = OrchestratorClient::new().await?;

    let mut req = client.request(method.clone(), path).await?;

    match (data, method) {
        (Some(body), Method::POST) => {
            req = req.json(&serde_json::from_str::<serde_json::Value>(body)?)
        }
        (Some(body), _) => req = req.body(body.to_string()),
        _ => (),
    }

    let resp = req.send().await?;

    if resp.status().is_success() {
        let body = resp.bytes().await?;
        stdout().write_all(&body)?;

        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await?;

        anyhow::bail!("orchestrator returned HTTP {status}: {body}")
    }
}

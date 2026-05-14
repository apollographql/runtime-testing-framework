use reqwest::{
    Body, Method, Request, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use rtf_integrations::orchestrator::{DEFAULT_ORCHESTRATOR_URL, OrchestratorClient};
use std::{
    io::{Write, stdout},
    str::FromStr,
};

pub async fn execute_rep_request(
    path: &str,
    method: &Method,
    data: Option<&str>,
    orchestrator_url: Option<&Url>,
) -> anyhow::Result<()> {
    let endpoint = orchestrator_url
        .unwrap_or(
            &Url::from_str(DEFAULT_ORCHESTRATOR_URL)
                .expect("default orchestrator url should be valid"),
        )
        .join(path)?;

    let mut request = Request::new(method.clone(), endpoint);

    if let Some(body) = data {
        *request.body_mut() = Some(Body::from(body.to_string()));
    }

    if method == Method::POST {
        request
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }

    let client = OrchestratorClient::new().await?;
    let response = client.send(request).await?;

    if response.status().is_success() {
        let body = response.bytes().await?;
        stdout().write_all(&body)?;

        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;

        anyhow::bail!("orchestrator returned HTTP {status}: {body}")
    }
}

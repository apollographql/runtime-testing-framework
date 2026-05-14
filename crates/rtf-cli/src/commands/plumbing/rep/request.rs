use reqwest::{
    Body, Method, Request, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use rtf_integrations::iap::IapClient;
use std::{io::Write, str::FromStr};

pub async fn execute_rep_request(
    path: &str,
    method: &str,
    data: Option<&str>,
    orchestrator_url: &str,
) -> anyhow::Result<()> {
    let method = Method::from_str(method)?;
    let mut request = Request::new(
        method.clone(),
        Url::from_str(&format!("{orchestrator_url}{path}"))?,
    );

    if let Some(body) = data {
        *request.body_mut() = Some(Body::from(body.to_string()));
    }

    if method == Method::POST {
        request
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }

    let client = IapClient::new(request).await?;
    let response = client.send().await?;

    if response.status().is_success() {
        let body = response.bytes().await?;
        std::io::stdout().write_all(&body)?;

        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;

        anyhow::bail!("orchestrator returned HTTP {status}: {body}")
    }
}

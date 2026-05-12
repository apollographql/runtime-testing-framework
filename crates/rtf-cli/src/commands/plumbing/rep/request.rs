//! Handler for `rtf rep request <PATH>`.

use anyhow::Context;
use rtf_integrations::iap::{
    DEFAULT_ORCHESTRATOR_URL, IapClient, ORCHESTRATOR_URL_ENV_VAR, RequestBody,
};
use std::path::PathBuf;

/// Execute `rtf rep request <PATH>`.
///
/// Resolves the orchestrator URL, constructs an [`IapClient`], sends the
/// request, and writes the response body to stdout on success. Exits with a
/// non-zero status on any non-2xx response.
pub async fn execute_rep_request(
    path: &str,
    method: Option<&str>,
    data: Option<&str>,
    headers: &[String],
    orchestrator_url: Option<&str>,
) -> anyhow::Result<()> {
    let env_url = std::env::var(ORCHESTRATOR_URL_ENV_VAR).ok();
    let url = resolve_orchestrator_url(orchestrator_url, env_url.as_deref());

    let body = match data.map(parse_body_source) {
        None => None,
        Some(source) => Some(body_source_to_request_body(source).await?),
    };

    let method = method.unwrap_or("GET");

    let client = IapClient::new(url).await?;
    let response = client.send(method, path, headers, body).await?;

    if response.is_success() {
        use std::io::Write;
        std::io::stdout()
            .write_all(&response.body)
            .context("writing response to stdout")?;

        Ok(())
    } else {
        anyhow::bail!(
            "orchestrator returned HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        )
    }
}

/// Source for the request body supplied via the `-d` flag.
#[derive(Debug, PartialEq)]
enum BodySource {
    /// Read body bytes from the file at the given path (`@<path>`).
    File(PathBuf),
    /// Read body bytes from stdin (`-`).
    Stdin,
    /// Send the string verbatim.
    Literal(String),
}

/// Parse the value of the `-d` flag into a [`BodySource`].
fn parse_body_source(s: &str) -> BodySource {
    if s == "-" {
        BodySource::Stdin
    } else if let Some(path) = s.strip_prefix('@') {
        BodySource::File(PathBuf::from(path))
    } else {
        BodySource::Literal(s.to_owned())
    }
}

/// Resolve the orchestrator base URL from available sources.
///
/// Precedence (highest first):
/// 1. `flag_url` — value of `--orchestrator-url`
/// 2. `env_url` — value of `APOLLO_REP_ORCHESTRATOR_URL`
/// 3. [`DEFAULT_ORCHESTRATOR_URL`]
fn resolve_orchestrator_url(flag_url: Option<&str>, env_url: Option<&str>) -> String {
    flag_url
        .or(env_url)
        .unwrap_or(DEFAULT_ORCHESTRATOR_URL)
        .to_owned()
}

/// Convert a [`BodySource`] into an IAP [`RequestBody`]. Files and stdin are
/// passed as streaming readers so the bytes flow through without being
/// buffered into memory first; literal strings are sent eagerly since they are
/// already in RAM by definition.
async fn body_source_to_request_body(source: BodySource) -> anyhow::Result<RequestBody> {
    match source {
        BodySource::Literal(s) => Ok(RequestBody::from_bytes(s.into_bytes())),
        BodySource::File(path) => {
            let file = tokio::fs::File::open(&path)
                .await
                .with_context(|| format!("opening request body file {}", path.display()))?;
            Ok(RequestBody::from_reader(file))
        }
        BodySource::Stdin => Ok(RequestBody::from_reader(tokio::io::stdin())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_source_at_path() {
        assert_eq!(
            parse_body_source("@/tmp/data.json"),
            BodySource::File(PathBuf::from("/tmp/data.json"))
        );
    }

    #[test]
    fn body_source_stdin() {
        assert_eq!(parse_body_source("-"), BodySource::Stdin);
    }

    #[test]
    fn body_source_literal() {
        assert_eq!(
            parse_body_source("{\"key\":\"value\"}"),
            BodySource::Literal("{\"key\":\"value\"}".to_owned())
        );
    }

    #[test]
    fn body_source_absent_represented_as_none() {
        // Callers pass None when -d is omitted; None.map(parse_body_source) == None.
        let result: Option<BodySource> = None::<&str>.map(parse_body_source);
        assert!(result.is_none());
    }

    #[test]
    fn url_resolution_env_var_only() {
        let result = resolve_orchestrator_url(None, Some("https://env.example.com"));
        assert_eq!(result, "https://env.example.com");
    }

    #[test]
    fn url_resolution_flag_takes_precedence_over_env_var() {
        let result = resolve_orchestrator_url(
            Some("https://flag.example.com"),
            Some("https://env.example.com"),
        );
        assert_eq!(result, "https://flag.example.com");
    }

    #[test]
    fn url_resolution_defaults_to_constant_when_neither_set() {
        let result = resolve_orchestrator_url(None, None);
        assert_eq!(result, DEFAULT_ORCHESTRATOR_URL);
    }
}

//! Handler for `rtf rep request <PATH>`.

use anyhow::Context;
use bytes::Bytes;
use rtf_integrations::iap::{DEFAULT_ORCHESTRATOR_URL, IapClient, ORCHESTRATOR_URL_ENV_VAR};
use std::io::Write;

/// Source for the request body supplied via the `-d` flag.
#[derive(Debug, PartialEq)]
enum BodySource {
    /// Read body bytes from the file at the given path (`@<path>`).
    File(String),
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
        BodySource::File(path.to_owned())
    } else {
        BodySource::Literal(s.to_owned())
    }
}

/// Parse a `"Key: Value"` header string into a `(key, value)` pair.
///
/// The split is performed at the first `": "` sequence; everything after
/// it (including any additional colons) is kept as the value.
///
/// Returns an error if the string contains no `": "` separator.
fn parse_header(s: &str) -> anyhow::Result<(String, String)> {
    s.split_once(": ")
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| anyhow::anyhow!("invalid header {s:?}: expected \"Key: Value\" format"))
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

/// Execute `rtf rep request <PATH>`.
///
/// Resolves the orchestrator URL, parses headers and the request body source,
/// constructs an [`IapClient`], sends the request, and writes the response
/// body to stdout on success. Exits with a non-zero status on any non-2xx
/// response.
pub async fn execute_rep_request(
    path: &str,
    method: Option<&str>,
    data: Option<&str>,
    headers: &[String],
    orchestrator_url: Option<&str>,
) -> anyhow::Result<()> {
    let env_url = std::env::var(ORCHESTRATOR_URL_ENV_VAR).ok();
    let url = resolve_orchestrator_url(orchestrator_url, env_url.as_deref());

    let parsed_headers: Vec<(String, String)> = headers
        .iter()
        .map(|h| parse_header(h))
        .collect::<anyhow::Result<_>>()?;

    let body: Option<Bytes> = match data.map(parse_body_source) {
        None => None,
        Some(BodySource::Literal(s)) => Some(Bytes::from(s.into_bytes())),
        Some(BodySource::File(p)) => {
            let content =
                std::fs::read(&p).with_context(|| format!("reading request body from {p:?}"))?;
            Some(Bytes::from(content))
        }
        Some(BodySource::Stdin) => {
            use std::io::Read;
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .context("reading request body from stdin")?;
            Some(Bytes::from(buf))
        }
    };

    let method = method.unwrap_or("GET");

    let client = IapClient::new(url).await?;

    let response = client.send(method, path, &parsed_headers, body).await?;

    if response.is_success() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_source_at_path() {
        assert_eq!(
            parse_body_source("@/tmp/data.json"),
            BodySource::File("/tmp/data.json".to_owned())
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
    fn parse_header_valid() {
        let (k, v) = parse_header("Content-Type: application/json").unwrap();
        assert_eq!(k, "Content-Type");
        assert_eq!(v, "application/json");
    }

    #[test]
    fn parse_header_value_containing_colon() {
        // Only the *first* ": " is used as the delimiter.
        let (k, v) = parse_header("X-Custom: foo: bar").unwrap();
        assert_eq!(k, "X-Custom");
        assert_eq!(v, "foo: bar");
    }

    #[test]
    fn parse_header_colon_without_space_is_error() {
        assert!(parse_header("Authorization:Bearer token").is_err());
    }

    #[test]
    fn parse_header_no_colon_is_error() {
        assert!(parse_header("BearerToken").is_err());
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

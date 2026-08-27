use crate::orchestrator::{self, Download};
use askama::Template;
use axum::{
    http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, LOCATION},
    response::{Html, IntoResponse, Response},
};
use reqwest::StatusCode;
use rtf_core::variables::VariableOverride;
use std::collections::HashMap;
use tracing::error;

pub mod execution_detail;
pub mod execution_log;
pub mod execution_output_zip;
pub mod index;
pub mod run_status;
pub mod test_plan_detail;
pub mod test_plans;
pub mod trigger;

fn render_body<T: Template>(status: StatusCode, template: T) -> (StatusCode, String) {
    match template.render() {
        Ok(body) => (status, body),
        Err(error) => {
            error!(%error, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, String::new())
        }
    }
}

fn to_response((status, body): (StatusCode, String)) -> Response {
    (status, Html(body)).into_response()
}

type TriggerVariables = HashMap<String, VariableOverride>;

fn parse_trigger_ref_and_variables(
    git_ref: &str,
    variables: &str,
) -> Result<(Option<String>, Option<TriggerVariables>), String> {
    let variables_json = variables.trim();
    let variables = if variables_json.is_empty() {
        None
    } else {
        serde_json::from_str(variables_json)
            .map_err(|error| format!("Variables must be a JSON object: {error}"))?
    };
    let git_ref = git_ref.trim();

    Ok(((!git_ref.is_empty()).then(|| git_ref.to_owned()), variables))
}

#[cfg(test)]
async fn body_text(resp: Response) -> String {
    use axum::body::to_bytes;

    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("response body");

    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

pub async fn health() -> &'static str {
    "ok"
}

fn download_response(
    result: Result<Download, orchestrator::Error>,
    content_type: &'static str,
    filename: String,
) -> Response {
    match result {
        Ok(Download::Ready(bytes)) => (
            [
                (CONTENT_TYPE, content_type.to_owned()),
                (
                    CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(Download::Redirect { status, location }) => {
            (status, [(LOCATION, location)]).into_response()
        }
        Ok(Download::NotFound) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Ok(Download::NotReady) => (StatusCode::CONFLICT, "output not ready yet").into_response(),
        Err(error) => {
            error!(%error, "failed to fetch download from orchestrator");
            (
                StatusCode::BAD_GATEWAY,
                "could not fetch download from orchestrator",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;
    use std::assert_matches;

    #[test]
    fn parse_trigger_ref_and_variables_returns_none_for_blank_fields() {
        let (git_ref, variables) = parse_trigger_ref_and_variables("", "").expect("should parse");

        assert_eq!(git_ref, None);
        assert_eq!(variables, None);
    }

    #[test]
    fn parse_trigger_ref_and_variables_trims_whitespace() {
        let (git_ref, variables) =
            parse_trigger_ref_and_variables("  main  ", "   ").expect("should parse");

        assert_eq!(git_ref, Some("main".to_owned()));
        assert_eq!(variables, None);
    }

    #[test]
    fn parse_trigger_ref_and_variables_parses_a_scalar_and_a_matrix_dimension_variable() {
        let (_, variables) = parse_trigger_ref_and_variables(
            "",
            r#"{"message": "hello", "region": ["us-east-1", "eu-west-1"]}"#,
        )
        .expect("should parse");
        let variables = variables.expect("variables should be present");

        assert_matches!(variables.get("message"), Some(VariableOverride::Scalar(_)));
        assert_matches!(
            variables.get("region"),
            Some(VariableOverride::Array(values)) if values.len() == 2
        );
    }

    #[test]
    fn parse_trigger_ref_and_variables_rejects_invalid_json() {
        let error = parse_trigger_ref_and_variables("", "not json")
            .expect_err("should reject invalid JSON");

        assert!(error.contains("Variables must be a JSON object"), "{error}");
    }

    #[tokio::test]
    async fn health_returns_ok() {
        assert_eq!(health().await, "ok");
    }

    #[tokio::test]
    async fn download_response_returns_bytes_with_a_download_header_on_success() {
        let resp = download_response(
            Ok(Download::Ready(b"hello".to_vec())),
            "text/plain; charset=utf-8",
            "example.txt".to_owned(),
        );

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"example.txt\""
        );
        assert_eq!(body_text(resp).await, "hello");
    }

    #[tokio::test]
    async fn download_response_relays_a_redirect_verbatim() {
        let resp = download_response(
            Ok(Download::Redirect {
                status: StatusCode::TEMPORARY_REDIRECT,
                location: "https://storage.googleapis.com/signed-url".to_owned(),
            }),
            "application/zip",
            "f.zip".to_owned(),
        );

        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            resp.headers().get(LOCATION).unwrap(),
            "https://storage.googleapis.com/signed-url"
        );
        assert_eq!(body_text(resp).await, "");
    }

    #[test_case(Download::NotFound, StatusCode::NOT_FOUND; "not found")]
    #[test_case(Download::NotReady, StatusCode::CONFLICT; "not ready")]
    #[tokio::test]
    async fn download_response_status_cases(download: Download, expected: StatusCode) {
        let resp = download_response(Ok(download), "text/plain", "f".to_owned());

        assert_eq!(resp.status(), expected);
    }

    #[tokio::test]
    async fn download_response_returns_502_when_the_fetch_fails() {
        let resp = download_response(
            Err(orchestrator::Error::Download {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                path: "/test-execution/1/log.txt".to_owned(),
            }),
            "text/plain",
            "f".to_owned(),
        );

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }
}

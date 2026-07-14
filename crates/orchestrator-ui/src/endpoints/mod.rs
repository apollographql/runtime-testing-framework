use crate::templates::IndexTemplate;
use askama::Template;
use axum::response::{Html, IntoResponse, Response};
use reqwest::StatusCode;
use tracing::error;

/// Render an askama template to a `200 OK` HTML response.
pub(crate) fn render<T: Template>(template: T) -> Response {
    render_with_status(template, StatusCode::OK)
}

/// Render an askama template to an HTML response with the given status, mapping render errors to a
/// 500.
pub(crate) fn render_with_status<T: Template>(template: T, status: StatusCode) -> Response {
    match template.render() {
        Ok(body) => (status, Html(body)).into_response(),
        Err(error) => {
            error!(%error, "template render failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /ui` — the run-id input form.
pub async fn index() -> Response {
    render(IndexTemplate)
}

/// `GET /ui/health` — liveness/readiness probe for the standalone service.
pub async fn health() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_text(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("response body");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    #[tokio::test]
    async fn index_renders() {
        let resp = index().await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(
            body.contains("<h2>Orchestrator UI Home</h2>"),
            "expected a placeholder message"
        );
    }

    #[tokio::test]
    async fn health_returns_ok() {
        assert_eq!(health().await, "ok");
    }
}

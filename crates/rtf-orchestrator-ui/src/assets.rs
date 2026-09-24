use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

pub async fn serve(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => (
            [(header::CONTENT_TYPE, file.metadata.mimetype())],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn htmx_is_embedded_in_the_binary() {
        assert!(
            Assets::get("htmx.min.js").is_some(),
            "htmx.min.js should be embedded"
        );
    }

    #[test]
    fn favicon_is_embedded_in_the_binary() {
        assert!(
            Assets::get("gongphin.png").is_some(),
            "gongphin.png should be embedded"
        );
    }

    #[tokio::test]
    async fn serve_returns_the_favicon_with_the_png_content_type() {
        let resp = serve(Path("gongphin.png".to_string())).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()[header::CONTENT_TYPE]
                .to_str()
                .expect("header is valid ascii"),
            "image/png"
        );
    }

    #[tokio::test]
    async fn serve_returns_the_embedded_asset() {
        let resp = serve(Path("htmx.min.js".to_string())).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()[header::CONTENT_TYPE]
                .to_str()
                .expect("header is valid ascii"),
            "text/javascript"
        );
    }

    #[tokio::test]
    async fn serve_404s_for_an_unknown_asset() {
        let resp = serve(Path("does-not-exist.js".to_string())).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}

use askama::Template;

#[derive(Debug, Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub message: String,
}

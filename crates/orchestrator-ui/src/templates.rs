use askama::Template;

/// The landing page: a form to enter a test-run id.
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

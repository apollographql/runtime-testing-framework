use crate::view::ViewerBadge;
use askama::Template;

/// Fragment loaded into the navbar by htmx on page load.
#[derive(Debug, Template)]
#[template(path = "viewer_badge.html")]
pub struct ViewerBadgeTemplate {
    pub badge: Option<ViewerBadge>,
}

use askama::Template;

#[derive(Debug, Default, Template)]
#[template(path = "trigger.html", blocks = ["form"])]
pub struct TriggerTemplate {
    pub org: String,
    pub repo: String,
    pub path: String,
    pub git_ref: String,
    pub variables: String,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::{preview, render_fragment};
    use askama::Template;

    #[test]
    fn form_snapshot() {
        insta::assert_snapshot!(render_fragment!(preview::trigger_empty().as_form()));
    }

    #[test]
    fn form_snapshot_repopulated_with_error() {
        insta::assert_snapshot!(render_fragment!(preview::trigger_error().as_form()));
    }
}

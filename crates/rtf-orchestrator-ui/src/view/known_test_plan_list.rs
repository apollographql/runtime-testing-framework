use rtf_orchestrator_shared::known_test_plan::{KnownTestPlanListResponse, KnownTestPlanSummary};
use url::form_urlencoded;
use uuid::Uuid;

/// One row in the known-test-plans table.
pub struct KnownTestPlanRowView {
    pub uuid: Uuid,
    pub name: String,
    /// Empty when the plan has no description, so the template can render it plainly.
    pub description: String,
    pub org: String,
    pub repo: String,
    pub path: String,
    /// Link to the test plan file on GitHub.
    pub github_url: String,
}

impl From<KnownTestPlanSummary> for KnownTestPlanRowView {
    fn from(plan: KnownTestPlanSummary) -> Self {
        Self {
            github_url: known_test_plan_github_url(&plan.org, &plan.repo, &plan.path),
            uuid: plan.uuid,
            name: plan.name,
            description: plan.description.unwrap_or_default(),
            org: plan.org,
            repo: plan.repo,
            path: plan.path,
        }
    }
}

/// The test plan file's URL on GitHub. Resolved against `blob/HEAD/...` (the repo's default
/// branch) rather than a specific ref, since no ref is persisted on a [`KnownTestPlanSummary`] —
/// only the run it produced (via `known_test_plan_run.git_sha`) records the ref actually used.
fn known_test_plan_github_url(org: &str, repo: &str, path: &str) -> String {
    format!("https://github.com/{org}/{repo}/blob/HEAD/{path}")
}

/// The known-test-plans page's table: the current page of rows plus enough state to render
/// Prev/Next pagination links, mirroring [`super::RunListView`].
pub struct KnownTestPlanListView {
    pub rows: Vec<KnownTestPlanRowView>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

impl KnownTestPlanListView {
    pub fn new(response: KnownTestPlanListResponse, limit: i64, offset: i64) -> Self {
        Self {
            rows: response
                .test_plans
                .into_iter()
                .map(KnownTestPlanRowView::from)
                .collect(),
            total: response.total,
            limit,
            offset,
        }
    }

    pub fn has_prev(&self) -> bool {
        self.offset > 0
    }

    /// See [`super::RunListView::has_next`] for why this is derived from `rows.len()`, not `limit`.
    pub fn has_next(&self) -> bool {
        self.offset + (self.rows.len() as i64) < self.total
    }

    pub fn prev_href(&self) -> String {
        self.href_with_offset(self.prev_offset())
    }

    pub fn next_href(&self) -> String {
        self.href_with_offset(self.offset + self.limit)
    }

    /// See [`super::RunListView::prev_offset`] for why this jumps straight to the last page with
    /// rows on it rather than naively stepping back by `limit`.
    fn prev_offset(&self) -> i64 {
        if self.rows.is_empty() && self.total > 0 {
            ((self.total - 1) / self.limit) * self.limit
        } else {
            (self.offset - self.limit).max(0)
        }
    }

    fn href_with_offset(&self, offset: i64) -> String {
        let mut qs = form_urlencoded::Serializer::new(String::new());
        qs.append_pair("offset", &offset.to_string());
        format!("/ui/test-plans?{}", qs.finish())
    }

    /// e.g. "1-20 of 42". `None` when there's nothing to summarize.
    pub fn showing_range(&self) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        let last = self.offset + self.rows.len() as i64;
        Some(format!("{}-{} of {}", self.offset + 1, last, self.total))
    }

    /// The message shown in the table in place of rows when there's nothing on this page.
    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.total == 0 {
            Some("No known test plans are registered.")
        } else {
            Some("No known test plans on this page.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    /// Builds a `KnownTestPlanListView` with `n_rows` placeholder rows out of `total` matching
    /// plans, at the given `limit`/`offset`.
    fn known_test_plan_list_view(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
    ) -> KnownTestPlanListView {
        let response = KnownTestPlanListResponse {
            test_plans: (0..n_rows)
                .map(|_| KnownTestPlanSummary {
                    uuid: Uuid::new_v4(),
                    name: "a-known-plan".to_owned(),
                    description: None,
                    org: "apollographql".to_owned(),
                    repo: "runtime-testing-framework".to_owned(),
                    path: "test-plans/example.yaml".to_owned(),
                })
                .collect(),
            total,
        };
        KnownTestPlanListView::new(response, limit, offset)
    }

    #[test]
    fn known_test_plan_row_defaults_a_missing_description_to_empty() {
        let row = KnownTestPlanRowView::from(KnownTestPlanSummary {
            uuid: Uuid::new_v4(),
            name: "plan".to_owned(),
            description: None,
            org: "org".to_owned(),
            repo: "repo".to_owned(),
            path: "path.yaml".to_owned(),
        });

        assert_eq!(row.description, "");
    }

    #[test]
    fn known_test_plan_row_builds_a_github_blob_url_against_head() {
        let row = KnownTestPlanRowView::from(KnownTestPlanSummary {
            uuid: Uuid::new_v4(),
            name: "plan".to_owned(),
            description: None,
            org: "apollographql".to_owned(),
            repo: "runtime-testing-framework".to_owned(),
            path: "test-plans/example.yaml".to_owned(),
        });

        assert_eq!(
            row.github_url,
            "https://github.com/apollographql/runtime-testing-framework/blob/HEAD/test-plans/example.yaml"
        );
    }

    #[test_case(20, 42, 20, 0, true, false; "more rows remain and first page")]
    #[test_case(20, 20, 20, 0, false, false; "last full page")]
    #[test_case(0, 15, 20, 40, false, true; "offset landed past the end")]
    #[test]
    fn known_test_plan_list_has_next_and_has_prev(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected_has_next: bool,
        expected_has_prev: bool,
    ) {
        let view = known_test_plan_list_view(n_rows, total, limit, offset);
        assert_eq!(view.has_next(), expected_has_next);
        assert_eq!(view.has_prev(), expected_has_prev);
    }

    #[test_case(20, 137, 20, 40, "/ui/test-plans?offset=20"; "steps back by limit normally")]
    // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
    #[test_case(0, 137, 20, 500, "/ui/test-plans?offset=120"; "jumps to the last real page when offset overshot")]
    #[test]
    fn known_test_plan_list_prev_href_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: &str,
    ) {
        let view = known_test_plan_list_view(n_rows, total, limit, offset);
        assert_eq!(view.prev_href(), expected);
    }

    #[test_case(20, 42, 20, 0, None; "rows are present")]
    #[test_case(0, 0, 20, 0, Some("No known test plans are registered."); "no matches at all")]
    #[test_case(0, 137, 20, 500, Some("No known test plans on this page."); "offset landed past the end")]
    #[test]
    fn known_test_plan_list_empty_message_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: Option<&str>,
    ) {
        assert_eq!(
            known_test_plan_list_view(n_rows, total, limit, offset).empty_message(),
            expected
        );
    }

    #[test_case(0, 0, 20, 0, None; "no matches at all")]
    #[test_case(0, 137, 20, 500, None; "offset landed past the end")]
    #[test_case(17, 137, 20, 120, Some("121-137 of 137"); "formats the current page")]
    #[test]
    fn known_test_plan_list_showing_range_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: Option<&str>,
    ) {
        assert_eq!(
            known_test_plan_list_view(n_rows, total, limit, offset).showing_range(),
            expected.map(str::to_owned)
        );
    }
}

use super::format_rfc3339;
use crate::status;
use rtf_orchestrator_shared::summary::{TestRunListResponse, TestRunSummary};
use url::form_urlencoded;
use uuid::Uuid;

/// One row in the recent-runs table on the home page.
pub struct RunListRowView {
    pub id: Uuid,
    pub name: String,
    pub status_label: String,
    pub status_class: &'static str,
    pub initiated_by: String,
    pub started_at: String,
}

impl From<TestRunSummary> for RunListRowView {
    fn from(run: TestRunSummary) -> Self {
        Self {
            id: run.id,
            name: run.name,
            status_label: run.current_status.to_string(),
            status_class: status::css_class(run.current_status),
            initiated_by: run.initiated_by,
            started_at: format_rfc3339(run.started_at),
        }
    }
}

enum RunListScope {
    Filtered {
        initiated_by: String,
        started_within: String,
    },
    KnownTestPlan(Uuid),
}

/// The recent-runs table shared by the home page and a known test plan's detail page: the current
/// page of rows plus enough state to render Prev/Next pagination links.
pub struct RunListView {
    pub rows: Vec<RunListRowView>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    scope: RunListScope,
}

impl RunListView {
    pub fn new(
        response: TestRunListResponse,
        limit: i64,
        offset: i64,
        initiated_by: String,
        started_within: String,
    ) -> Self {
        Self::build(
            response,
            limit,
            offset,
            RunListScope::Filtered {
                initiated_by,
                started_within,
            },
        )
    }

    pub fn for_known_test_plan(
        response: TestRunListResponse,
        limit: i64,
        offset: i64,
        plan_uuid: Uuid,
    ) -> Self {
        Self::build(
            response,
            limit,
            offset,
            RunListScope::KnownTestPlan(plan_uuid),
        )
    }

    fn build(response: TestRunListResponse, limit: i64, offset: i64, scope: RunListScope) -> Self {
        Self {
            rows: response
                .runs
                .into_iter()
                .map(RunListRowView::from)
                .collect(),
            total: response.total,
            limit,
            offset,
            scope,
        }
    }

    pub fn has_prev(&self) -> bool {
        self.offset > 0
    }

    pub fn has_next(&self) -> bool {
        self.offset + (self.rows.len() as i64) < self.total
    }

    pub fn prev_href(&self) -> String {
        self.href_with_offset(self.prev_offset())
    }

    pub fn next_href(&self) -> String {
        self.href_with_offset(self.offset + self.limit)
    }

    fn prev_offset(&self) -> i64 {
        if self.rows.is_empty() && self.total > 0 {
            ((self.total - 1) / self.limit) * self.limit
        } else {
            (self.offset - self.limit).max(0)
        }
    }

    fn href_with_offset(&self, offset: i64) -> String {
        match &self.scope {
            RunListScope::KnownTestPlan(plan_uuid) => {
                format!("/ui/test-plan/{plan_uuid}?offset={offset}")
            }
            RunListScope::Filtered {
                initiated_by,
                started_within,
            } => {
                let mut qs = form_urlencoded::Serializer::new(String::new());
                if !initiated_by.is_empty() {
                    qs.append_pair("initiated_by", initiated_by);
                }
                if !started_within.is_empty() {
                    qs.append_pair("started_within", started_within);
                }
                qs.append_pair("offset", &offset.to_string());
                format!("/ui?{}", qs.finish())
            }
        }
    }

    /// e.g. "1-20 of 137". `None` when there's nothing to summarize — no matching runs at all, or
    /// `offset` landed past the last page — so the template shows `empty_message` instead rather
    /// than both a range and a "no runs" row at once.
    pub fn showing_range(&self) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        let last = self.offset + self.rows.len() as i64;
        Some(format!("{}-{} of {}", self.offset + 1, last, self.total))
    }

    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.total == 0 {
            Some(match self.scope {
                RunListScope::KnownTestPlan(_) => "This test plan has no runs yet.",
                RunListScope::Filtered { .. } => "No runs match these filters.",
            })
        } else {
            Some("No runs on this page.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    /// Builds a `RunListView` with `n_rows` placeholder rows out of `total` matching runs, at the
    /// given `limit`/`offset`.
    fn list_view(n_rows: usize, total: i64, limit: i64, offset: i64) -> RunListView {
        list_view_with_filters(n_rows, total, limit, offset, "", "")
    }

    fn list_view_with_filters(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        initiated_by: &str,
        started_within: &str,
    ) -> RunListView {
        let response = TestRunListResponse {
            runs: vec![TestRunSummary::default(); n_rows],
            total,
        };
        RunListView::new(
            response,
            limit,
            offset,
            initiated_by.to_owned(),
            started_within.to_owned(),
        )
    }

    #[test_case(20, 137, 20, 0, true, false; "more rows remain and first page")]
    #[test_case(20, 20, 20, 0, false, false; "last full page")]
    #[test_case(0, 15, 20, 40, false, true; "offset landed past the end")]
    #[test]
    fn has_next_and_has_prev(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected_has_next: bool,
        expected_has_prev: bool,
    ) {
        let view = list_view(n_rows, total, limit, offset);
        assert_eq!(view.has_next(), expected_has_next);
        assert_eq!(view.has_prev(), expected_has_prev);
    }

    #[test_case(20, 137, 20, 40, "/ui?offset=20"; "steps back by limit normally")]
    // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
    #[test_case(0, 137, 20, 500, "/ui?offset=120"; "jumps to the last real page when offset overshot")]
    #[test]
    fn prev_href_cases(n_rows: usize, total: i64, limit: i64, offset: i64, expected: &str) {
        let view = list_view(n_rows, total, limit, offset);
        assert_eq!(view.prev_href(), expected);
    }

    #[test_case(20, 137, 20, 0, None; "rows are present")]
    #[test_case(0, 0, 20, 0, Some("No runs match these filters."); "no matches at all")]
    #[test_case(0, 137, 20, 500, Some("No runs on this page."); "offset landed past the end")]
    #[test]
    fn empty_message_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: Option<&str>,
    ) {
        assert_eq!(
            list_view(n_rows, total, limit, offset).empty_message(),
            expected
        );
    }

    #[test_case(0, 0, 20, 0, None; "no matches at all")]
    #[test_case(0, 137, 20, 500, None; "offset landed past the end")]
    #[test_case(17, 137, 20, 120, Some("121-137 of 137"); "formats the current page")]
    #[test]
    fn showing_range_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: Option<&str>,
    ) {
        assert_eq!(
            list_view(n_rows, total, limit, offset).showing_range(),
            expected.map(str::to_owned)
        );
    }

    #[test]
    fn hrefs_percent_encode_special_characters_in_filters() {
        let view = list_view_with_filters(20, 137, 20, 40, "a&b#c d", "day");
        let href = view.prev_href();

        assert!(
            href.contains("initiated_by=a%26b%23c+d"),
            "expected percent-encoded initiated_by, got {href}"
        );
        assert!(href.contains("started_within=day"));
    }

    /// Builds a `RunListView` scoped to an arbitrary known test plan (via
    /// [`RunListView::for_known_test_plan`]), with `n_rows` placeholder rows out of `total`
    /// matching runs, at the given `limit`/`offset`.
    fn run_list_view_for_known_test_plan(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
    ) -> RunListView {
        let response = TestRunListResponse {
            runs: vec![TestRunSummary::default(); n_rows],
            total,
        };
        RunListView::for_known_test_plan(response, limit, offset, Uuid::from_u128(1))
    }

    #[test_case(20, 137, 20, 0, true, false; "more rows remain and first page")]
    #[test_case(20, 20, 20, 0, false, false; "last full page")]
    #[test_case(0, 15, 20, 40, false, true; "offset landed past the end")]
    #[test]
    fn run_list_for_known_test_plan_has_next_and_has_prev(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected_has_next: bool,
        expected_has_prev: bool,
    ) {
        let view = run_list_view_for_known_test_plan(n_rows, total, limit, offset);
        assert_eq!(view.has_next(), expected_has_next);
        assert_eq!(view.has_prev(), expected_has_prev);
    }

    #[test_case(20, 137, 20, 40, 20; "steps back by limit normally")]
    // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
    #[test_case(0, 137, 20, 500, 120; "jumps to the last real page when offset overshot")]
    #[test]
    fn run_list_for_known_test_plan_prev_href_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected_offset: i64,
    ) {
        let view = run_list_view_for_known_test_plan(n_rows, total, limit, offset);
        assert_eq!(
            view.prev_href(),
            format!(
                "/ui/test-plan/{}?offset={expected_offset}",
                Uuid::from_u128(1)
            )
        );
    }

    #[test]
    fn run_list_for_known_test_plan_next_href_is_scoped_to_the_plan() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 0);
        assert_eq!(
            view.next_href(),
            format!("/ui/test-plan/{}?offset=20", Uuid::from_u128(1))
        );
    }

    #[test_case(0, 0, 20, 0, Some("This test plan has no runs yet."); "no runs at all")]
    #[test_case(0, 137, 20, 500, Some("No runs on this page."); "offset landed past the end")]
    #[test]
    fn run_list_for_known_test_plan_empty_message_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: Option<&str>,
    ) {
        assert_eq!(
            run_list_view_for_known_test_plan(n_rows, total, limit, offset).empty_message(),
            expected
        );
    }

    #[test]
    fn run_list_for_known_test_plan_showing_range_formats_the_current_page() {
        let view = run_list_view_for_known_test_plan(17, 137, 20, 120);
        assert_eq!(view.showing_range(), Some("121-137 of 137".to_owned()));
    }
}

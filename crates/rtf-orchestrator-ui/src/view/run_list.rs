use crate::view::{Pagination, StatusView, format_initiator, format_rfc3339};
use rtf_orchestrator_shared::summary::{TestRunListResponse, TestRunSummary};
use url::form_urlencoded;
use uuid::Uuid;

#[derive(Debug)]
pub struct RunListRowView {
    pub id: Uuid,
    pub name: String,
    pub status: StatusView,
    pub initiated_by: String,
    pub started_at: String,
}

impl From<TestRunSummary> for RunListRowView {
    fn from(run: TestRunSummary) -> Self {
        Self {
            id: run.id,
            name: run.name,
            status: run.current_status.into(),
            initiated_by: format_initiator(run.initiated_by),
            started_at: format_rfc3339(run.started_at),
        }
    }
}

#[derive(Debug)]
enum RunListScope {
    Filtered {
        initiated_by: String,
        started_within: String,
    },
    KnownTestPlan(Uuid),
}

#[derive(Debug)]
pub struct RunListView {
    pub rows: Vec<RunListRowView>,
    pagination: Pagination,
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
        let rows: Vec<RunListRowView> = response
            .runs
            .into_iter()
            .map(RunListRowView::from)
            .collect();
        let pagination = Pagination {
            total: response.total,
            limit,
            offset,
            n_rows: rows.len(),
        };

        Self {
            rows,
            pagination,
            scope,
        }
    }

    pub fn has_prev(&self) -> bool {
        self.pagination.has_prev()
    }

    pub fn has_next(&self) -> bool {
        self.pagination.has_next()
    }

    pub fn prev_href(&self) -> String {
        self.href_with_offset(self.pagination.prev_offset())
    }

    pub fn next_href(&self) -> String {
        self.href_with_offset(self.pagination.next_offset())
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

    pub fn showing_range(&self) -> Option<String> {
        self.pagination.showing_range()
    }

    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.pagination.total == 0 {
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

    #[test]
    fn has_next_and_has_prev_delegate_to_pagination() {
        let view = list_view(20, 137, 20, 40);
        assert!(view.has_next());
        assert!(view.has_prev());
    }

    #[test]
    fn prev_href_incorporates_the_paged_back_offset() {
        let view = list_view(20, 137, 20, 40);
        assert_eq!(view.prev_href(), "/ui?offset=20");
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

    #[test]
    fn showing_range_delegates_to_pagination() {
        let view = list_view(17, 137, 20, 120);
        assert_eq!(view.showing_range(), Some("121-137 of 137".to_owned()));
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

    #[test]
    fn run_list_for_known_test_plan_has_next_and_has_prev_delegate_to_pagination() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 40);
        assert!(view.has_next());
        assert!(view.has_prev());
    }

    #[test]
    fn run_list_for_known_test_plan_prev_href_is_scoped_to_the_plan() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 40);
        assert_eq!(
            view.prev_href(),
            format!("/ui/test-plan/{}?offset=20", Uuid::from_u128(1))
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

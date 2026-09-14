use crate::view::Pagination;
use rtf_orchestrator_shared::known_test_plan::{KnownTestPlanListResponse, KnownTestPlanSummary};
use url::form_urlencoded;
use uuid::Uuid;

pub struct KnownTestPlanRowView {
    pub uuid: Uuid,
    pub name: String,
    pub description: String,
    pub org: String,
    pub repo: String,
    pub path: String,
    pub github_url: String,
    pub allow_k8s_write: bool,
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
            allow_k8s_write: plan.allow_k8s_write,
        }
    }
}

fn known_test_plan_github_url(org: &str, repo: &str, path: &str) -> String {
    format!("https://github.com/{org}/{repo}/blob/HEAD/{path}")
}

pub struct KnownTestPlanListView {
    pub rows: Vec<KnownTestPlanRowView>,
    pagination: Pagination,
}

impl KnownTestPlanListView {
    pub fn new(response: KnownTestPlanListResponse, limit: i64, offset: i64) -> Self {
        let rows: Vec<KnownTestPlanRowView> = response
            .test_plans
            .into_iter()
            .map(KnownTestPlanRowView::from)
            .collect();
        let pagination = Pagination {
            total: response.total,
            limit,
            offset,
            n_rows: rows.len(),
        };

        Self { rows, pagination }
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
        let mut qs = form_urlencoded::Serializer::new(String::new());
        qs.append_pair("offset", &offset.to_string());
        format!("/ui/test-plans?{}", qs.finish())
    }

    pub fn showing_range(&self) -> Option<String> {
        self.pagination.showing_range()
    }

    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.pagination.total == 0 {
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
                    pinned_workload_cluster: None,
                    allow_k8s_write: false,
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
            pinned_workload_cluster: None,
            allow_k8s_write: false,
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
            pinned_workload_cluster: None,
            allow_k8s_write: false,
        });

        assert_eq!(
            row.github_url,
            "https://github.com/apollographql/runtime-testing-framework/blob/HEAD/test-plans/example.yaml"
        );
    }

    #[test]
    fn known_test_plan_list_has_next_and_has_prev_delegate_to_pagination() {
        let view = known_test_plan_list_view(20, 137, 20, 40);
        assert!(view.has_next());
        assert!(view.has_prev());
    }

    #[test]
    fn known_test_plan_list_prev_href_incorporates_the_paged_back_offset() {
        let view = known_test_plan_list_view(20, 137, 20, 40);
        assert_eq!(view.prev_href(), "/ui/test-plans?offset=20");
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

    #[test]
    fn known_test_plan_list_showing_range_delegates_to_pagination() {
        let view = known_test_plan_list_view(17, 137, 20, 120);
        assert_eq!(view.showing_range(), Some("121-137 of 137".to_owned()));
    }
}

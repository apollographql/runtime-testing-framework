use chrono::{DateTime, Utc};
use rtf_orchestrator_shared::{
    test_plan::{EnvironmentService, ServiceReplicas},
    test_plan_details::{
        ConfigSection, TestPlanDetails, TestPlanHistory, TestPlanVariable, VariableValue,
    },
};
use serde::Serialize;
use url::form_urlencoded;
use uuid::Uuid;

fn format_day(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d").to_string()
}

fn current_value_list(v: &TestPlanVariable) -> Vec<String> {
    match &v.current_value {
        Some(VariableValue::Dimension(values)) => values.iter().map(ToString::to_string).collect(),
        Some(VariableValue::Scalar(s)) => vec![s.to_string()],
        None => v
            .declarations
            .iter()
            .find_map(|d| d.default.as_ref())
            .map(|d| vec![d.to_string()])
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerVariableView {
    pub name: String,
    pub sections: Vec<&'static str>,
    pub description: String,
    pub ghost_text: String,
    pub full_values: Vec<String>,
    pub full_values_multiline: String,
    pub allowed_values: Option<Vec<String>>,
}

impl TriggerVariableView {
    fn new(v: &TestPlanVariable) -> Self {
        let mut sections: Vec<&'static str> = v
            .declarations
            .iter()
            .map(|d| match d.section {
                ConfigSection::Scenario => "scenario",
                ConfigSection::Environment => "environment",
            })
            .collect();
        sections.dedup();

        let full_values = current_value_list(v);
        let allowed_values = v
            .declarations
            .iter()
            .find_map(|d| d.allowed_values.as_ref())
            .map(|values| values.iter().map(ToString::to_string).collect());

        Self {
            name: v.name.clone(),
            sections,
            description: v
                .declarations
                .first()
                .map(|d| d.description.clone())
                .unwrap_or_default(),
            ghost_text: full_values.join(", "),
            full_values_multiline: full_values.join("\n"),
            full_values,
            allowed_values,
        }
    }

    pub fn is_selected(&self, value: &str) -> bool {
        self.full_values.iter().any(|v| v == value)
    }
}

#[derive(Debug, Clone)]
pub struct ServiceRowView {
    pub name: String,
    pub image: String,
    pub replicas_label: String,
}

impl ServiceRowView {
    fn new(svc: &EnvironmentService) -> Self {
        Self {
            name: svc.name.clone(),
            image: svc.image.clone().unwrap_or_else(|| "—".to_owned()),
            replicas_label: match &svc.replicas {
                ServiceReplicas::Fixed(n) => n.to_string(),
                ServiceReplicas::Variable(token) => token.clone(),
            },
        }
    }
}

#[derive(Serialize)]
pub struct TestPlanDetailsJsData<'a> {
    pub variables: &'a [TriggerVariableView],
    pub n_include_groups: usize,
    pub history: &'a TestPlanHistory,
}

pub struct TestPlanDetailsView {
    pub uuid: Uuid,
    pub github_url: String,
    pub source_sha: String,
    pub variables: Vec<TriggerVariableView>,
    pub n_executions: usize,
    pub n_services: usize,
    pub matrix_formula: String,
    pub fixed_groups: Vec<Vec<(String, String)>>,
    pub services: Vec<ServiceRowView>,
    pub services_vary_by_matrix: bool,
    pub n_containers_per_execution: Option<usize>,
    pub history_from_label: String,
    pub history_to_label: String,
    pub sampled_runs: u64,
    days_back: u32,
    days: u32,
    history: TestPlanHistory,
}

impl TestPlanDetailsView {
    pub fn new(details: TestPlanDetails, days_back: u32, days: u32) -> Self {
        let source = details.source;
        let matrix = details.matrix;
        let environment = details.environment;
        let history = details.history;

        let github_url = format!(
            "https://github.com/{}/{}/blob/{}/{}",
            source.org, source.repo, source.sha, source.path
        );

        let n_containers_per_execution = (!environment.has_variable_replicas).then(|| {
            environment
                .services
                .iter()
                .map(|s| match s.replicas {
                    ServiceReplicas::Fixed(n) => n as usize,
                    ServiceReplicas::Variable(_) => 0,
                })
                .sum()
        });

        let mut matrix_formula_parts: Vec<String> = matrix
            .dimensions
            .iter()
            .map(|(name, values)| format!("{} {name}", values.len()))
            .collect();
        if !matrix.include_groups.is_empty() {
            let n = matrix.include_groups.len();
            matrix_formula_parts.push(format!(
                "{n} include group{}",
                if n == 1 { "" } else { "s" }
            ));
        }

        Self {
            uuid: details.uuid,
            github_url,
            source_sha: source.sha,
            n_executions: matrix.n_executions,
            n_services: environment.services.len(),
            matrix_formula: matrix_formula_parts.join(" × "),
            fixed_groups: matrix
                .include_groups
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .map(|(k, v)| (k.clone(), v.to_string()))
                        .collect()
                })
                .collect(),
            services: environment
                .services
                .iter()
                .map(ServiceRowView::new)
                .collect(),
            services_vary_by_matrix: environment.services_vary_by_matrix,
            n_containers_per_execution,
            history_from_label: format_day(history.window.from),
            history_to_label: format_day(history.window.to),
            sampled_runs: history.execution_durations.total,
            variables: details
                .variables
                .iter()
                .map(TriggerVariableView::new)
                .collect(),
            days_back,
            days,
            history,
        }
    }

    pub fn js_data(&self) -> TestPlanDetailsJsData<'_> {
        TestPlanDetailsJsData {
            variables: &self.variables,
            n_include_groups: self.fixed_groups.len(),
            history: &self.history,
        }
    }

    pub fn prev_history_href(&self, current_ref: &str) -> String {
        self.history_href_with_days_back(current_ref, self.days_back + self.days)
    }

    pub fn next_history_href(&self, current_ref: &str) -> String {
        self.history_href_with_days_back(current_ref, self.days_back.saturating_sub(self.days))
    }

    pub fn has_next_history(&self) -> bool {
        self.days_back > 0
    }

    fn history_href_with_days_back(&self, current_ref: &str, days_back: u32) -> String {
        let mut qs = form_urlencoded::Serializer::new(String::new());
        if !current_ref.is_empty() {
            qs.append_pair("trigger_ref", current_ref);
        }
        qs.append_pair("days_back", &days_back.to_string());
        format!("/ui/test-plan/{}?{}", self.uuid, qs.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtf_config::templating::Scalar;
    use rtf_orchestrator_shared::test_plan_details::{
        DEFAULT_DAYS, DEFAULT_DAYS_BACK, EnvironmentSummary, MatrixSummary, TestPlanSource,
        VariableDeclaration,
    };
    use simple_test_case::test_case;
    use std::collections::BTreeMap;

    fn sample_variable(
        name: &str,
        current_value: Option<VariableValue>,
        allowed_values: Option<Vec<Scalar>>,
    ) -> TestPlanVariable {
        TestPlanVariable {
            name: name.to_owned(),
            declarations: vec![VariableDeclaration {
                section: ConfigSection::Scenario,
                description: "a description".to_owned(),
                default: None,
                allowed_values,
            }],
            current_value,
            required: false,
        }
    }

    #[test]
    fn trigger_variable_view_ghost_text_for_a_scalar() {
        let v = sample_variable(
            "message",
            Some(VariableValue::Scalar(Scalar::from("hello"))),
            None,
        );
        let view = TriggerVariableView::new(&v);

        assert_eq!(view.ghost_text, "hello");
        assert_eq!(view.full_values, vec!["hello".to_owned()]);
        assert_eq!(view.full_values_multiline, "hello");
        assert_eq!(view.allowed_values, None);
    }

    #[test]
    fn trigger_variable_view_ghost_text_for_a_dimension() {
        let v = sample_variable(
            "region",
            Some(VariableValue::Dimension(vec![
                Scalar::from("us-east-1"),
                Scalar::from("eu-west-1"),
            ])),
            None,
        );
        let view = TriggerVariableView::new(&v);

        assert_eq!(view.ghost_text, "us-east-1, eu-west-1");
        assert_eq!(view.full_values_multiline, "us-east-1\neu-west-1");
    }

    #[test]
    fn trigger_variable_view_falls_back_to_the_first_default_when_the_plan_sets_no_value() {
        let mut v = sample_variable("duration", None, None);
        v.declarations[0].default = Some(Scalar::from("5m"));

        let view = TriggerVariableView::new(&v);

        assert_eq!(view.ghost_text, "5m");
    }

    #[test]
    fn trigger_variable_view_has_no_ghost_text_when_nothing_is_set_at_all() {
        let v = sample_variable("needs_a_value", None, None);
        let view = TriggerVariableView::new(&v);

        assert_eq!(view.ghost_text, "");
        assert!(view.full_values.is_empty());
    }

    #[test]
    fn trigger_variable_view_surfaces_allowed_values_as_strings() {
        let v = sample_variable(
            "tier",
            None,
            Some(vec![Scalar::from("free"), Scalar::from("paid")]),
        );
        let view = TriggerVariableView::new(&v);

        assert_eq!(
            view.allowed_values,
            Some(vec!["free".to_owned(), "paid".to_owned()])
        );
    }

    #[test]
    fn trigger_variable_view_dedupes_sections_declared_more_than_once() {
        let v = TestPlanVariable {
            name: "shared".to_owned(),
            declarations: vec![
                VariableDeclaration {
                    section: ConfigSection::Scenario,
                    description: "a".to_owned(),
                    default: None,
                    allowed_values: None,
                },
                VariableDeclaration {
                    section: ConfigSection::Environment,
                    description: "b".to_owned(),
                    default: None,
                    allowed_values: None,
                },
            ],
            current_value: None,
            required: false,
        };
        let view = TriggerVariableView::new(&v);

        assert_eq!(view.sections, vec!["scenario", "environment"]);
        assert_eq!(view.description, "a");
    }

    #[test]
    fn trigger_variable_view_is_selected_matches_current_values_only() {
        let v = sample_variable(
            "tier",
            Some(VariableValue::Scalar(Scalar::from("paid"))),
            Some(vec![
                Scalar::from("free"),
                Scalar::from("paid"),
                Scalar::from("enterprise"),
            ]),
        );
        let view = TriggerVariableView::new(&v);

        assert!(view.is_selected("paid"));
        assert!(!view.is_selected("free"));
    }

    #[test]
    fn service_row_view_shows_a_fixed_replica_count() {
        let svc = EnvironmentService {
            name: "web".to_owned(),
            image: Some("nginx:1.25".to_owned()),
            replicas: ServiceReplicas::Fixed(3),
        };
        let row = ServiceRowView::new(&svc);

        assert_eq!(row.image, "nginx:1.25");
        assert_eq!(row.replicas_label, "3");
    }

    #[test]
    fn service_row_view_shows_the_raw_token_for_a_variable_replica_count() {
        let svc = EnvironmentService {
            name: "subgraph".to_owned(),
            image: None,
            replicas: ServiceReplicas::Variable("${SUBGRAPH_REPLICAS}".to_owned()),
        };
        let row = ServiceRowView::new(&svc);

        assert_eq!(row.image, "—");
        assert_eq!(row.replicas_label, "${SUBGRAPH_REPLICAS}");
    }

    fn sample_details(uuid: Uuid) -> TestPlanDetails {
        TestPlanDetails {
            uuid,
            name: "plan".to_owned(),
            description: None,
            source: TestPlanSource {
                org: "org".to_owned(),
                repo: "repo".to_owned(),
                path: "path.yaml".to_owned(),
                git_ref: None,
                sha: "abcdef1234567890".to_owned(),
            },
            variables: Vec::new(),
            matrix: MatrixSummary {
                n_executions: 1,
                dimensions: BTreeMap::new(),
                include_groups: Vec::new(),
            },
            environment: EnvironmentSummary {
                services: Vec::new(),
                has_variable_replicas: false,
                services_vary_by_matrix: false,
                resolved_for_variant: None,
            },
            history: TestPlanHistory::default(),
        }
    }

    #[test]
    fn test_plan_details_view_formula_omits_include_groups_when_there_are_none() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.matrix.dimensions.insert(
            "region".to_owned(),
            vec![Scalar::from("a"), Scalar::from("b")],
        );

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(view.matrix_formula, "2 region");
    }

    #[test]
    fn test_plan_details_view_formula_includes_a_nonempty_include_group_count() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.matrix.dimensions.insert(
            "region".to_owned(),
            vec![Scalar::from("a"), Scalar::from("b")],
        );
        details.matrix.include_groups = vec![BTreeMap::new(), BTreeMap::new(), BTreeMap::new()];

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(view.matrix_formula, "2 region × 3 include groups");
    }

    #[test]
    fn test_plan_details_view_sums_containers_only_when_replicas_are_all_fixed() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.environment.services = vec![
            EnvironmentService {
                name: "a".to_owned(),
                image: None,
                replicas: ServiceReplicas::Fixed(1),
            },
            EnvironmentService {
                name: "b".to_owned(),
                image: None,
                replicas: ServiceReplicas::Fixed(2),
            },
        ];
        details.environment.has_variable_replicas = false;

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(view.n_containers_per_execution, Some(3));
    }

    #[test]
    fn test_plan_details_view_omits_the_container_total_when_any_replica_count_is_unresolved() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.environment.services = vec![
            EnvironmentService {
                name: "a".to_owned(),
                image: None,
                replicas: ServiceReplicas::Fixed(1),
            },
            EnvironmentService {
                name: "b".to_owned(),
                image: None,
                replicas: ServiceReplicas::Variable("${N}".to_owned()),
            },
        ];
        details.environment.has_variable_replicas = true;

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(view.n_containers_per_execution, None);
    }

    #[test_case(0, 30, "days_back=30"; "first page pages back by the window size")]
    #[test_case(30, 30, "days_back=60"; "an earlier page keeps paging back")]
    #[test]
    fn test_plan_details_view_prev_history_href_pages_back_by_the_window_size(
        days_back: u32,
        days: u32,
        expected_days_back: &str,
    ) {
        let view = TestPlanDetailsView::new(sample_details(Uuid::from_u128(1)), days_back, days);

        assert!(view.prev_history_href("").contains(expected_days_back));
    }

    #[test]
    fn test_plan_details_view_next_history_href_pages_forward_by_the_window_size() {
        let view = TestPlanDetailsView::new(sample_details(Uuid::from_u128(1)), 60, 30);

        assert!(view.next_history_href("").contains("days_back=30"));
    }

    #[test_case(0, false; "no earlier days_back means no more-recent window to page to")]
    #[test_case(30, true; "a nonzero days_back means a more-recent window exists")]
    #[test]
    fn test_plan_details_view_has_next_history_cases(days_back: u32, expected: bool) {
        let view = TestPlanDetailsView::new(sample_details(Uuid::from_u128(1)), days_back, 30);

        assert_eq!(view.has_next_history(), expected);
    }

    #[test]
    fn test_plan_details_view_history_hrefs_carry_the_current_ref() {
        let view = TestPlanDetailsView::new(sample_details(Uuid::from_u128(1)), 30, 30);

        assert!(
            view.prev_history_href("a branch")
                .contains("trigger_ref=a+branch")
        );
    }
}

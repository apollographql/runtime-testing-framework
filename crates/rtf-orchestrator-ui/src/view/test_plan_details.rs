use chrono::{DateTime, Duration, Utc};
use rtf_config::{
    formats::{EnvironmentService, ServiceReplicas},
    templating::Scalar,
};
use rtf_orchestrator_shared::test_plan_details::{
    ConfigSection, EnvironmentSummary, ManifestLocation, TestPlanDetails, TestPlanHistory,
    TestPlanVariable, VariableValue,
};
use serde::Serialize;
use std::collections::BTreeMap;
use url::form_urlencoded;
use uuid::Uuid;

fn format_day(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d").to_string()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn current_values(v: &TestPlanVariable) -> Vec<Scalar> {
    match &v.current_value {
        Some(VariableValue::Dimension(values)) => values.clone(),
        Some(VariableValue::Scalar(s)) => vec![s.clone()],
        None => v
            .declarations
            .iter()
            .find_map(|d| d.default.clone())
            .into_iter()
            .collect(),
    }
}

/// Serialized into the page for `test-plan-detail.js`.
#[derive(Debug, Clone, Serialize)]
pub struct TriggerVariableView {
    pub name: String,
    pub sections: Vec<&'static str>,
    pub description: String,
    pub default: Vec<Scalar>,
    pub allowed: Option<Vec<Scalar>>,
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

        Self {
            name: v.name.clone(),
            sections,
            description: v
                .declarations
                .first()
                .map(|d| d.description.clone())
                .unwrap_or_default(),
            default: current_values(v),
            allowed: v.declarations.iter().find_map(|d| d.allowed_values.clone()),
        }
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

#[derive(Debug, Clone)]
pub enum EnvironmentDetailView {
    DockerCompose(ComposeDetailView),
    K8s(K8sDetailView),
}

#[derive(Debug, Clone)]
pub struct ComposeDetailView {
    pub n_services: usize,
    pub services: Vec<ServiceRowView>,
    pub services_vary_by_matrix: bool,
    pub n_containers_per_execution: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct K8sDetailView {
    pub manifests: Vec<ManifestLocation>,
}

impl EnvironmentDetailView {
    fn new(environment: EnvironmentSummary) -> Self {
        match environment {
            EnvironmentSummary::DockerCompose(compose) => {
                let n_containers_per_execution = (!compose.has_variable_replicas).then(|| {
                    compose
                        .services
                        .iter()
                        .map(|s| match s.replicas {
                            ServiceReplicas::Fixed(n) => n as usize,
                            ServiceReplicas::Variable(_) => 0,
                        })
                        .sum()
                });

                Self::DockerCompose(ComposeDetailView {
                    n_services: compose.services.len(),
                    services: compose.services.iter().map(ServiceRowView::new).collect(),
                    services_vary_by_matrix: compose.services_vary_by_matrix,
                    n_containers_per_execution,
                })
            }

            EnvironmentSummary::K8s(k8s) => Self::K8s(K8sDetailView {
                manifests: k8s.manifests,
            }),
        }
    }
}

pub type CompoundGroups = BTreeMap<String, Vec<BTreeMap<String, Scalar>>>;

#[derive(Debug, Serialize)]
pub struct TestPlanDetailsJsData<'a> {
    pub variables: &'a [TriggerVariableView],
    pub compound_groups: &'a CompoundGroups,
    pub history: &'a TestPlanHistory,
}

#[derive(Debug)]
pub struct TestPlanDetailsView {
    pub uuid: Uuid,
    pub github_url: String,
    pub source_sha: String,
    pub pool: String,
    pub variables: Vec<TriggerVariableView>,
    pub n_executions: usize,
    pub matrix_formula: String,
    pub variables_summary: String,
    pub compound_groups: CompoundGroups,
    pub environment: Option<EnvironmentDetailView>,
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

        let mut matrix_formula_parts: Vec<String> = matrix
            .dimensions
            .iter()
            .map(|(name, values)| format!("{} {name}", values.len()))
            .collect();
        matrix_formula_parts.extend(
            matrix
                .compound_groups
                .iter()
                .map(|(name, entries)| format!("{} {name}", entries.len())),
        );

        let n_compound_members: usize = matrix
            .compound_groups
            .values()
            .map(|entries| entries.first().map_or(0, BTreeMap::len))
            .sum();
        let variables_summary = [
            plural(
                details.variables.len() + n_compound_members,
                "variable",
                "variables",
            ),
            plural(
                matrix.dimensions.len(),
                "matrix dimension",
                "matrix dimensions",
            ),
            plural(
                matrix.compound_groups.len(),
                "compound dimension",
                "compound dimensions",
            ),
        ]
        .join(" · ");

        Self {
            uuid: details.uuid,
            github_url,
            source_sha: source.sha,
            pool: details.pool,
            n_executions: matrix.n_executions,
            matrix_formula: matrix_formula_parts.join(" × "),
            variables_summary,
            compound_groups: matrix.compound_groups,
            environment: environment.map(EnvironmentDetailView::new),
            history_from_label: format_day(history.window.from),
            history_to_label: format_day(history.window.to - Duration::days(1)),
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
            compound_groups: &self.compound_groups,
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
    use chrono::TimeZone;
    use rtf_config::templating::Scalar;
    use rtf_orchestrator_shared::test_plan_details::{
        ComposeEnvironmentSummary, DEFAULT_DAYS, DEFAULT_DAYS_BACK, EnvironmentSummary,
        HistoryWindow, K8sEnvironmentSummary, ManifestLocation, MatrixSummary, TestPlanSource,
        VariableDeclaration,
    };
    use simple_test_case::test_case;
    use std::collections::BTreeMap;

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

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
    fn trigger_variable_view_default_for_a_scalar() {
        let v = sample_variable(
            "message",
            Some(VariableValue::Scalar(Scalar::from("hello"))),
            None,
        );
        let view = TriggerVariableView::new(&v);

        assert_eq!(view.default, vec![Scalar::from("hello")]);
        assert_eq!(view.allowed, None);
    }

    #[test]
    fn trigger_variable_view_default_for_a_dimension() {
        let v = sample_variable(
            "region",
            Some(VariableValue::Dimension(vec![
                Scalar::from("us-east-1"),
                Scalar::from("eu-west-1"),
            ])),
            None,
        );
        let view = TriggerVariableView::new(&v);

        assert_eq!(
            view.default,
            vec![Scalar::from("us-east-1"), Scalar::from("eu-west-1")]
        );
    }

    #[test]
    fn trigger_variable_view_falls_back_to_the_first_default_when_the_plan_sets_no_value() {
        let mut v = sample_variable("duration", None, None);
        v.declarations[0].default = Some(Scalar::from("5m"));

        let view = TriggerVariableView::new(&v);

        assert_eq!(view.default, vec![Scalar::from("5m")]);
    }

    #[test]
    fn trigger_variable_view_has_no_default_when_nothing_is_set_at_all() {
        let v = sample_variable("needs_a_value", None, None);
        let view = TriggerVariableView::new(&v);

        assert!(view.default.is_empty());
    }

    #[test]
    fn trigger_variable_view_surfaces_allowed_values() {
        let v = sample_variable(
            "tier",
            None,
            Some(vec![Scalar::from("free"), Scalar::from("paid")]),
        );
        let view = TriggerVariableView::new(&v);

        assert_eq!(
            view.allowed,
            Some(vec![Scalar::from("free"), Scalar::from("paid")])
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
            pool: "alpha".to_owned(),
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
                compound_groups: BTreeMap::new(),
            },
            environment: Some(EnvironmentSummary::DockerCompose(
                ComposeEnvironmentSummary {
                    services: Vec::new(),
                    has_variable_replicas: false,
                    services_vary_by_matrix: false,
                    resolved_for_variant: None,
                },
            )),
            history: TestPlanHistory::default(),
        }
    }

    fn compose_view(view: &TestPlanDetailsView) -> &ComposeDetailView {
        match view.environment.as_ref() {
            Some(EnvironmentDetailView::DockerCompose(compose)) => compose,
            other => panic!("expected a docker-compose environment view, got {other:?}"),
        }
    }

    #[test]
    fn test_plan_details_view_to_label_shows_the_last_included_day() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.history.window = HistoryWindow {
            from: day(2026, 7, 18),
            to: day(2026, 8, 18),
        };

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(view.history_from_label, "2026-07-18");
        assert_eq!(view.history_to_label, "2026-08-17");
    }

    #[test]
    fn test_plan_details_view_formula_omits_compound_groups_when_there_are_none() {
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
    fn test_plan_details_view_formula_includes_named_compound_groups_separately() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.matrix.dimensions.insert(
            "region".to_owned(),
            vec![Scalar::from("a"), Scalar::from("b")],
        );
        details.matrix.compound_groups = BTreeMap::from([
            ("colours".to_owned(), vec![BTreeMap::new(), BTreeMap::new()]),
            (
                "subjects".to_owned(),
                vec![BTreeMap::new(), BTreeMap::new(), BTreeMap::new()],
            ),
        ]);

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(view.matrix_formula, "2 region × 2 colours × 3 subjects");
    }

    #[test]
    fn test_plan_details_view_variables_summary_counts_compound_members_as_variables() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.variables = vec![sample_variable("region", None, None)];
        details.matrix.dimensions.insert(
            "region".to_owned(),
            vec![Scalar::from("a"), Scalar::from("b")],
        );
        details.matrix.compound_groups = BTreeMap::from([(
            "subjects".to_owned(),
            vec![BTreeMap::from([
                ("setup".to_owned(), Scalar::from("x")),
                ("scenario".to_owned(), Scalar::from("y")),
            ])],
        )]);

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(
            view.variables_summary,
            "3 variables · 1 matrix dimension · 1 compound dimension"
        );
    }

    #[test]
    fn test_plan_details_view_sums_containers_only_when_replicas_are_all_fixed() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.environment = Some(EnvironmentSummary::DockerCompose(
            ComposeEnvironmentSummary {
                services: vec![
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
                ],
                has_variable_replicas: false,
                services_vary_by_matrix: false,
                resolved_for_variant: None,
            },
        ));

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(compose_view(&view).n_containers_per_execution, Some(3));
    }

    #[test]
    fn test_plan_details_view_omits_the_container_total_when_any_replica_count_is_unresolved() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.environment = Some(EnvironmentSummary::DockerCompose(
            ComposeEnvironmentSummary {
                services: vec![
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
                ],
                has_variable_replicas: true,
                services_vary_by_matrix: false,
                resolved_for_variant: None,
            },
        ));

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert_eq!(compose_view(&view).n_containers_per_execution, None);
    }

    #[test]
    fn test_plan_details_view_shows_k8s_manifest_links_alphabetically() {
        let uuid = Uuid::from_u128(1);
        let manifests = vec![
            ManifestLocation::gh(
                "deployment.yaml",
                "https://github.com/org/repo/blob/main/deployment.yaml",
            ),
            ManifestLocation::inline(
                "generated.yaml",
                "https://github.com/org/repo/blob/main/env.yaml",
            ),
        ];

        let mut details = sample_details(uuid);
        details.environment = Some(EnvironmentSummary::K8s(K8sEnvironmentSummary {
            manifests: manifests.clone(),
            resolved_for_variant: None,
        }));

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        match view.environment {
            Some(EnvironmentDetailView::K8s(k8s)) => {
                assert_eq!(k8s.manifests, manifests);
            }
            other => panic!("expected a k8s environment view, got {other:?}"),
        }
    }

    #[test]
    fn test_plan_details_view_has_no_environment_view_for_a_skipped_environment() {
        let uuid = Uuid::from_u128(1);
        let mut details = sample_details(uuid);
        details.environment = None;

        let view = TestPlanDetailsView::new(details, DEFAULT_DAYS_BACK, DEFAULT_DAYS);

        assert!(view.environment.is_none());
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

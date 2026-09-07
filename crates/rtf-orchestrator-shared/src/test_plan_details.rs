//! Request and response types for `GET /test-plan/{uuid}/details`.
use crate::test_plan::{EnvironmentService, OrchestratorTestPlan, ServiceReplicas};
use chrono::{DateTime, Days, NaiveDate, NaiveTime, Utc};
use rtf_config::{
    StableSource, VariableDefinition,
    context::ResolutionContext,
    formats::{self, Matrix},
    inlining,
    templating::{self, Scalar, Template, TemplateContext},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    mem::take,
};
use uuid::Uuid;

pub const DAY_FORMAT: &str = "%Y-%m-%d";

pub const DEFAULT_DAYS_BACK: u32 = 30;
pub const DEFAULT_DAYS: u32 = 30;
pub const MAX_DAYS_BACK: u32 = 365;
pub const MAX_DAYS: u32 = 365;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPlanDetailsParams {
    #[serde(default = "default_days_back")]
    pub days_back: u32,
    #[serde(default = "default_days")]
    pub days: u32,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

fn default_days_back() -> u32 {
    DEFAULT_DAYS_BACK
}

fn default_days() -> u32 {
    DEFAULT_DAYS
}

impl Default for TestPlanDetailsParams {
    fn default() -> Self {
        Self {
            days_back: default_days_back(),
            days: default_days(),
            git_ref: None,
        }
    }
}

impl TestPlanDetailsParams {
    pub fn history_window(&self, now: DateTime<Utc>) -> HistoryWindow {
        let days_back = self.days_back.min(MAX_DAYS_BACK);
        let days = self.days.clamp(1, MAX_DAYS);
        let today = now.date_naive();
        let midnight = |date: NaiveDate| date.and_time(NaiveTime::MIN).and_utc();

        let end_of_today = midnight(
            today
                .checked_add_days(Days::new(1))
                .expect("a date one day after now is representable"),
        );
        let from = midnight(
            today
                .checked_sub_days(Days::new(days_back.into()))
                .unwrap_or(today),
        );
        let raw_to = from
            .checked_add_days(Days::new(days.into()))
            .unwrap_or(end_of_today);

        let to = if raw_to >= midnight(today) {
            end_of_today
        } else {
            raw_to
        };

        HistoryWindow { from, to }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryWindow {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl HistoryWindow {
    pub fn days(&self) -> Vec<String> {
        let mut days = Vec::new();
        let mut date = self.from.date_naive();
        let last = self.to.date_naive();

        while date < last {
            days.push(date.format(DAY_FORMAT).to_string());
            match date.checked_add_days(Days::new(1)) {
                Some(next) => date = next,
                None => break,
            }
        }

        days
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestPlanDetails {
    pub uuid: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: TestPlanSource,
    pub cluster: String,
    pub variables: Vec<TestPlanVariable>,
    pub matrix: MatrixSummary,
    pub environment: EnvironmentSummary,
    pub history: TestPlanHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlanSource {
    pub org: String,
    pub repo: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    pub sha: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSection {
    Scenario,
    Environment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestPlanVariable {
    pub name: String,
    pub declarations: Vec<VariableDeclaration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_value: Option<VariableValue>,
    pub required: bool,
}

impl TestPlanVariable {
    pub fn from_test_plan(test_plan: &OrchestratorTestPlan) -> Vec<Self> {
        let include_keys: HashSet<&String> = test_plan
            .matrix
            .compound
            .values()
            .flat_map(|entries| entries.iter().take(1).flat_map(|group| group.keys()))
            .collect();

        let sections = [
            (
                ConfigSection::Scenario,
                &test_plan.scenario.variable_definitions,
            ),
            (
                ConfigSection::Environment,
                &test_plan.environment.variable_definitions,
            ),
        ];

        let mut by_name: BTreeMap<&String, Vec<VariableDeclaration>> = BTreeMap::new();
        for (section, definitions) in sections {
            for def in definitions
                .iter()
                .filter(|def| !include_keys.contains(&def.name))
            {
                by_name
                    .entry(&def.name)
                    .or_default()
                    .push(VariableDeclaration::new(section, def));
            }
        }

        by_name
            .into_iter()
            .map(|(name, declarations)| {
                let current_value = VariableValue::try_new(test_plan, name);

                TestPlanVariable {
                    name: name.clone(),
                    required: current_value.is_none()
                        && declarations.iter().any(|d| d.default.is_none()),
                    declarations,
                    current_value,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableDeclaration {
    pub section: ConfigSection,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Scalar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<Scalar>>,
}

impl VariableDeclaration {
    fn new(section: ConfigSection, def: &VariableDefinition) -> Self {
        Self {
            section,
            description: def.description.clone(),
            default: def.default.clone(),
            allowed_values: def.allowed_values.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableValue {
    Scalar(Scalar),
    Dimension(Vec<Scalar>),
}

impl VariableValue {
    fn try_new(test_plan: &OrchestratorTestPlan, name: &String) -> Option<Self> {
        if let Some(scalar) = test_plan.variables.get(name) {
            return Some(Self::Scalar(scalar.clone()));
        }

        test_plan
            .matrix
            .dimensions
            .get(name)
            .map(|values| Self::Dimension(values.clone()))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixSummary {
    pub n_executions: usize,
    pub dimensions: BTreeMap<String, Vec<Scalar>>,
    pub compound_groups: BTreeMap<String, Vec<BTreeMap<String, Scalar>>>,
}

impl MatrixSummary {
    pub fn new(matrix: &Matrix) -> Self {
        Self {
            n_executions: matrix.n_variants(),
            dimensions: matrix
                .dimensions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            compound_groups: matrix
                .compound
                .iter()
                .map(|(name, entries)| {
                    let entries = entries
                        .iter()
                        .map(|g| g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .collect();

                    (name.clone(), entries)
                })
                .collect(),
        }
    }
}

#[derive(Debug)]
pub enum EnvironmentSummaryError {
    Formats(formats::Error),
    Inlining(inlining::Errors),
    Templating(templating::Errors),
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSummary {
    pub services: Vec<EnvironmentService>,
    pub has_variable_replicas: bool,
    pub services_vary_by_matrix: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_for_variant: Option<String>,
}

impl EnvironmentSummary {
    /// The services deployed for each execution, resolved for the first matrix variant.
    pub async fn try_from_test_plan(
        test_plan: &OrchestratorTestPlan,
        ctx: &impl ResolutionContext,
    ) -> Result<Self, EnvironmentSummaryError> {
        let (variant_name, mut variant) = test_plan
            .try_expand_variant(0)
            .map_err(EnvironmentSummaryError::Formats)?
            .expect("a test plan always expands to at least one variant");

        let variables = take(&mut variant.variables);
        let template_ctx =
            TemplateContext::new(variables, HashMap::new(), ctx.custom_provider_definitions());
        variant
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(EnvironmentSummaryError::Templating)?;

        let mut env = variant.environment.execution;
        env.inline_manifest_files(ctx, &mut HashMap::new())
            .await
            .map_err(EnvironmentSummaryError::Inlining)?;

        let services = env
            .services()
            .map_err(|e| EnvironmentSummaryError::Formats(e.into()))?;

        Ok(Self::new(
            services,
            services_vary_by_matrix(test_plan),
            (!test_plan.matrix.is_empty()).then_some(variant_name),
        ))
    }

    fn new(
        services: Vec<EnvironmentService>,
        services_vary_by_matrix: bool,
        resolved_for_variant: Option<String>,
    ) -> Self {
        Self {
            has_variable_replicas: services
                .iter()
                .any(|s| !matches!(s.replicas, ServiceReplicas::Fixed(_))),
            services,
            services_vary_by_matrix,
            resolved_for_variant,
        }
    }
}

/// Whether any compose file provider is templated on a matrix variable, which is what would make
/// different executions deploy a different set of services.
fn services_vary_by_matrix(test_plan: &OrchestratorTestPlan) -> bool {
    let matrix_keys: HashSet<&String> = test_plan.matrix.keys().collect();

    test_plan
        .environment
        .execution
        .manifest_template_variables()
        .iter()
        .any(|name| matrix_keys.contains(name))
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestPlanHistory {
    #[serde(flatten)]
    pub window: HistoryWindow,
    pub runs_by_day: BTreeMap<String, DayCounts>,
    pub execution_durations: DurationHistogram,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayCounts {
    pub successful: u64,
    pub failed: u64,
    pub unrunnable: u64,
}

impl DayCounts {
    pub fn total(&self) -> u64 {
        self.successful + self.failed + self.unrunnable
    }
}

/// Durations are whole seconds, rounded to nearest. An empty histogram has no bins and a `total` of
/// zero, so its bounds are both zero.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationHistogram {
    pub bins: Vec<DurationBin>,
    pub min_secs: u64,
    pub max_secs: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationBin {
    pub lower_secs: u64,
    pub upper_secs: u64,
    pub count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use indoc::indoc;
    use rtf_config::context::Context;
    use simple_test_case::test_case;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 17, 14, 32, 5).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    /// Exercises every shape the variable summary has to cover: a variable the plan sets, one set as
    /// a matrix dimension, one declared by both sections with differing defaults, one with no default
    /// anywhere, and one supplied by the `include` block.
    const TEST_PLAN: &str = indoc!(
        r#"
        name: my-plan
        description: a test plan
        variables:
          message: hello
        matrix:
          dimensions:
            region:
              - us-east-1
              - eu-west-1
          compound:
            include:
              - tier: free
                quota: 10
              - tier: paid
                quota: 100
        scenario:
          name: my-scenario
          description: a scenario
          variable_definitions:
            - name: message
              description: what to say
              default: hi
            - name: shared
              description: used by both sections
              default: from-scenario
            - name: region
              description: where to run
            - name: tier
              description: an include block variable
          docker:
            image: my-image
            command: run
        environment:
          name: my-environment
          description: an environment
          variable_definitions:
            - name: shared
              description: used by both sections
              default: from-environment
            - name: needs_a_value
              description: no default anywhere
              allowed_values:
                - a
                - b
          compose_files:
            - name: compose.yaml
              kind: inline
              content: |
                services:
                  web:
                    image: nginx:1.25
                    deploy:
                      replicas: 2
        "#
    );

    /// A compose file selected by a path templated on a matrix dimension.
    const MATRIX_TEMPLATED_COMPOSE: &str = indoc!(
        r#"
        name: my-plan
        description: a test plan
        matrix:
          dimensions:
            region:
              - us-east-1
              - eu-west-1
        scenario:
          name: my-scenario
          description: a scenario
          docker:
            image: my-image
            command: run
        environment:
          name: my-environment
          description: an environment
          compose_files:
            - name: compose.yaml
              kind: relative_path
              path: "{{ region }}"
        "#
    );

    fn test_plan(yaml: &str) -> OrchestratorTestPlan {
        serde_yaml::from_str(yaml).expect("test plan should deserialize")
    }

    fn variables(yaml: &str) -> Vec<TestPlanVariable> {
        TestPlanVariable::from_test_plan(&test_plan(yaml))
    }

    fn named(vars: &[TestPlanVariable], name: &str) -> TestPlanVariable {
        vars.iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("expected a variable named {name} in {vars:?}"))
            .clone()
    }

    fn window_params(days_back: u32, days: u32) -> TestPlanDetailsParams {
        TestPlanDetailsParams {
            days_back,
            days,
            ..Default::default()
        }
    }

    #[test_case(MAX_DAYS_BACK + 1, day(2025, 8, 17); "days_back capped at the maximum")]
    #[test_case(0, day(2026, 8, 17); "zero days back starts today")]
    #[test]
    fn history_window_caps_days_back(days_back: u32, expected_from: DateTime<Utc>) {
        let window = window_params(days_back, DEFAULT_DAYS).history_window(now());

        assert_eq!(window.from, expected_from);
    }

    #[test_case(0, 1; "zero days clamped up to one")]
    #[test_case(MAX_DAYS + 1, MAX_DAYS + 1; "days capped at the maximum")]
    #[test]
    fn history_window_clamps_days(days: u32, expected: u32) {
        let window = window_params(MAX_DAYS_BACK, days).history_window(now());

        assert_eq!(window.days().len(), expected as usize);
    }

    #[test_case(DEFAULT_DAYS_BACK, DEFAULT_DAYS, true; "default days_back and days include today")]
    #[test_case(60, 30, false; "before today")]
    #[test]
    fn history_window_includes_all_of_today_when_the_window_intersects_it(
        days_back: u32,
        days: u32,
        included: bool,
    ) {
        let window = window_params(days_back, days).history_window(now());
        let today = day(2026, 8, 17);

        assert_eq!(window.to > today, included, "window: {window:?}");
    }

    #[test_case(ServiceReplicas::Fixed(3), false; "fixed replicas")]
    #[test_case(ServiceReplicas::Variable("${N}".to_string()), true; "variable replicas")]
    #[test]
    fn environment_summary_derives_has_variable_replicas(
        replicas: ServiceReplicas,
        expected: bool,
    ) {
        let services = vec![EnvironmentService {
            name: "web".to_string(),
            image: Some("nginx".to_string()),
            replicas,
        }];

        assert_eq!(
            EnvironmentSummary::new(services, false, None).has_variable_replicas,
            expected
        );
    }

    #[test]
    fn test_plan_variables_are_ordered_by_name_and_omit_include_block_variables() {
        let names: Vec<_> = variables(TEST_PLAN).into_iter().map(|v| v.name).collect();

        assert_eq!(names, vec!["message", "needs_a_value", "region", "shared"]);
    }

    #[test]
    fn test_plan_variables_report_a_value_the_plan_sets() {
        let message = named(&variables(TEST_PLAN), "message");

        assert_eq!(
            message.current_value,
            Some(VariableValue::Scalar(Scalar::from("hello")))
        );
        assert!(!message.required, "the plan already sets it");
    }

    #[test]
    fn test_plan_variables_report_a_matrix_dimension_as_its_full_value_set() {
        let region = named(&variables(TEST_PLAN), "region");

        assert_eq!(
            region.current_value,
            Some(VariableValue::Dimension(vec![
                Scalar::from("us-east-1"),
                Scalar::from("eu-west-1"),
            ]))
        );
        assert!(!region.required, "the matrix supplies it");
    }

    #[test]
    fn test_plan_variables_record_a_declaration_per_declaring_section() {
        let shared = named(&variables(TEST_PLAN), "shared");
        let defaults: Vec<_> = shared
            .declarations
            .iter()
            .map(|d| (d.section, d.default.clone()))
            .collect();

        assert_eq!(
            defaults,
            vec![
                (ConfigSection::Scenario, Some(Scalar::from("from-scenario"))),
                (
                    ConfigSection::Environment,
                    Some(Scalar::from("from-environment"))
                ),
            ]
        );
        assert!(!shared.required, "both sections have a default");
    }

    #[test]
    fn test_plan_variables_mark_a_variable_with_no_default_as_required() {
        let needs_a_value = named(&variables(TEST_PLAN), "needs_a_value");

        assert!(needs_a_value.required);
        assert_eq!(needs_a_value.current_value, None);
        assert_eq!(
            needs_a_value.declarations[0].allowed_values,
            Some(vec![Scalar::from("a"), Scalar::from("b")])
        );
    }

    #[test]
    fn matrix_summary_multiplies_dimensions_by_compound_groups() {
        let summary = MatrixSummary::new(&test_plan(TEST_PLAN).matrix);

        assert_eq!(
            summary.n_executions, 4,
            "2 regions x 2 compound group entries"
        );
        assert_eq!(summary.dimensions.len(), 1);
        assert_eq!(summary.compound_groups.len(), 1);
        assert_eq!(summary.compound_groups["include"].len(), 2);
        assert_eq!(
            summary.compound_groups["include"][0].get("tier"),
            Some(&Scalar::from("free"))
        );
    }

    #[test]
    fn matrix_summary_reports_multiple_compound_groups_separately() {
        let matrix = Matrix {
            variant_names: None,
            dimensions: HashMap::new(),
            compound: HashMap::from([
                (
                    "subjects".to_string(),
                    vec![
                        HashMap::from([("setup_subject".to_string(), Scalar::from("world!"))]),
                        HashMap::from([("setup_subject".to_string(), Scalar::from("mother"))]),
                    ],
                ),
                (
                    "colours".to_string(),
                    vec![
                        HashMap::from([("foreground".to_string(), Scalar::from("red"))]),
                        HashMap::from([("foreground".to_string(), Scalar::from("black"))]),
                    ],
                ),
            ]),
        };

        let summary = MatrixSummary::new(&matrix);

        assert_eq!(summary.n_executions, 4, "2 subjects x 2 colours");
        assert_eq!(
            summary.compound_groups.keys().collect::<Vec<_>>(),
            vec!["colours", "subjects"],
            "groups are reported separately and ordered by name"
        );
        assert_eq!(summary.compound_groups["subjects"].len(), 2);
        assert_eq!(summary.compound_groups["colours"].len(), 2);
    }

    #[tokio::test]
    async fn environment_summary_reports_the_deployed_services() {
        let summary =
            EnvironmentSummary::try_from_test_plan(&test_plan(TEST_PLAN), &Context::new())
                .await
                .expect("inline compose files need no resolution");

        assert_eq!(
            summary.services,
            vec![EnvironmentService {
                name: "web".to_string(),
                image: Some("nginx:1.25".to_string()),
                replicas: ServiceReplicas::Fixed(2),
            }]
        );
        assert!(!summary.has_variable_replicas);
        assert!(!summary.services_vary_by_matrix);
        assert_eq!(
            summary.resolved_for_variant,
            Some("matrix_variant_1".to_string())
        );
    }

    #[test]
    fn services_vary_by_matrix_when_a_compose_path_templates_on_a_dimension() {
        assert!(services_vary_by_matrix(&test_plan(
            MATRIX_TEMPLATED_COMPOSE
        )));
        assert!(!services_vary_by_matrix(&test_plan(TEST_PLAN)));
    }
}

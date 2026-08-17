//! Request and response types for `GET /test-plan/{uuid}/details`.
use crate::test_plan::{EnvironmentService, ServiceReplicas};
use chrono::{DateTime, Days, NaiveDate, NaiveTime, Utc};
use rtf_config::templating::Scalar;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, str::FromStr};
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
    #[serde(default = "default_include")]
    pub include: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

fn default_days_back() -> u32 {
    DEFAULT_DAYS_BACK
}

fn default_days() -> u32 {
    DEFAULT_DAYS
}

fn default_include() -> String {
    DetailsSection::ALL.map(|section| section.name()).join(",")
}

impl Default for TestPlanDetailsParams {
    fn default() -> Self {
        Self {
            days_back: default_days_back(),
            days: default_days(),
            include: default_include(),
            git_ref: None,
        }
    }
}

impl TestPlanDetailsParams {
    pub fn sections(&self) -> Result<Vec<DetailsSection>, UnknownDetailsSection> {
        let mut sections: Vec<_> = self
            .include
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(DetailsSection::from_str)
            .collect::<Result<_, _>>()?;

        sections.sort_unstable();
        sections.dedup();

        Ok(sections)
    }

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
        let to = from
            .checked_add_days(Days::new(days.into()))
            .unwrap_or(end_of_today)
            .min(end_of_today);

        HistoryWindow { from, to }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailsSection {
    History,
    Matrix,
    Services,
    Variables,
}

impl DetailsSection {
    pub const ALL: [Self; 4] = [Self::History, Self::Matrix, Self::Services, Self::Variables];

    pub fn name(&self) -> &'static str {
        match self {
            Self::History => "history",
            Self::Matrix => "matrix",
            Self::Services => "services",
            Self::Variables => "variables",
        }
    }

    pub fn needs_test_plan(&self) -> bool {
        !matches!(self, Self::History)
    }
}

impl FromStr for DetailsSection {
    type Err = UnknownDetailsSection;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|section| section.name() == s)
            .ok_or_else(|| UnknownDetailsSection {
                name: s.to_string(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown details section '{name}'")]
pub struct UnknownDetailsSection {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<Vec<TestPlanVariable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MatrixSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<TestPlanHistory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlanSource {
    pub org: String,
    pub repo: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableDeclaration {
    pub section: ConfigSection,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Scalar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<Scalar>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableValue {
    Scalar(Scalar),
    Dimension(Vec<Scalar>),
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixSummary {
    pub n_executions: usize,
    pub dimensions: BTreeMap<String, Vec<Scalar>>,
    pub include_groups: Vec<BTreeMap<String, Scalar>>,
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
    pub fn new(
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    use simple_test_case::test_case;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 17, 14, 32, 5).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    fn params(include: &str) -> TestPlanDetailsParams {
        TestPlanDetailsParams {
            include: include.to_string(),
            ..Default::default()
        }
    }

    #[test_case("history", &[DetailsSection::History]; "single section")]
    #[test_case("variables,matrix", &[DetailsSection::Matrix, DetailsSection::Variables]; "several sections")]
    #[test_case(" history , matrix ", &[DetailsSection::History, DetailsSection::Matrix]; "whitespace trimmed and names sorted")]
    #[test_case("history,history", &[DetailsSection::History]; "repeated section")]
    #[test_case("", &[]; "no sections")]
    #[test]
    fn sections_parses_the_include_list(include: &str, expected: &[DetailsSection]) {
        assert_eq!(params(include).sections().unwrap(), expected.to_vec());
    }

    #[test]
    fn sections_errors_on_an_unknown_name() {
        assert_eq!(
            params("variables,nope").sections(),
            Err(UnknownDetailsSection {
                name: "nope".to_string()
            })
        );
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
    #[test_case(MAX_DAYS + 1, MAX_DAYS; "days capped at the maximum")]
    #[test]
    fn history_window_clamps_days(days: u32, expected: u32) {
        let window = window_params(MAX_DAYS_BACK, days).history_window(now());

        assert_eq!(window.days().len(), expected as usize);
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
}

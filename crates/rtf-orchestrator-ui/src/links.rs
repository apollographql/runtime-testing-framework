use crate::config::Config;
use chrono::{DateTime, SecondsFormat, Utc};
use url::form_urlencoded::byte_serialize;

#[derive(Debug, Clone)]
pub struct LinksConfig {
    pub gcp_project: String,
    pub grafana_base_url: String,
    pub grafana_dashboard_uid: String,
    pub grafana_dashboard_slug: String,
    pub grafana_datasource_uid: String,
}

impl From<&Config> for LinksConfig {
    fn from(cfg: &Config) -> Self {
        Self {
            gcp_project: cfg.gcp_project.clone(),
            grafana_base_url: cfg.grafana_base_url.clone(),
            grafana_dashboard_uid: cfg.grafana_dashboard_uid.clone(),
            grafana_dashboard_slug: cfg.grafana_dashboard_slug.clone(),
            grafana_datasource_uid: cfg.grafana_datasource_uid.clone(),
        }
    }
}

/// Scoped to the execution's namespace (named after its id) over the execution's time window.
pub fn gcp_logs(
    cfg: &LinksConfig,
    cluster: &str,
    namespace: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> String {
    let query = format!(
        "resource.labels.cluster_name=\"{cluster}\"\nresource.labels.namespace_name=\"{namespace}\"",
    );
    let start = start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = end.to_rfc3339_opts(SecondsFormat::Millis, true);

    format!(
        "https://console.cloud.google.com/logs/query;query={};cursorTimestamp={start};startTime={start};endTime={end}?referrer=search&project={}&supportedpurview=project",
        percent_encode(&query),
        cfg.gcp_project,
    )
}

/// Scoped to the execution's namespace (named after its id) over the execution's time window.
pub fn grafana(
    cfg: &LinksConfig,
    namespace: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> String {
    let start = start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = end.to_rfc3339_opts(SecondsFormat::Millis, true);

    format!(
        "{}/d/{}/{}?orgId=1&from={start}&to={end}&timezone=utc&var-datasource={}&var-cluster=&var-namespace={}&refresh=10s",
        cfg.grafana_base_url,
        cfg.grafana_dashboard_uid,
        cfg.grafana_dashboard_slug,
        cfg.grafana_datasource_uid,
        percent_encode(namespace),
    )
}

fn percent_encode(input: &str) -> String {
    byte_serialize(input.as_bytes()).collect()
}

pub(crate) fn sample_config() -> LinksConfig {
    LinksConfig {
        gcp_project: "gcp-project".to_owned(),
        grafana_base_url: "http://grafana.com".to_owned(),
        grafana_dashboard_uid: "dash-uuid".to_owned(),
        grafana_dashboard_slug: "dash-slug".to_owned(),
        grafana_datasource_uid: "data-uuid".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn builds_the_expected_deep_link() {
        let cfg = sample_config();
        let start = Utc.with_ymd_and_hms(2026, 7, 16, 8, 44, 56).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 7, 16, 8, 45, 56).unwrap();
        let namespace = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

        let link = gcp_logs(&cfg, "cluster_name", namespace, start, end);

        assert_eq!(
            link,
            format!(
                "https://console.cloud.google.com/logs/query;query=resource.labels.cluster_name%3D%22cluster_name%22%0Aresource.labels.namespace_name%3D%22{namespace}%22;cursorTimestamp=2026-07-16T08:44:56.000Z;startTime=2026-07-16T08:44:56.000Z;endTime=2026-07-16T08:45:56.000Z?referrer=search&project=gcp-project&supportedpurview=project"
            )
        );
    }

    #[test]
    fn builds_the_expected_grafana_link() {
        let cfg = sample_config();
        let start = Utc.with_ymd_and_hms(2026, 7, 16, 8, 13, 31).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 7, 16, 8, 34, 27).unwrap();
        let namespace = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

        let link = grafana(&cfg, namespace, start, end);

        assert_eq!(
            link,
            format!(
                "http://grafana.com/d/dash-uuid/dash-slug?orgId=1&from=2026-07-16T08:13:31.000Z&to=2026-07-16T08:34:27.000Z&timezone=utc&var-datasource=data-uuid&var-cluster=&var-namespace={namespace}&refresh=10s"
            )
        );
    }
}

use chrono::{DateTime, SecondsFormat, Utc};
use url::form_urlencoded::byte_serialize;

// The orchestrator's workload cluster/project. Hardcoded rather than configured since the orchestrator
// can only target a single cluster (for now)
const PROJECT: &str = "runtime-env-provisioner";
const CLUSTER_NAME: &str = "alpha";

// The Grafana dashboard this links into: "Kubernetes / Compute Resources / Namespace (Pods)".
const GRAFANA_BASE_URL: &str = "https://grafana.rtf.apollographql.com";
const GRAFANA_DASHBOARD_UID: &str = "85a562078cdf77779eaa1add43ccec1e";
const GRAFANA_DASHBOARD_SLUG: &str = "kubernetes-compute-resources-namespace-pods";
const GRAFANA_DATASOURCE_UID: &str = "PC78D5C87463EA889";

/// Build a Cloud Logging deep link scoped to one execution's workload namespace (namespace name
/// == execution id), with the time range narrowed to the execution's own window.
pub fn gcp_logs(namespace: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let query = format!(
        "resource.labels.cluster_name=\"{CLUSTER_NAME}\"\nresource.labels.namespace_name=\"{namespace}\""
    );
    let start = start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = end.to_rfc3339_opts(SecondsFormat::Millis, true);
    format!(
        "https://console.cloud.google.com/logs/query;query={};cursorTimestamp={start};startTime={start};endTime={end}?referrer=search&project={PROJECT}&supportedpurview=project",
        percent_encode(&query),
    )
}

/// Build a Grafana deep link to the "Compute Resources / Namespace (Pods)" dashboard, scoped to
/// one execution's workload namespace (namespace name == execution id), with the time range
/// narrowed to the execution's own window.
pub fn grafana(namespace: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let start = start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = end.to_rfc3339_opts(SecondsFormat::Millis, true);
    format!(
        "{GRAFANA_BASE_URL}/d/{GRAFANA_DASHBOARD_UID}/{GRAFANA_DASHBOARD_SLUG}?orgId=1&from={start}&to={end}&timezone=utc&var-datasource={GRAFANA_DATASOURCE_UID}&var-cluster=&var-namespace={}&refresh=10s",
        percent_encode(namespace),
    )
}

/// Percent-encode `input` for embedding in a URL.
fn percent_encode(input: &str) -> String {
    byte_serialize(input.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn builds_the_expected_deep_link() {
        let start = Utc.with_ymd_and_hms(2026, 7, 16, 8, 44, 56).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 7, 16, 8, 45, 56).unwrap();
        let namespace = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

        let link = gcp_logs(namespace, start, end);

        assert_eq!(
            link,
            format!(
                "https://console.cloud.google.com/logs/query;query=resource.labels.cluster_name%3D%22alpha%22%0Aresource.labels.namespace_name%3D%22{namespace}%22;cursorTimestamp=2026-07-16T08:44:56.000Z;startTime=2026-07-16T08:44:56.000Z;endTime=2026-07-16T08:45:56.000Z?referrer=search&project=runtime-env-provisioner&supportedpurview=project"
            )
        );
    }

    #[test]
    fn builds_the_expected_grafana_link() {
        let start = Utc.with_ymd_and_hms(2026, 7, 16, 8, 13, 31).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 7, 16, 8, 34, 27).unwrap();
        let namespace = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

        let link = grafana(namespace, start, end);

        assert_eq!(
            link,
            format!(
                "https://grafana.rtf.apollographql.com/d/85a562078cdf77779eaa1add43ccec1e/kubernetes-compute-resources-namespace-pods?orgId=1&from=2026-07-16T08:13:31.000Z&to=2026-07-16T08:34:27.000Z&timezone=utc&var-datasource=PC78D5C87463EA889&var-cluster=&var-namespace={namespace}&refresh=10s"
            )
        );
    }
}

use chrono::{DateTime, SecondsFormat, Utc};

// The orchestrator's workload cluster/project. Hardcoded rather than configured since the orchestrator
// can only target a single cluster (for now)
const PROJECT: &str = "runtime-env-provisioner";
const CLUSTER_NAME: &str = "alpha";

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

/// Percent-encode everything outside the URL-unreserved set (`A-Za-z0-9-_.~`). Good enough for the
/// small inputs this module builds URLs from (cluster name, namespace/execution id) — no need for
/// a full `url`-crate dependency just for this.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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
}

//! Smoke test for the [`PrometheusClient`] against a real Prometheus instance.
//!
//! Requires Prometheus to be reachable at `localhost:9090` (e.g. via
//! `kubectl port-forward <prometheus-service-or-pod> 9090:9090`):
//! ```
//! cargo run -p rtf-integrations --example prometheus-query
//! ```
//!
//! Runs a handful of queries chosen to exercise different response shapes:
//! - a valid query that returns data (`up`)
//! - a valid query that returns no data (a `job` label that doesn't match any series)
//! - an invalid query (unclosed brace) — Prometheus responds 400 `bad_data`, caught at
//!   parse time regardless of what's being scraped
//! - a query that parses fine but fails at evaluation time — Prometheus responds 422
//!   `execution` ("many-to-many matching not allowed: matching labels must be unique on
//!   one side"). `on()` matches on the empty label set, so both sides of the `+` collapse
//!   into a single match group; this errors as soon as `up` has 2+ series, which holds for
//!   any Prometheus scraping more than one target
use chrono::{Duration, Utc};
use rtf_integrations::prometheus::{Error, PrometheusClient};

#[tokio::main]
async fn main() {
    let client = PrometheusClient::new("http://localhost:9090");

    run_query(&client, "valid query with data", "up").await;
    run_query(
        &client,
        "valid query with no matching series",
        r#"up{job="this-job-does-not-exist"}"#,
    )
    .await;
    run_query(&client, "invalid syntax (unclosed brace)", "up{").await;
    run_query(
        &client,
        "many-to-many match (evaluation error)",
        "up + on() up",
    )
    .await;
}

async fn run_query(client: &PrometheusClient, label: &str, query: &str) {
    let end = Utc::now();
    let start = end - Duration::minutes(5);

    println!("--- {label} ---");
    println!("query: {query}");

    match client.query_range(query, "15s", &start, &end).await {
        Ok(result) => println!("ok: {result}"),
        Err(Error::Api {
            status,
            error_type,
            message,
        }) => println!("api error ({status}): {error_type}: {message}"),
        Err(Error::Reqwest(err)) => println!("transport error: {err}"),
    }

    println!();
}

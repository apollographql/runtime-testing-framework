use crate::new_client;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use graphql_client::GraphQLQuery;
use rtf_core::graphos::platform_query::PlatformQuery;
use serde::Serialize;

pub async fn get_launch_history(graph_ref: &str, n: usize) -> Result<()> {
    let (graph_id, variant) = graph_ref
        .split_once('@')
        .ok_or(anyhow!("invalid graph ref"))?;

    let client = new_client();
    let platform_client = client.platform_client().ok_or(anyhow!(
        "no API credentials provided for making Apollo platform requests"
    ))?;

    let max_batch_size = 100; // enforced by the studio API
    let batches = n / max_batch_size;
    let mut overflow = n % max_batch_size;
    let mut launches = Vec::with_capacity(n);
    let mut offset = 0;

    for _ in 0..batches {
        let (batch_size, batch) = LaunchHistory::fetch(
            launch_history::Variables {
                graph_id: graph_id.into(),
                variant: variant.into(),
                limit: 100,
                offset,
            },
            platform_client,
        )
        .await?;

        launches.extend(batch);
        offset += batch_size as i64;
        if batch_size < max_batch_size {
            overflow = 0;
            break;
        }
    }

    if overflow > 0 {
        let (_, batch) = LaunchHistory::fetch(
            launch_history::Variables {
                graph_id: graph_id.into(),
                variant: variant.into(),
                limit: overflow as i64,
                offset,
            },
            platform_client,
        )
        .await?;

        launches.extend(batch);
    }

    let mut prev = launches[0];
    for at in launches {
        let delta = prev.signed_duration_since(at);
        let delta_mins = delta.num_minutes();
        prev = at;

        println!("{}", serde_json::to_string(&Launch { at, delta_mins })?);
    }

    Ok(())
}

#[derive(Serialize)]
struct Launch {
    at: Timestamp,
    delta_mins: i64,
}

type Timestamp = DateTime<Utc>;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "../rtf-core/resources/engine-prod-schema.graphql",
    query_path = "resources/queries/launch_history.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
pub struct LaunchHistory;

impl PlatformQuery for LaunchHistory {
    type Output = (usize, Vec<Timestamp>);
    type Error = anyhow::Error;

    fn try_parse(
        data: Self::ResponseData,
        _vars: launch_history::Variables,
    ) -> Result<(usize, Vec<Timestamp>)> {
        let launches = data
            .graph
            .ok_or(anyhow!("unknown graph"))?
            .variant
            .ok_or(anyhow!("unknown variant"))?
            .launch_history
            .unwrap_or_default();

        let n = launches.len();

        Ok((
            n,
            launches
                .into_iter()
                .filter(|l| l.is_completed == Some(true))
                .flat_map(|l| l.completed_at)
                .collect(),
        ))
    }
}

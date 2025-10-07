use crate::new_client;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use graphql_client::GraphQLQuery;
use rtf_core::graphos::platform_query::PlatformQuery;
use serde::Serialize;
use tabled::{Table, Tabled, settings::Style};

pub async fn print_launch_history(graph_ref: String, n: usize, json_output: bool) -> Result<()> {
    let launches = get_launch_history(graph_ref, n).await?;

    if json_output {
        println!("{}", serde_json::to_string(&launches)?);
    } else {
        let mut table = Table::new(launches);
        table.with(Style::markdown());
        println!("{table}");
    }

    Ok(())
}

pub(crate) async fn get_launch_history(graph_ref: String, n: usize) -> Result<Vec<Launch>> {
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
    let mut raw_launches = Vec::with_capacity(n);
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

        raw_launches.extend(batch);
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

        raw_launches.extend(batch);
    }

    let mut launches = Vec::with_capacity(raw_launches.len());
    let mut prev = raw_launches[0].1;
    for (id, at) in raw_launches {
        let delta = prev.signed_duration_since(at);
        let delta_mins = delta.num_minutes();
        prev = at;

        launches.push(Launch { id, at, delta_mins });
    }

    Ok(launches)
}

#[derive(Serialize, Tabled)]
pub(crate) struct Launch {
    pub id: String,
    pub at: Timestamp,
    pub delta_mins: i64,
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
    type Output = (usize, Vec<(String, Timestamp)>);
    type Error = anyhow::Error;

    fn try_parse(
        data: Self::ResponseData,
        _vars: launch_history::Variables,
    ) -> Result<(usize, Vec<(String, Timestamp)>)> {
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
                .flat_map(|l| l.completed_at.map(|cmp_at| (l.id, cmp_at)))
                .collect(),
        ))
    }
}

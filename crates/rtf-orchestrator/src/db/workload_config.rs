use crate::db::{Error, Result};
use rtf_orchestrator_shared::workload_config::WorkloadConfig;
use serde_json::Value;
use sqlx::PgConnection;

/// Insert a resolved workload config, returning the id of the row that holds it.
pub async fn upsert_workload_config(
    config: &WorkloadConfig,
    conn: &mut PgConnection,
) -> Result<i32> {
    let data = serde_json::to_value(config).expect("workload config to serialize");
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO workload_config (data)
         VALUES ($1)
         ON CONFLICT (data) DO UPDATE SET data = EXCLUDED.data
         RETURNING id;
        ",
    )
    .bind(data)
    .fetch_one(conn)
    .await?;

    Ok(id)
}

/// Fetch the resolved workload config stored under the given id.
pub async fn get_workload_config(id: i32, conn: &mut PgConnection) -> Result<WorkloadConfig> {
    let (data,): (Value,) = sqlx::query_as("SELECT data FROM workload_config WHERE id = $1;")
        .bind(id)
        .fetch_one(conn)
        .await?;

    serde_json::from_value(data).map_err(Error::MalformedWorkloadConfig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn;

    fn config(allow_k8s_write: bool, weights: &[(&str, u32)]) -> WorkloadConfig {
        WorkloadConfig {
            allow_k8s_write,
            node_label_weights: weights.iter().map(|(v, w)| (v.to_string(), *w)).collect(),
        }
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn upsert_returns_same_id_for_identical_configs() -> Result<()> {
        let c = conn!();
        let cfg = config(true, &[("a", 1), ("b", 2)]);

        let id1 = upsert_workload_config(&cfg, c).await?;
        let id2 = upsert_workload_config(&cfg, c).await?;

        assert_eq!(id1, id2, "identical configs should dedupe to the same id");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn upsert_returns_new_id_for_distinct_configs() -> Result<()> {
        let c = conn!();

        let id1 = upsert_workload_config(&config(false, &[("a", 1)]), c).await?;
        let id2 = upsert_workload_config(&config(false, &[("a", 2)]), c).await?;

        assert_ne!(id1, id2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn weight_order_is_significant() -> Result<()> {
        let c = conn!();

        let id1 = upsert_workload_config(&config(false, &[("a", 1), ("b", 2)]), c).await?;
        let id2 = upsert_workload_config(&config(false, &[("b", 2), ("a", 1)]), c).await?;

        assert_ne!(
            id1, id2,
            "weights are an ordered list (ties go to the earliest label) so order must be kept"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_returns_what_was_stored() -> Result<()> {
        let c = conn!();
        let cfg = config(true, &[("a", 1), ("b", 2)]);

        let id = upsert_workload_config(&cfg, c).await?;

        assert_eq!(get_workload_config(id, c).await?, cfg);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_reports_a_malformed_row_rather_than_panicking() -> Result<()> {
        let c = conn!();
        let (id,): (i32,) = sqlx::query_as(
            "INSERT INTO workload_config (data) VALUES ('{\"node_label_weights\": \"nope\"}') \
             ON CONFLICT (data) DO UPDATE SET data = EXCLUDED.data RETURNING id;",
        )
        .fetch_one(&mut *c)
        .await?;

        assert!(matches!(
            get_workload_config(id, c).await,
            Err(Error::MalformedWorkloadConfig(_))
        ));

        Ok(())
    }
}

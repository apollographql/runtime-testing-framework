use crate::db::Result;
use serde_json::Value;
use sqlx::PgConnection;

/// Insert a captured variables blob, returning the id of the row that holds it.
///
/// Identical blobs deduplicate via the `UNIQUE` constraint on `data`: a repeat insert returns the
/// existing row's id rather than creating a new one. `ON CONFLICT ... DO UPDATE` (rather than `DO
/// NOTHING`) guarantees a row is always returned so `RETURNING id` yields the existing id on a
/// conflict. Postgres normalizes jsonb on storage, so key order does not affect equality.
pub async fn upsert_variables(data: &Value, conn: &mut PgConnection) -> Result<i32> {
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO variables (data)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn;
    use serde_json::json;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn upsert_returns_same_id_for_identical_blobs() -> Result<()> {
        let c = conn!();
        let data = json!({ "region": "us", "tier": [1, 2, 3] });

        let id1 = upsert_variables(&data, c).await?;
        let id2 = upsert_variables(&data, c).await?;

        assert_eq!(id1, id2, "identical blobs should dedupe to the same id");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn upsert_dedupes_regardless_of_key_order() -> Result<()> {
        let c = conn!();

        let id_a = upsert_variables(&json!({ "region": "us", "tier": 3 }), c).await?;
        let id_b = upsert_variables(&json!({ "tier": 3, "region": "us" }), c).await?;

        assert_eq!(
            id_a, id_b,
            "jsonb normalizes key order, so these should dedupe"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn upsert_returns_new_id_for_distinct_blobs() -> Result<()> {
        let c = conn!();

        let id1 = upsert_variables(&json!({ "only_in_first": 1 }), c).await?;
        let id2 = upsert_variables(&json!({ "only_in_second": 2 }), c).await?;

        assert_ne!(id1, id2, "distinct blobs should get distinct ids");

        Ok(())
    }
}

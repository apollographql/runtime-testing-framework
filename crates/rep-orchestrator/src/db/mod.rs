use sqlx::{Database, FromRow, PgConnection, Postgres};
use thiserror::Error;

pub mod pool;
pub mod test_execution;
pub mod test_run;

#[macro_export]
macro_rules! conn {
    { } => {
        &mut *(
            $crate::db::pool::get_pool()
                .await?
                .acquire()
                .await
                .map_err($crate::db::Error::from)?
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// Helper trait for common queries and semantics when interacting with the DB.
pub trait Queryable: Send + Unpin + for<'r> FromRow<'r, <Postgres as Database>::Row> {
    const TABLE_NAME: &'static str;

    fn id(&self) -> i32;

    fn get_by_id(
        id: i32,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<Option<Self>>> + Send {
        async move {
            Ok(sqlx::query_as(&format!(
                "SELECT * FROM {} WHERE id = $1;",
                Self::TABLE_NAME
            ))
            .bind(id)
            .fetch_optional(conn)
            .await?)
        }
    }

    fn get_by_id_unchecked(
        id: i32,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<Self>> + Send {
        async move {
            Ok(sqlx::query_as(&format!(
                "SELECT * FROM {} WHERE id = $1;",
                Self::TABLE_NAME
            ))
            .bind(id)
            .fetch_one(conn)
            .await?)
        }
    }
}

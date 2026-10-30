use crate::{config::Config, conn, db::Result};
use sqlx::{
    Connection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::sync::OnceCell;
use tracing::info;

const MAX_POOL_CONNECTIONS: u32 = 20;
static POOL: OnceCell<PgPool> = OnceCell::const_new();

/// Marker to track when migrations have been run during tests
#[cfg(test)]
static MIGRATED: OnceCell<()> = OnceCell::const_new();

fn connect_options(cfg: &Config) -> PgConnectOptions {
    let mut opts = PgConnectOptions::new()
        .host(&cfg.db.host)
        .port(cfg.db.port)
        .username(&cfg.db.user)
        .database(&cfg.db.name);

    if let Some(ref pass) = cfg.db.pass {
        opts = opts.password(pass);
    }

    opts
}

pub async fn init_pool(cfg: &Config) -> Result<PgPool> {
    info!("Initialising DB connection pool");
    let pool = PgPoolOptions::new()
        .max_connections(MAX_POOL_CONNECTIONS)
        .connect_with(connect_options(cfg))
        .await?;

    Ok(pool)
}

pub async fn init_pool_and_migrate() -> Result<PgPool> {
    let pool = init_pool(Config::get()).await?;

    info!("Running DB migrations");
    sqlx::migrate!("sql/migrations").run(&pool).await?;

    Ok(pool)
}

pub async fn get_pool() -> Result<&'static PgPool> {
    POOL.get_or_try_init(init_pool_and_migrate).await
}

/// Acquire a connection from the shared pool. Prefer the [conn] macro over calling this directly.
#[cfg(not(test))]
pub async fn get_conn() -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    Ok(get_pool().await?.acquire().await?)
}

/// When running tests we open a fresh, unpooled connection each time rather than sharing the
/// global pool: a shared pool ends up owned by the runtime of the test that initialised it, and
/// tears down when that test completes. Each connection is closed when it is dropped so we don't
/// exhaust the connections available on the DB.
#[cfg(test)]
pub async fn get_conn() -> Result<Box<sqlx::PgConnection>> {
    let opts = connect_options(Config::get());
    let mut conn = sqlx::PgConnection::connect_with(&opts).await?;

    MIGRATED
        .get_or_try_init(|| async {
            sqlx::migrate!("sql/migrations")
                .run_direct(None, &mut conn, false)
                .await
        })
        .await?;

    Ok(Box::new(conn))
}

pub async fn check_db_conn() -> Result<()> {
    conn!().ping().await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn check_db_conn_returns_ok_with_running_db() {
        let res = check_db_conn().await;
        assert!(res.is_ok(), "{res:?}");
    }
}

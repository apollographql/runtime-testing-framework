use crate::{config::Config, conn, db::Result};
use sqlx::{
    Connection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::sync::OnceCell;
use tracing::info;

const MAX_POOL_CONNECTIONS: u32 = 20;
static POOL: OnceCell<PgPool> = OnceCell::const_new();

pub async fn init_pool(cfg: &Config) -> Result<PgPool> {
    info!("Initialising DB connection pool");
    let opts = PgConnectOptions::new()
        .host(&cfg.host)
        .port(cfg.db_port)
        .username(&cfg.db_user)
        .password(&cfg.db_pass)
        .database(&cfg.db_name);

    let pool = PgPoolOptions::new()
        .max_connections(MAX_POOL_CONNECTIONS)
        .connect_with(opts)
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
    if cfg!(test) {
        // When running tests we need to create a connection pool per-test in order to avoid
        // async-drop issues, otherwise POOL ends up being owned by the test that initialised it
        // and that test completing tears it down.
        let pool = init_pool_and_migrate().await?;
        Ok(Box::leak(Box::new(pool)))
    } else {
        POOL.get_or_try_init(init_pool_and_migrate).await
    }
}

pub async fn check_db_conn() -> Result<()> {
    conn!().ping().await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn check_works_with_a_running_db() {
        let res = check_db_conn().await;
        assert!(res.is_ok(), "{res:?}");
    }
}

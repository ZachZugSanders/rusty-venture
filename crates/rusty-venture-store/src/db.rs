use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

pub type Pool = sqlx::SqlitePool;

/// Open (or create) the SQLite database at `database_url` and run all pending
/// migrations. Call once at application startup.
///
/// Accepts `sqlite://path/to/file.db` or `sqlite://:memory:`.
/// If `database_url` is `None`, defaults to `sqlite://rusty-venture.db` in the
/// current working directory.
pub async fn open_pool(database_url: Option<&str>) -> Result<Pool> {
    let url = database_url.unwrap_or("sqlite://rusty-venture.db");

    let options = SqliteConnectOptions::from_str(url)
        .with_context(|| format!("Invalid database URL: {url}"))?
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| format!("Failed to open SQLite database at {url}"))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    tracing::info!(url, "Database ready");
    Ok(pool)
}

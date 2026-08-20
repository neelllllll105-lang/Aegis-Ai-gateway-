//! Connection pool and migrations.

use crate::config::Config;
use crate::error::{AegisError, Result};
use sqlx::postgres::{PgPoolOptions, PgSslMode};
use sqlx::PgPool;
use std::time::Duration;

/// Migrations, embedded in the binary.
///
/// Embedding rather than reading from disk means the deployed artifact carries the exact
/// schema it expects. A container cannot start against migrations it was not built with,
/// which removes an entire class of "it worked in staging" failures.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Connect to PostgreSQL.
///
/// The pool is bounded by `DATABASE_MAX_CONNECTIONS` (default 20) against a server
/// configured for 200: several gateway replicas plus workers and a psql session must all
/// fit without exhausting the server.
pub async fn connect(config: &Config) -> Result<PgPool> {
    let url = config
        .database_url
        .as_deref()
        .ok_or_else(|| AegisError::Config("DATABASE_URL is not set".into()))?;

    let mut options: sqlx::postgres::PgConnectOptions = url
        .parse()
        .map_err(|e| AegisError::Config(format!("invalid DATABASE_URL: {e}")))?;

    if config.environment.is_production_like() {
        options = options.ssl_mode(PgSslMode::Require);
    }

    PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .min_connections(2)
        // Long enough to ride out a brief failover, short enough that a request does not
        // sit waiting past its own timeout.
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1_800))
        .test_before_acquire(true)
        .connect_with(options)
        .await
        .map_err(AegisError::Database)
}

/// Apply pending migrations.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| AegisError::Internal(format!("migration failed: {e}")))?;
    tracing::info!("database migrations applied");
    Ok(())
}

/// Ensure the current and next two monthly usage partitions exist.
///
/// Called at startup and hourly. A missing partition rejects inserts into the billing
/// source of truth, so this runs before the gateway accepts any traffic.
pub async fn maintain_partitions(pool: &PgPool) -> Result<Vec<String>> {
    let rows: Vec<(Option<String>,)> = sqlx::query_as("SELECT maintain_usage_partitions()")
        .fetch_all(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(rows.into_iter().filter_map(|(name,)| name).collect())
}

/// Round-trip check for `/health`.
pub async fn ping(pool: &PgPool) -> Result<()> {
    sqlx::query("SELECT 1")
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(AegisError::Database)
}

/// Pool statistics for the admin console.
pub fn pool_stats(pool: &PgPool) -> (u32, usize) {
    (pool.size(), pool.num_idle())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connecting_without_a_url_is_a_configuration_error() {
        let config = Config::for_tests();
        let err = connect(&config).await.unwrap_err();
        assert_eq!(err.error_type(), "internal_error");
        assert!(format!("{err}").contains("DATABASE_URL"));
    }

    #[tokio::test]
    async fn a_malformed_url_is_rejected_before_any_connection_attempt() {
        let mut config = Config::for_tests();
        config.database_url = Some("this is not a url".into());
        assert!(connect(&config).await.is_err());
    }

    #[test]
    fn every_migration_is_embedded() {
        // Guards against a migration file added to the directory but never shipped,
        // which would leave production on an older schema than the code expects.
        let migrations = MIGRATOR.migrations.len();
        assert!(migrations >= 2, "expected at least 2 migrations, found {migrations}");
    }

    #[test]
    fn migrations_are_sequentially_versioned() {
        let mut versions: Vec<i64> = MIGRATOR.migrations.iter().map(|m| m.version).collect();
        versions.sort_unstable();
        for (index, version) in versions.iter().enumerate() {
            assert_eq!(
                *version,
                index as i64 + 1,
                "migration versions must be contiguous from 1: {versions:?}"
            );
        }
    }
}

//! Shared harness for integration tests.
//!
//! # Why these tests skip rather than fail without a database
//!
//! `AEGIS_TEST_DATABASE_URL` is set in CI and usually not locally. When it is absent the
//! tests return early instead of failing, so `cargo test` is green on a fresh clone with
//! nothing installed but Rust — the property `docs/adr/0004-runtime-checked-sql.md` exists
//! to protect.
//!
//! The obvious objection is that a skipped test proves nothing, and that is true. The
//! mitigation is that CI always provides the database, so nothing merges without these
//! having run. What is avoided is the far worse failure mode where a contributor sees red
//! tests on a clean checkout, learns the suite is unreliable, and stops reading it.

// Cargo compiles this module separately into every integration test binary, and each one
// uses a different subset of it. Without this, adding a helper that only one test file
// needs breaks the build for all the others.
#![allow(dead_code)]

use aegis_gateway::db::pool;
use aegis_gateway::store::RedisStore;
use aegis_gateway::AppState;
use sqlx::PgPool;
use uuid::Uuid;

/// Connect to the test database, or return `None` when one is not configured.
///
/// Prints a clearly marked notice so a skipped run is visible in the output rather than
/// looking like a pass.
pub async fn test_pool() -> Option<PgPool> {
    let url = std::env::var("AEGIS_TEST_DATABASE_URL").ok()?;

    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        Err(e) => {
            // A configured-but-unreachable database is a real failure, not a skip.
            panic!("AEGIS_TEST_DATABASE_URL is set but unreachable: {e}");
        }
    };

    pool::migrate(&pool).await.expect("migrations must apply");
    pool::maintain_partitions(&pool)
        .await
        .expect("usage partitions must exist");

    Some(pool)
}

/// Connect to a real Redis instance, or return `None` when one is not configured.
///
/// The counterpart to [`test_pool`] for exactly the gap the enterprise readiness audit
/// found: CI has provisioned a real Redis service container and set
/// `AEGIS_TEST_REDIS_URL` since this project's CI was written, and until this helper
/// existed, no test anywhere read that variable — every "atomic under concurrency" proof
/// (the rate limiter, the scheduler's distributed claim, the budget reservation) had only
/// ever run against `MemoryStore`, whose atomicity comes from a single in-process mutex
/// and says nothing about whether the equivalent Lua scripts and `INCRBY` calls are
/// genuinely atomic against a real Redis server.
pub async fn test_redis_store() -> Option<RedisStore> {
    let url = std::env::var("AEGIS_TEST_REDIS_URL").ok()?;
    match RedisStore::connect(&url).await {
        Ok(store) => Some(store),
        // A configured-but-unreachable Redis is a real failure, not a skip — mirrors
        // test_pool's identical reasoning for AEGIS_TEST_DATABASE_URL.
        Err(e) => panic!("AEGIS_TEST_REDIS_URL is set but unreachable: {e}"),
    }
}

/// Announce a Redis-specific skip.
pub fn skip_redis(test_name: &str) {
    eprintln!(
        "SKIPPED {test_name}: set AEGIS_TEST_REDIS_URL to run this test against real Redis \
         (CI always does)"
    );
}

/// Announce a skip so it is visible in test output.
pub fn skip(test_name: &str) {
    eprintln!(
        "SKIPPED {test_name}: set AEGIS_TEST_DATABASE_URL to run integration tests \
         (CI always does)"
    );
}

/// Build application state backed by a real database.
pub fn state_with(pool: PgPool) -> AppState {
    AppState {
        db: Some(pool),
        ..AppState::for_tests()
    }
}

/// A test organisation with an owner, created fresh.
pub struct Fixture {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
}

/// Create an isolated organisation and owner.
///
/// Every fixture gets a unique email and slug so tests can run concurrently and repeatedly
/// against the same database without colliding or needing a truncate between runs.
pub async fn create_org(pool: &PgPool, label: &str) -> Fixture {
    use aegis_gateway::db::repo;

    let unique = Uuid::new_v4().simple().to_string();
    let email = format!("{label}-{}@test.invalid", &unique[..12]);

    let user = repo::create_user(pool, &email, Some("$argon2id$fake"), Some(label))
        .await
        .expect("user creation");

    let org = repo::create_org_with_owner(
        pool,
        &format!("{label} org"),
        &format!("{label}-{}", &unique[..12]),
        user.id,
    )
    .await
    .expect("org creation");

    Fixture {
        org_id: org.id,
        user_id: user.id,
        email,
    }
}

/// Create a shared (unassigned) API key for an organisation, returning the plaintext and
/// its id.
pub async fn create_key(pool: &PgPool, fixture: &Fixture, name: &str) -> (String, Uuid) {
    create_key_assigned(pool, fixture, name, None).await
}

/// Create an API key issued to a specific person, so its traffic attributes to them.
pub async fn create_key_assigned(
    pool: &PgPool,
    fixture: &Fixture,
    name: &str,
    assigned_to: Option<Uuid>,
) -> (String, Uuid) {
    use aegis_gateway::crypto;
    use aegis_gateway::db::repo;

    let generated = crypto::generate_api_key();
    let key = repo::create_api_key(
        pool,
        fixture.org_id,
        Some(fixture.user_id),
        name,
        &generated.prefix,
        &generated.hash,
        None,
        60,
        None,
        None,
        None,
        assigned_to,
    )
    .await
    .expect("key creation");

    (generated.plaintext, key.id)
}

/// Remove a fixture organisation and everything cascading from it.
pub async fn cleanup(pool: &PgPool, fixture: &Fixture) {
    let _ = sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(fixture.org_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(fixture.user_id)
        .execute(pool)
        .await;
}

/// Shorthand for state plus a pool in one call.
pub async fn setup() -> Option<(AppState, PgPool)> {
    let pool = test_pool().await?;
    Some((state_with(pool.clone()), pool))
}

/// Re-export so tests need only one import.
///
/// Not every integration test binary that pulls in `common` uses this one — `#![allow(
/// dead_code)]` above covers unused functions, but not an unused `pub use`, so it needs
/// its own allow rather than tripping `-D warnings` in whichever binary happens not to
/// touch `repo` directly.
#[allow(unused_imports)]
pub use aegis_gateway::db::repo;

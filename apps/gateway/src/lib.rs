//! Aegis gateway.
//!
//! An AI cost-optimization gateway: applications point at us instead of at OpenAI,
//! Anthropic, or Google, and every request is authenticated, rate limited, budget
//! checked, cached, routed to the cheapest model that preserves quality, metered, and
//! attributed — in under a millisecond of our own added latency.
//!
//! Start with `MASTER_BUILD.md` for the product blueprint and `MEMORY.md` for current
//! state. The request pipeline is `MASTER_BUILD.md` Part 5, implemented in
//! [`routes::openai_compat`] and orchestrated by [`engine`].

pub mod billing;
pub mod cache;
pub mod config;
pub mod crypto;
pub mod db;
pub mod engine;
pub mod enterprise;
pub mod error;
pub mod metering;
pub mod metrics;
pub mod middleware;
pub mod money;
pub mod providers;
pub mod router;
pub mod routes;
pub mod store;
pub mod telemetry;
pub mod types;
pub mod workers;

pub use router::build_router;

use crate::config::Config;
use crate::engine::bandit::RoutingBandit;
use crate::engine::fallback::ProviderHealth;
use crate::metering::pricing::PricingTable;
use crate::metrics::Metrics;
use crate::middleware::auth::KeyCache;
use crate::providers::pool::SharedKeyPool;
use crate::providers::ProviderRegistry;
use crate::store::KvStore;
use std::sync::Arc;
use std::time::Instant;

/// Everything a request handler needs, shared immutably across all workers.
///
/// Cheap to clone: every field is either `Arc` or a handle type whose clone is a
/// reference count bump. Axum clones this per request, so nothing expensive belongs here
/// by value.
#[derive(Clone)]
pub struct AppState {
    /// Parsed, validated configuration. Immutable after startup.
    pub config: Arc<Config>,
    /// Hot-path key/value store (Redis in production, in-memory in development).
    pub store: Arc<dyn KvStore>,
    /// Prometheus metric registry.
    pub metrics: Arc<Metrics>,
    /// PostgreSQL pool. `None` when running without a database, in which case endpoints
    /// that require persistence return 503 rather than panicking.
    pub db: Option<sqlx::PgPool>,

    /// Read replica for analytics queries. `None` means "use the primary".
    pub db_replica: Option<sqlx::PgPool>,
    /// Model pricing, refreshed periodically from the database.
    pub pricing: Arc<PricingTable>,
    /// Provider adapters, keyed by provider id.
    pub providers: Arc<ProviderRegistry>,
    /// In-process LRU in front of Redis for API key lookups.
    pub key_cache: Arc<KeyCache>,
    /// Per-provider circuit breakers.
    pub health: Arc<ProviderHealth>,
    /// Outcome-driven routing statistics (Phase 7).
    pub bandit: Arc<RoutingBandit>,
    /// Round-robin over our own pooled provider keys, backing the free tier.
    pub shared_pool: Arc<SharedKeyPool>,
    /// Shared HTTP client — connection pooling across every upstream call.
    pub http: reqwest::Client,
    /// Process start time, for uptime reporting.
    pub started_at: Instant,
}

impl AppState {
    /// The database pool, or a 503-shaped error when running without persistence.
    pub fn db(&self) -> error::Result<&sqlx::PgPool> {
        self.db.as_ref().ok_or_else(|| {
            error::AegisError::Internal(
                "this endpoint requires a database; DATABASE_URL is not configured".into(),
            )
        })
    }

    /// The pool analytics queries should use.
    ///
    /// The replica when one is configured, the primary otherwise. Callers do not branch
    /// on this: a report reads from whatever this returns, so removing the replica from
    /// the environment changes performance and nothing else.
    ///
    /// Never use this for anything a write depends on. Replication lag is real, and a
    /// read-after-write against a replica can legitimately return the previous value.
    pub fn analytics_db(&self) -> error::Result<&sqlx::PgPool> {
        if let Some(replica) = self.db_replica.as_ref() {
            return Ok(replica);
        }
        self.db()
    }

    /// Build state for tests: in-memory store, no database, seeded pricing.
    pub fn for_tests() -> AppState {
        AppState {
            config: Arc::new(Config::for_tests()),
            store: Arc::new(store::MemoryStore::new()),
            metrics: Arc::new(Metrics::new()),
            db: None,
            db_replica: None,
            pricing: Arc::new(PricingTable::with_seed_data()),
            providers: Arc::new(ProviderRegistry::with_builtins()),
            key_cache: Arc::new(KeyCache::default()),
            health: Arc::new(ProviderHealth::new()),
            bandit: Arc::new(RoutingBandit::new()),
            shared_pool: Arc::new(SharedKeyPool::new()),
            http: reqwest::Client::new(),
            started_at: Instant::now(),
        }
    }
}

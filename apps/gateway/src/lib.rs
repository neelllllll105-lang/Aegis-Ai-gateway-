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
use crate::metering::pricing::{PricingSnapshot, PricingSource, PricingTable};
use crate::metrics::Metrics;
use crate::middleware::auth::KeyCache;
use crate::providers::pool::SharedKeyPool;
use crate::providers::ProviderRegistry;
use crate::store::KvStore;
use std::sync::{Arc, RwLock};
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
    /// Model pricing. Loaded at startup and re-read from the database every
    /// [`workers::pricing_refresh::REFRESH_INTERVAL`] by a background worker, so a price
    /// a human has just verified and committed reaches every replica within minutes
    /// rather than at the next deploy. Behind a lock rather than a bare `Arc<PricingTable>`
    /// so that refresh is possible at all; reads are a lock acquisition plus one `Arc`
    /// clone, which is not meaningfully different in cost from the network call to an
    /// upstream model provider that every request already makes. Use [`AppState::pricing`]
    /// to read it — the field itself holds the swappable cell.
    pub pricing: Arc<RwLock<PricingSnapshot>>,
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
    /// Vector storage for the semantic cache (Qdrant in production, in-memory in
    /// development) — pipeline stage [5b].
    pub semantic_store: Arc<dyn cache::semantic::VectorStore>,
    /// Generates embeddings for semantic-cache lookups, on Aegis's own pooled credential
    /// rather than a customer's BYOK key. See `cache::embed` for why.
    pub embedder: Arc<dyn cache::embed::Embedder>,
    /// Process start time, for uptime reporting.
    pub started_at: Instant,
}

impl AppState {
    /// The database pool, or a 503-shaped error when running without persistence.
    pub fn db(&self) -> error::Result<&sqlx::PgPool> {
        self.db.as_ref().ok_or_else(|| {
            // 503, not 500: no database configured is a known, anticipated operational
            // state — the same one `/health` already reports clearly — not a bug in this
            // request. Found live, testing the dashboard against a database-less gateway:
            // signup returned a bare 500 "internal_error" with no actionable signal,
            // instead of the 401-or-503 pattern the rest of the management API already
            // followed for a missing dependency.
            error::AegisError::ServiceUnavailable(
                "This service is temporarily unavailable — the database is not reachable. \
                 Please try again shortly."
                    .into(),
            )
        })
    }

    /// The current pricing table. What every request handler calls.
    ///
    /// A lock acquisition plus an `Arc` clone — nanoseconds, and no request ever blocks
    /// on it for longer than that, since [`AppState::set_pricing`] holds the write lock
    /// only long enough to swap one pointer.
    pub fn pricing(&self) -> Arc<PricingTable> {
        self.pricing
            .read()
            .expect("pricing lock poisoned")
            .table
            .clone()
    }

    /// The full snapshot — table plus when and where it was loaded from. What the admin
    /// console's staleness banner and `GET /api/admin/pricing` read; ordinary request
    /// handling never needs this, only [`AppState::pricing`].
    pub fn pricing_snapshot(&self) -> PricingSnapshot {
        self.pricing.read().expect("pricing lock poisoned").clone()
    }

    /// Atomically replace the pricing table.
    ///
    /// Called by the periodic refresh worker and the manual `POST
    /// /api/admin/pricing/reload` endpoint. A request that already read a table via
    /// [`AppState::pricing`] keeps using that `Arc` to completion — nothing is
    /// invalidated mid-request, so a price update can never change the bill for a request
    /// already in flight. See `docs/runbooks/pricing-update.md`.
    pub fn set_pricing(&self, table: PricingTable, source: PricingSource) {
        let mut guard = self.pricing.write().expect("pricing lock poisoned");
        *guard = PricingSnapshot {
            table: Arc::new(table),
            loaded_at: chrono::Utc::now(),
            source,
        };
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
            pricing: Arc::new(RwLock::new(PricingSnapshot::seed())),
            providers: Arc::new(ProviderRegistry::with_builtins()),
            key_cache: Arc::new(KeyCache::default()),
            health: Arc::new(ProviderHealth::new()),
            bandit: Arc::new(RoutingBandit::new()),
            shared_pool: Arc::new(SharedKeyPool::new()),
            http: reqwest::Client::new(),
            semantic_store: Arc::new(cache::semantic::MemoryVectorStore::new()),
            // Semantic caching is off by default in tests so every pre-existing test's
            // behaviour is unchanged; a test that wants to exercise it swaps this field.
            embedder: Arc::new(cache::embed::NullEmbedder),
            started_at: Instant::now(),
        }
    }
}

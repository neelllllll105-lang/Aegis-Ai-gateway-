//! Aegis gateway binary.
//!
//! Startup order matters and is deliberate:
//!
//! 1. Load and **validate** configuration. A bad master key fails the process here rather
//!    than encrypting a customer credential with something unusable at 3am.
//! 2. Initialise telemetry, including a redaction self-check.
//! 3. Connect the store and the database, run migrations, ensure usage partitions exist.
//!    A missing partition would reject inserts into the billing source of truth, so this
//!    happens *before* the listener opens.
//! 4. Start background workers.
//! 5. Only then bind the port and accept traffic.

use aegis_gateway::build_router;
use aegis_gateway::config::Config;
use aegis_gateway::engine::bandit::RoutingBandit;
use aegis_gateway::engine::fallback::ProviderHealth;
use aegis_gateway::metering::pricing::PricingTable;
use aegis_gateway::metrics::Metrics;
use aegis_gateway::middleware::auth::KeyCache;
use aegis_gateway::providers::pool::SharedKeyPool;
use aegis_gateway::providers::ProviderRegistry;
use aegis_gateway::store::{KvStore, MemoryStore, RedisStore};
use aegis_gateway::{db, telemetry, workers, AppState};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    // The container healthcheck runs this binary rather than requiring curl in the
    // runtime image. Keeping the image free of shell utilities is worth a few lines here.
    if std::env::args().any(|arg| arg == "--health-check") {
        std::process::exit(health_check().await);
    }

    if let Err(error) = run().await {
        // Startup failures print rather than log: tracing may not be initialised yet, and
        // an operator staring at a crashed container needs the reason on stderr.
        eprintln!("aegis-gateway failed to start: {error}");
        std::process::exit(1);
    }
}

/// Probe the local readiness endpoint. Returns a process exit code.
///
/// Deliberately checks `/ready`, not `/health`: an instance whose store is unreachable
/// should be drained by the load balancer, and the orchestrator should not restart a
/// process that is itself perfectly healthy.
async fn health_check() -> i32 {
    let bind = std::env::var("AEGIS_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let port = bind.rsplit(':').next().unwrap_or("8080");
    let url = format!("http://127.0.0.1:{port}/ready");

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(_) => return 1,
    };

    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => 0,
        Ok(response) => {
            eprintln!("health check: {url} returned {}", response.status());
            1
        }
        Err(e) => {
            eprintln!("health check: {url} unreachable: {e}");
            1
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    telemetry::init(&config);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        environment = ?config.environment,
        region = %config.region,
        "starting aegis gateway"
    );

    // ---- Store -----------------------------------------------------------------
    let store: Arc<dyn KvStore> = match &config.redis_url {
        Some(url) => {
            let store = RedisStore::connect(url).await?;
            store.ping().await?;
            tracing::info!("connected to redis");
            Arc::new(store)
        }
        None => {
            // `Config::validate` has already refused this combination outside
            // development, so reaching here means a deliberate local run.
            tracing::warn!(
                "REDIS_URL not set — using the in-process store. Rate limits and budgets \
                 will not hold across replicas."
            );
            Arc::new(MemoryStore::new())
        }
    };

    // ---- Database --------------------------------------------------------------
    let (db, db_replica, pricing) = match config.database_url.as_ref() {
        Some(_) => {
            let pool = db::pool::connect(&config).await?;
            db::pool::migrate(&pool).await?;

            // Before any traffic: a missing partition rejects usage inserts.
            let partitions = db::pool::maintain_partitions(&pool).await?;
            tracing::info!(?partitions, "usage partitions ready");

            // Database pricing is authoritative; seed data is only a bootstrap for an
            // empty table.
            let rows = db::repo::load_pricing(&pool).await?;
            let table = if rows.is_empty() {
                tracing::warn!(
                    "model_pricing is empty — falling back to seed data. Run \
                     scripts/seed.sql and verify prices before billing anyone."
                );
                PricingTable::with_seed_data()
            } else {
                tracing::info!(models = rows.len(), "loaded pricing from database");
                PricingTable::from_models(rows.into_iter().map(into_model).collect())
            };

            // Analytics replica, if one is configured. Reported explicitly at startup
            // so a deployment that meant to have one but typed the variable wrong is
            // visible in the first ten lines of the log rather than in a latency graph
            // three weeks later.
            let replica = db::pool::connect_replica(&config).await?;
            match &replica {
                Some(_) => tracing::info!("read replica connected — analytics will use it"),
                None => tracing::info!("no read replica configured — analytics use the primary"),
            }

            (Some(pool), replica, table)
        }
        None => {
            tracing::warn!(
                "DATABASE_URL not set — management endpoints will return an error and \
                 usage will not be persisted."
            );
            (None, None, PricingTable::with_seed_data())
        }
    };

    let state = AppState {
        config: Arc::clone(&config),
        store: Arc::clone(&store),
        metrics: Arc::new(Metrics::new()),
        db,
        db_replica,
        pricing: Arc::new(pricing),
        providers: Arc::new(ProviderRegistry::with_builtins()),
        key_cache: Arc::new(KeyCache::default()),
        health: Arc::new(ProviderHealth::new()),
        bandit: Arc::new(RoutingBandit::new()),
        shared_pool: Arc::new(SharedKeyPool::new()),
        http: build_http_client(&config)?,
        started_at: Instant::now(),
    };

    // ---- Background workers -----------------------------------------------------
    if state.db.is_some() {
        tokio::spawn(workers::usage_writer::run(state.clone()));
        tokio::spawn(workers::usage_writer::run_partition_maintenance(
            state.clone(),
        ));
        // Periodic jobs with external side effects. Safe to start on every replica: the
        // scheduler claims each run in the shared store, so exactly one replica acts.
        tokio::spawn(workers::scheduler::run(state.clone()));
        tracing::info!("background workers started");
    } else {
        tracing::warn!("workers not started: no database configured");
    }

    // ---- HTTP -------------------------------------------------------------------
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    tracing::info!(address = %config.bind_address, "listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("shutdown complete");
    Ok(())
}

/// Build the shared HTTP client.
///
/// One client for the whole process, so connections to each provider are pooled across
/// requests. A fresh client per request would add a TLS handshake to every upstream call
/// and blow the latency budget entirely.
fn build_http_client(config: &Config) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .pool_max_idle_per_host(32)
        .pool_idle_timeout(Duration::from_secs(90))
        .timeout(config.provider_timeout_reasoning)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("aegis-gateway/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Translate a database pricing row into the in-memory form.
fn into_model(row: db::repo::PricingRow) -> aegis_gateway::metering::pricing::ModelPricing {
    aegis_gateway::metering::pricing::ModelPricing {
        model_id: row.model_id,
        provider: row.provider,
        display_name: row.display_name,
        tier: aegis_gateway::types::ModelTier::parse(&row.tier),
        input_per_mtok: aegis_gateway::money::MicroCents(row.input_cost_per_mtok_mc),
        output_per_mtok: aegis_gateway::money::MicroCents(row.output_cost_per_mtok_mc),
        context_window: row.context_window.max(0) as u32,
        supports_tools: row.supports_tools,
        supports_vision: row.supports_vision,
        is_active: row.is_active,
        source: row.source,
    }
}

/// Wait for SIGINT or SIGTERM.
///
/// Graceful shutdown lets in-flight requests finish and, importantly, lets the usage
/// writer drain — killing it mid-batch would leave metered requests unpersisted until the
/// next start.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received interrupt"),
        _ = terminate => tracing::info!("received terminate"),
    }
}

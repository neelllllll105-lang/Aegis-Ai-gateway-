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

use aegis_gateway::config::Config;
use aegis_gateway::engine::bandit::RoutingBandit;
use aegis_gateway::engine::fallback::ProviderHealth;
use aegis_gateway::metering::pricing::PricingTable;
use aegis_gateway::metrics::Metrics;
use aegis_gateway::middleware::auth::KeyCache;
use aegis_gateway::middleware::security_headers;
use aegis_gateway::providers::pool::SharedKeyPool;
use aegis_gateway::providers::ProviderRegistry;
use aegis_gateway::routes::{admin, anthropic_compat, health, management, openai_compat};
use aegis_gateway::store::{KvStore, MemoryStore, RedisStore};
use aegis_gateway::{db, telemetry, workers, AppState};
use axum::routing::{delete, get, patch, post};
use axum::Router;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        // Startup failures print rather than log: tracing may not be initialised yet, and
        // an operator staring at a crashed container needs the reason on stderr.
        eprintln!("aegis-gateway failed to start: {error}");
        std::process::exit(1);
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
    let (db, pricing) = match config.database_url.as_ref() {
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

            (Some(pool), table)
        }
        None => {
            tracing::warn!(
                "DATABASE_URL not set — management endpoints will return an error and \
                 usage will not be persisted."
            );
            (None, PricingTable::with_seed_data())
        }
    };

    let state = AppState {
        config: Arc::clone(&config),
        store: Arc::clone(&store),
        metrics: Arc::new(Metrics::new()),
        db,
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

/// Assemble the full router.
pub fn build_router(state: AppState) -> Router {
    let max_body = state.config.max_body_bytes;
    let is_production = state.config.environment.is_production_like();

    let public = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/metrics", get(health::metrics))
        .route("/status", get(health::public_status));

    // The OpenAI- and Anthropic-compatible surfaces. These are what a customer points
    // their SDK at.
    let gateway = Router::new()
        .route(
            "/v1/chat/completions",
            post(openai_compat::chat_completions),
        )
        .route("/v1/models", get(openai_compat::list_models))
        .route("/v1/embeddings", post(openai_compat::embeddings))
        .route("/v1/messages", post(anthropic_compat::messages));

    let api = Router::new()
        .route("/api/auth/signup", post(management::signup))
        .route("/api/auth/login", post(management::login))
        .route("/api/auth/logout", post(management::logout))
        .route("/api/auth/me", get(management::me))
        .route(
            "/api/keys",
            get(management::list_keys).post(management::create_key),
        )
        .route("/api/keys/{id}", get(management::get_key))
        .route("/api/keys/{id}", patch(management::update_key))
        .route("/api/keys/{id}", delete(management::revoke_key))
        .route(
            "/api/org",
            get(management::get_org).patch(management::update_org),
        )
        .route("/api/org/members", get(management::list_members))
        .route("/api/org/members/invite", post(management::invite_member))
        .route("/api/org/members/{id}", delete(management::remove_member))
        .route(
            "/api/org/teams",
            get(management::list_teams).post(management::create_team),
        )
        .route("/api/org/teams/{id}", delete(management::delete_team))
        .route(
            "/api/providers",
            get(management::list_providers).post(management::create_provider),
        )
        .route("/api/providers/{id}", delete(management::delete_provider))
        .route("/api/providers/{id}/test", post(management::test_provider))
        .route(
            "/api/policies",
            get(management::list_policies).post(management::create_policy),
        )
        .route("/api/policies/{id}", delete(management::delete_policy))
        .route(
            "/api/budgets",
            get(management::list_budgets).post(management::create_budget),
        )
        .route("/api/budgets/{id}", delete(management::delete_budget))
        .route("/api/usage/summary", get(management::usage_summary))
        .route("/api/requests", get(management::list_requests))
        .route(
            "/api/savings/report.csv",
            get(management::savings_report_csv),
        )
        .route("/api/billing/plan", get(management::billing_plan));

    let admin_routes = Router::new()
        .route("/api/admin/metrics", get(admin::system_metrics))
        .route("/api/admin/routing", get(admin::routing_intelligence))
        .route("/api/admin/pricing", get(admin::pricing_table))
        .route("/api/admin/audit", get(admin::audit_log))
        .route(
            "/api/admin/providers/{provider}/reset",
            post(admin::reset_circuit),
        );

    Router::new()
        .merge(public)
        .merge(gateway)
        .merge(api)
        .merge(admin_routes)
        .layer(axum::middleware::from_fn(move |request, next| {
            security_headers_layer(request, next, is_production)
        }))
        // Part 9 item 7: a hard body cap, applied before any parsing.
        .layer(RequestBodyLimitLayer::new(max_body))
        .layer(TraceLayer::new_for_http())
        // The dashboard is served from a different origin, and credentials must be
        // allowed for the session cookie to travel.
        .layer(cors_layer(&state.config.app_url))
        .with_state(state)
}

/// Apply security headers to every response.
async fn security_headers_layer(
    request: axum::extract::Request,
    next: axum::middleware::Next,
    is_production: bool,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in security_headers::headers(is_production) {
        headers.insert(name, value);
    }
    response
}

/// CORS for the dashboard origin.
///
/// A specific origin rather than a wildcard: `Access-Control-Allow-Credentials` and `*`
/// are mutually exclusive, and the session cookie needs credentials.
fn cors_layer(app_url: &str) -> CorsLayer {
    match app_url.parse::<axum::http::HeaderValue>() {
        Ok(origin) => CorsLayer::new()
            .allow_origin(origin)
            .allow_credentials(true)
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderName::from_static("x-aegis-routing-hint"),
                axum::http::HeaderName::from_static("x-aegis-org"),
                axum::http::HeaderName::from_static("x-api-key"),
            ])
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ]),
        Err(_) => {
            tracing::warn!(app_url, "invalid AEGIS_APP_URL; CORS disabled");
            CorsLayer::new()
        }
    }
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

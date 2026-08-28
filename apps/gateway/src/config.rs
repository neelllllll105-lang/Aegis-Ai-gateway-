//! Typed environment configuration.
//!
//! All configuration arrives through environment variables (12-factor), is parsed once at
//! startup, and is then immutable. Invalid configuration fails the process at boot rather
//! than at the first request — a gateway that starts with a bad master key is worse than
//! one that refuses to start.

use crate::error::{AegisError, Result};
use std::env;
use std::time::Duration;

/// Deployment environment. Controls cookie flags, log format, and safety rails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Dev,
    Staging,
    Prod,
}

impl Environment {
    /// True in production-like environments, where we require TLS and real secrets.
    pub fn is_production_like(self) -> bool {
        matches!(self, Environment::Staging | Environment::Prod)
    }

    fn parse(raw: &str) -> Environment {
        match raw.to_ascii_lowercase().as_str() {
            "prod" | "production" => Environment::Prod,
            "staging" | "stage" => Environment::Staging,
            _ => Environment::Dev,
        }
    }
}

/// Complete gateway configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub environment: Environment,
    /// Address the HTTP server binds, e.g. `0.0.0.0:8080`.
    pub bind_address: String,
    /// Public base URL of this gateway, used in docs links and OAuth redirects.
    pub base_url: String,
    /// Public base URL of the dashboard.
    pub app_url: String,

    /// PostgreSQL connection string. `None` runs the gateway in stateless mode: the hot
    /// path still works (Redis-backed), but management endpoints that need persistence
    /// return 503. This is what makes the crate testable without a database.
    pub database_url: Option<String>,
    pub database_max_connections: u32,

    /// Redis connection string. `None` falls back to the in-process store, which is
    /// correct for a single instance and explicitly not for a cluster.
    pub redis_url: Option<String>,

    /// Qdrant REST base URL for the semantic cache (Phase 3). Still read as the fallback
    /// vector store when `qdrant_grpc_url` isn't set — see `docs/adr/0010-qdrant-grpc-client.md`.
    pub qdrant_url: Option<String>,
    /// Qdrant's gRPC endpoint (default port 6334, distinct from the REST port 6333) —
    /// deliberately a separate, explicit setting rather than derived by substituting the
    /// REST URL's port: Qdrant Cloud and some self-hosted setups front REST and gRPC on
    /// different hosts entirely, and a derived-and-wrong URL fails in a way that's much
    /// harder to notice than an unset one falling back to REST. `None` means "use REST."
    pub qdrant_grpc_url: Option<String>,

    /// 32-byte AES-256-GCM master key, base64-encoded. Encrypts BYOK provider
    /// credentials. Never logged, never stored in the database.
    pub master_key: [u8; 32],

    /// Pooled provider keys backing the free tier's shared models, by provider id.
    ///
    /// Populated from `SHARED_<PROVIDER>_KEYS` environment variables, each a
    /// comma-separated list. These are keys **we** pay for, so they are the only real
    /// cost a free-tier user imposes; the round-robin in
    /// [`crate::providers::pool::SharedKeyPool`] spreads load so no single key hits its
    /// own provider-side rate limit.
    pub shared_provider_keys: std::collections::HashMap<String, Vec<String>>,

    /// Resend API key for transactional email. `None` logs emails instead of sending.
    pub resend_api_key: Option<String>,
    pub email_from: String,

    /// Stripe secret key and webhook signing secret (Phase 4).
    pub stripe_secret_key: Option<String>,
    pub stripe_webhook_secret: Option<String>,

    /// Ed25519-style HMAC secret used to sign self-hosted licenses (Phase 6).
    pub license_signing_secret: Option<String>,

    /// Region identifier for data residency (Phase 6), e.g. `eu-central`.
    pub region: String,

    /// Optional read-replica connection string for analytics queries.
    ///
    /// Dashboard aggregates scan far more rows than the request path ever does, and a
    /// finance user pulling a year of history should not be able to slow down request
    /// authentication. When unset, analytics simply use the primary — a missing replica
    /// degrades performance, never correctness.
    pub read_replica_url: Option<String>,

    /// Hard ceiling on request body size. Part 9 item 7.
    pub max_body_bytes: usize,
    /// Hard ceiling on an upstream provider call.
    pub provider_timeout: Duration,
    /// Longer ceiling for reasoning models.
    pub provider_timeout_reasoning: Duration,
    /// Outer deadline on a whole request, applied at the router.
    ///
    /// Bounds the worst case that `provider_timeout` alone cannot: retries multiply it,
    /// and the fallback chain multiplies it again. Set above
    /// `provider_timeout_reasoning` or a reasoning model can never finish; the default of
    /// 180s leaves room for one 120s reasoning call plus a failover attempt.
    pub request_deadline: Duration,

    /// Default per-key rate limit, requests/minute.
    pub default_rate_limit_per_minute: u32,
    /// Exact-cache TTL.
    pub cache_ttl: Duration,
    /// Minimum cosine similarity for a semantic cache hit. Part 5 stage [5b].
    pub semantic_similarity_threshold: f32,
    /// How long a query that has been asked more than once stays in the durable
    /// (Postgres, encrypted) cache tier, sliding on every further hit.
    ///
    /// Deliberately separate from `cache_ttl`: the hot Redis tier is short and cheap so a
    /// one-off prompt doesn't linger; this tier exists only for queries the hot tier has
    /// already proven repeat, so it can safely last much longer without storing anything
    /// that was only ever asked once. See `docs/adr/0008-tiered-durable-cache.md`.
    pub durable_cache_ttl_days: i64,

    /// The token circuit breaker: the hard ceiling on a single request's `max_tokens`,
    /// enforced platform-wide before routing.
    ///
    /// Budget checking (`middleware::budget`) bounds *aggregate* spend over a period; it
    /// says nothing about one request. A prompt with no `max_tokens` set, or a reasoning
    /// model given free rein, can generate far more output than the 1/3-of-input estimate
    /// `budget::project_cost` uses to reserve against — the aggregate budget eventually
    /// catches up (the reservation is trued up after the fact), but a single request can
    /// still land as a large, surprising outlier before that happens. This closes that gap
    /// independently of budget: every request's effective `max_tokens` is clamped to this
    /// ceiling, unconditionally, regardless of remaining budget headroom.
    pub max_tokens_per_request: u32,

    /// Free-tier monthly request allowance.
    pub free_tier_monthly_requests: u64,
}

impl Config {
    /// Load configuration from the process environment.
    ///
    /// Fails when a required value is missing or malformed. In dev, a development master
    /// key is generated deterministically so a fresh clone runs with zero setup; in
    /// staging and production a real `AEGIS_MASTER_KEY` is mandatory.
    pub fn from_env() -> Result<Config> {
        let environment = Environment::parse(&opt("AEGIS_ENV").unwrap_or_default());

        let master_key = load_master_key(environment)?;

        let config = Config {
            environment,
            bind_address: opt("AEGIS_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_string()),
            base_url: opt("AEGIS_BASE_URL").unwrap_or_else(|| "http://localhost:8080".to_string()),
            app_url: opt("AEGIS_APP_URL").unwrap_or_else(|| "http://localhost:3000".to_string()),

            database_url: opt("DATABASE_URL"),
            database_max_connections: num("DATABASE_MAX_CONNECTIONS", 20)?,

            redis_url: opt("REDIS_URL"),
            qdrant_url: opt("QDRANT_URL"),
            qdrant_grpc_url: opt("QDRANT_GRPC_URL"),

            master_key,

            shared_provider_keys: load_shared_keys(),

            resend_api_key: opt("RESEND_API_KEY"),
            email_from: opt("AEGIS_EMAIL_FROM")
                .unwrap_or_else(|| "Aegis <noreply@aegis.dev>".into()),

            stripe_secret_key: opt("STRIPE_SECRET_KEY"),
            stripe_webhook_secret: opt("STRIPE_WEBHOOK_SECRET"),

            license_signing_secret: opt("AEGIS_LICENSE_SIGNING_SECRET"),

            region: opt("AEGIS_REGION").unwrap_or_else(|| "eu-central".to_string()),
            read_replica_url: opt("DATABASE_REPLICA_URL"),

            max_body_bytes: num::<usize>("AEGIS_MAX_BODY_BYTES", 10 * 1024 * 1024)?,
            provider_timeout: Duration::from_secs(num("AEGIS_PROVIDER_TIMEOUT_SECS", 30)?),
            provider_timeout_reasoning: Duration::from_secs(num(
                "AEGIS_PROVIDER_TIMEOUT_REASONING_SECS",
                120,
            )?),
            request_deadline: Duration::from_secs(num("AEGIS_REQUEST_DEADLINE_SECS", 180)?),

            default_rate_limit_per_minute: num("AEGIS_DEFAULT_RATE_LIMIT", 60)?,
            cache_ttl: Duration::from_secs(num("AEGIS_CACHE_TTL_SECS", 86_400)?),
            semantic_similarity_threshold: fnum("AEGIS_SEMANTIC_THRESHOLD", 0.95)?,
            durable_cache_ttl_days: num("AEGIS_DURABLE_CACHE_TTL_DAYS", 30)?,
            max_tokens_per_request: num("AEGIS_MAX_TOKENS_PER_REQUEST", 16_384)?,

            free_tier_monthly_requests: num("AEGIS_FREE_TIER_MONTHLY_REQUESTS", 10_000)?,
        };

        config.validate()?;
        Ok(config)
    }

    /// Configuration for tests: no external dependencies, deterministic key.
    pub fn for_tests() -> Config {
        Config {
            environment: Environment::Dev,
            bind_address: "127.0.0.1:0".to_string(),
            base_url: "http://localhost:8080".to_string(),
            app_url: "http://localhost:3000".to_string(),
            database_url: None,
            database_max_connections: 5,
            redis_url: None,
            qdrant_url: None,
            qdrant_grpc_url: None,
            master_key: [7u8; 32],
            shared_provider_keys: std::collections::HashMap::new(),
            resend_api_key: None,
            email_from: "Aegis <noreply@aegis.dev>".to_string(),
            stripe_secret_key: None,
            stripe_webhook_secret: None,
            license_signing_secret: Some("test-license-secret".to_string()),
            region: "test".to_string(),
            read_replica_url: None,
            max_body_bytes: 10 * 1024 * 1024,
            provider_timeout: Duration::from_secs(30),
            provider_timeout_reasoning: Duration::from_secs(120),
            request_deadline: Duration::from_secs(180),
            default_rate_limit_per_minute: 60,
            cache_ttl: Duration::from_secs(86_400),
            semantic_similarity_threshold: 0.95,
            durable_cache_ttl_days: 30,
            max_tokens_per_request: 16_384,
            free_tier_monthly_requests: 10_000,
        }
    }

    /// Reject configurations that are individually valid but collectively unsafe.
    fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.semantic_similarity_threshold) {
            return Err(AegisError::Config(
                "AEGIS_SEMANTIC_THRESHOLD must be between 0.0 and 1.0".into(),
            ));
        }
        if self.durable_cache_ttl_days < 1 {
            return Err(AegisError::Config(
                "AEGIS_DURABLE_CACHE_TTL_DAYS must be at least 1".into(),
            ));
        }
        if self.max_tokens_per_request == 0 {
            return Err(AegisError::Config(
                "AEGIS_MAX_TOKENS_PER_REQUEST must be at least 1".into(),
            ));
        }
        if self.environment.is_production_like() {
            if self.database_url.is_none() {
                return Err(AegisError::Config(
                    "DATABASE_URL is required outside development".into(),
                ));
            }
            if self.redis_url.is_none() {
                // Without Redis, rate limits and budgets are per-instance only, which
                // silently breaks enforcement the moment we run two replicas.
                return Err(AegisError::Config(
                    "REDIS_URL is required outside development: the in-process store \
                     cannot enforce limits across replicas"
                        .into(),
                ));
            }
            if !self.base_url.starts_with("https://") {
                return Err(AegisError::Config(
                    "AEGIS_BASE_URL must be https outside development".into(),
                ));
            }
        }
        Ok(())
    }

    /// Pooled keys for a provider, if any.
    pub fn shared_keys(&self, provider: &str) -> &[String] {
        self.shared_provider_keys
            .get(provider)
            .map(|keys| keys.as_slice())
            .unwrap_or(&[])
    }

    /// Whether session cookies get the `Secure` attribute.
    pub fn secure_cookies(&self) -> bool {
        self.environment.is_production_like()
    }
}

/// Load and decode the AES-256-GCM master key.
fn load_master_key(environment: Environment) -> Result<[u8; 32]> {
    use base64::Engine;

    match env::var("AEGIS_MASTER_KEY") {
        Ok(encoded) if !encoded.trim().is_empty() => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|_| AegisError::Config("AEGIS_MASTER_KEY must be valid base64".into()))?;
            let len = bytes.len();
            <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
                AegisError::Config(format!(
                    "AEGIS_MASTER_KEY must decode to exactly 32 bytes, got {len}"
                ))
            })
        }
        _ if environment.is_production_like() => Err(AegisError::Config(
            "AEGIS_MASTER_KEY is required outside development. Generate one with: \
             openssl rand -base64 32"
                .into(),
        )),
        // Development only: a fixed, obviously-fake key so a fresh clone runs immediately.
        // Anything encrypted with it is worthless, which is the point.
        _ => Ok(*b"aegis-development-key-do-not-use"),
    }
}

/// Collect `SHARED_<PROVIDER>_KEYS` for every provider we ship an adapter for.
fn load_shared_keys() -> std::collections::HashMap<String, Vec<String>> {
    let providers = [
        "openai",
        "anthropic",
        "google",
        "openrouter",
        "moonshot",
        "deepseek",
        "mistral",
        "groq",
        "custom",
    ];
    providers
        .iter()
        .filter_map(|provider| {
            let keys = list(&format!("SHARED_{}_KEYS", provider.to_uppercase()));
            (!keys.is_empty()).then(|| (provider.to_string(), keys))
        })
        .collect()
}

fn opt(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn list(key: &str) -> Vec<String> {
    opt(key)
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn num<T: std::str::FromStr>(key: &str, default: T) -> Result<T> {
    match opt(key) {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| AegisError::Config(format!("{key} must be a number, got {raw:?}"))),
    }
}

fn fnum(key: &str, default: f32) -> Result<f32> {
    num(key, default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_parsing_defaults_to_dev() {
        assert_eq!(Environment::parse("production"), Environment::Prod);
        assert_eq!(Environment::parse("PROD"), Environment::Prod);
        assert_eq!(Environment::parse("staging"), Environment::Staging);
        assert_eq!(Environment::parse(""), Environment::Dev);
        assert_eq!(Environment::parse("anything-else"), Environment::Dev);
    }

    #[test]
    fn production_requires_real_infrastructure() {
        let mut config = Config::for_tests();
        config.environment = Environment::Prod;
        config.base_url = "https://api.aegis.dev".into();

        // No database.
        assert!(config.validate().is_err());

        config.database_url = Some("postgres://localhost/aegis".into());
        // Still no Redis — limits would not hold across replicas.
        let err = config.validate().unwrap_err();
        assert!(format!("{err}").contains("REDIS_URL"), "{err}");

        config.redis_url = Some("redis://localhost".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn production_rejects_plaintext_base_url() {
        let mut config = Config::for_tests();
        config.environment = Environment::Prod;
        config.database_url = Some("postgres://localhost/aegis".into());
        config.redis_url = Some("redis://localhost".into());
        config.base_url = "http://api.aegis.dev".into();
        let err = config.validate().unwrap_err();
        assert!(format!("{err}").contains("https"), "{err}");
    }

    #[test]
    fn dev_runs_with_no_configuration_at_all() {
        let config = Config::for_tests();
        assert!(config.validate().is_ok());
        assert!(config.database_url.is_none());
        assert!(!config.secure_cookies());
    }

    #[test]
    fn similarity_threshold_is_bounded() {
        let mut config = Config::for_tests();
        config.semantic_similarity_threshold = 1.5;
        assert!(config.validate().is_err());
        config.semantic_similarity_threshold = -0.1;
        assert!(config.validate().is_err());
        config.semantic_similarity_threshold = 0.95;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn dev_master_key_is_exactly_32_bytes() {
        let key = load_master_key(Environment::Dev).expect("dev key");
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn production_refuses_the_development_master_key() {
        // With no AEGIS_MASTER_KEY set, production must fail rather than silently
        // encrypting every customer credential with a key published in this repository.
        let result = load_master_key(Environment::Prod);
        // The test process may or may not have the variable set; assert the contract for
        // the unset case only.
        if env::var("AEGIS_MASTER_KEY").is_err() {
            assert!(result.is_err());
        }
    }
}

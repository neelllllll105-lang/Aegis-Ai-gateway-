//! Shared provider key pool for the free tier.
//!
//! Free-tier requests run on keys **we** pay for, which makes this the only genuine COGS
//! a free user imposes. Two problems follow, and this module solves both:
//!
//! 1. **Provider-side rate limits are per key.** One key serving all free traffic would
//!    hit its own limit long before our capacity ran out, so requests are spread
//!    round-robin across a pool.
//! 2. **A burned key must not take the tier down.** Keys are selected by an atomic
//!    counter with no per-key state, so removing one from configuration is a restart, not
//!    a migration.

use crate::config::Config;
use crate::error::{AegisError, Result};
use crate::providers::Credential;
use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Round-robin selector over pooled keys.
#[derive(Debug, Default)]
pub struct SharedKeyPool {
    cursors: DashMap<String, AtomicUsize>,
}

impl SharedKeyPool {
    /// An empty pool.
    pub fn new() -> SharedKeyPool {
        SharedKeyPool::default()
    }

    /// Take the next key for a provider.
    ///
    /// `Relaxed` ordering is correct here: the counter only needs to advance, and two
    /// requests occasionally landing on the same key is harmless — the point is
    /// distribution, not exact fairness.
    pub fn next_credential(&self, config: &Config, provider: &str) -> Result<Credential> {
        let keys = config.shared_keys(provider);
        if keys.is_empty() {
            return Err(AegisError::Unauthorized(format!(
                "no credential available for {provider}. Add your own key on the \
                 providers page, or upgrade to a plan with BYOK."
            )));
        }

        let cursor = self
            .cursors
            .entry(provider.to_string())
            .or_insert_with(|| AtomicUsize::new(0));
        let index = cursor.fetch_add(1, Ordering::Relaxed) % keys.len();

        Ok(Credential::new(keys[index].clone()))
    }

    /// Whether any pooled key exists for a provider.
    pub fn has_keys(config: &Config, provider: &str) -> bool {
        !config.shared_keys(provider).is_empty()
    }

    /// Providers with pooled keys, sorted. For `/health` and the admin console.
    pub fn available_providers(config: &Config) -> Vec<String> {
        let mut providers: Vec<String> = config
            .shared_provider_keys
            .iter()
            .filter(|(_, keys)| !keys.is_empty())
            .map(|(provider, _)| provider.clone())
            .collect();
        providers.sort();
        providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(provider: &str, keys: &[&str]) -> Config {
        let mut config = Config::for_tests();
        config.shared_provider_keys.insert(
            provider.to_string(),
            keys.iter().map(|k| k.to_string()).collect(),
        );
        config
    }

    #[test]
    fn keys_are_handed_out_round_robin() {
        // The property that keeps any single key off its provider-side rate limit.
        let config = config_with("openai", &["key-a", "key-b", "key-c"]);
        let pool = SharedKeyPool::new();

        let selected: Vec<String> = (0..6)
            .map(|_| pool.next_credential(&config, "openai").unwrap().api_key)
            .collect();

        assert_eq!(
            selected,
            vec!["key-a", "key-b", "key-c", "key-a", "key-b", "key-c"]
        );
    }

    #[test]
    fn a_single_key_pool_always_returns_it() {
        let config = config_with("google", &["only-key"]);
        let pool = SharedKeyPool::new();
        for _ in 0..5 {
            assert_eq!(
                pool.next_credential(&config, "google").unwrap().api_key,
                "only-key"
            );
        }
    }

    #[test]
    fn providers_have_independent_cursors() {
        let mut config = config_with("openai", &["oa-1", "oa-2"]);
        config
            .shared_provider_keys
            .insert("google".into(), vec!["g-1".into(), "g-2".into()]);
        let pool = SharedKeyPool::new();

        assert_eq!(pool.next_credential(&config, "openai").unwrap().api_key, "oa-1");
        // Advancing one provider must not skip another forward.
        assert_eq!(pool.next_credential(&config, "google").unwrap().api_key, "g-1");
        assert_eq!(pool.next_credential(&config, "openai").unwrap().api_key, "oa-2");
        assert_eq!(pool.next_credential(&config, "google").unwrap().api_key, "g-2");
    }

    #[test]
    fn an_empty_pool_gives_an_actionable_error() {
        let pool = SharedKeyPool::new();
        let err = pool
            .next_credential(&Config::for_tests(), "openai")
            .unwrap_err();
        assert_eq!(err.error_type(), "unauthorized");
        // The message must tell the user what to do, not just that something is missing.
        let message = format!("{err}");
        assert!(message.contains("providers page"), "{message}");
        assert!(message.contains("upgrade"), "{message}");
    }

    #[test]
    fn availability_is_reported_per_provider() {
        let config = config_with("openai", &["k"]);
        assert!(SharedKeyPool::has_keys(&config, "openai"));
        assert!(!SharedKeyPool::has_keys(&config, "anthropic"));
        assert_eq!(SharedKeyPool::available_providers(&config), vec!["openai"]);
    }

    #[test]
    fn distribution_stays_even_under_concurrency() {
        use std::collections::HashMap;
        use std::sync::Arc;

        let config = Arc::new(config_with("openai", &["a", "b", "c", "d"]));
        let pool = Arc::new(SharedKeyPool::new());
        let counts = Arc::new(std::sync::Mutex::new(HashMap::<String, usize>::new()));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let config = Arc::clone(&config);
            let pool = Arc::clone(&pool);
            let counts = Arc::clone(&counts);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let key = pool.next_credential(&config, "openai").unwrap().api_key;
                    *counts.lock().unwrap().entry(key).or_insert(0) += 1;
                }
            }));
        }
        for handle in handles {
            let _ = handle.join();
        }

        let counts = counts.lock().unwrap();
        assert_eq!(counts.len(), 4, "every key should have been used");
        // 800 requests over 4 keys: perfectly even is 200 each. Relaxed ordering permits
        // some drift, but nothing close to a hot key.
        for (key, count) in counts.iter() {
            assert!(
                (150..=250).contains(count),
                "key {key} received {count} of 800 — distribution is skewed"
            );
        }
    }
}

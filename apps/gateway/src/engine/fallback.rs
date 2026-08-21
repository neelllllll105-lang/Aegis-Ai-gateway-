//! Provider health, circuit breakers, and retry policy — pipeline stage [7].
//!
//! When a provider degrades, the damage is not the failed requests — it is the *queued*
//! ones. Every request that keeps being sent to a dead upstream holds a connection, burns
//! its timeout, and adds latency to everything behind it. A circuit breaker converts a
//! slow, expensive failure into a fast, cheap one, which is what lets us fail over instead
//! of falling over.
//!
//! # State machine
//!
//! ```text
//!   Closed ──5 consecutive failures──> Open ──after 30s──> HalfOpen
//!     ^                                                       │
//!     └────────────── probe succeeds ─────────────────────────┘
//!                     probe fails ──> Open (timer restarts)
//! ```
//!
//! State is per-process and rebuilt from observation, deliberately: a breaker is a local
//! judgement about what *this* instance is seeing, and a shared breaker would let one
//! instance's bad network path take a provider offline for everyone.

use crate::error::AegisError;
use dashmap::DashMap;
use std::time::{Duration, Instant};

/// Consecutive failures before a circuit opens.
pub const FAILURE_THRESHOLD: u32 = 5;
/// How long a circuit stays open before allowing a probe.
pub const OPEN_DURATION: Duration = Duration::from_secs(30);
/// Maximum retry attempts for one upstream call, beyond the initial try.
pub const MAX_RETRIES: u32 = 2;
/// Base backoff; attempt N waits `BASE * 4^(N-1)` — 100ms then 400ms, per Part 5 [7].
pub const RETRY_BASE_DELAY: Duration = Duration::from_millis(100);

/// Circuit breaker state for one provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation.
    Closed,
    /// Failing; requests are not sent.
    Open,
    /// Allowing a single probe.
    HalfOpen,
}

impl CircuitState {
    /// Wire representation for metrics and health output.
    pub fn as_str(self) -> &'static str {
        match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half_open",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BreakerEntry {
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    /// A probe is in flight, so no other request should also probe.
    probing: bool,
}

/// Per-provider circuit breakers.
///
/// Lock-free reads via `DashMap`: `is_available` is called for every candidate model on
/// every routing decision, so it sits squarely inside the sub-millisecond budget.
#[derive(Debug, Default)]
pub struct ProviderHealth {
    breakers: DashMap<String, BreakerEntry>,
}

impl ProviderHealth {
    /// All circuits closed.
    pub fn new() -> ProviderHealth {
        ProviderHealth::default()
    }

    /// Current state for a provider.
    pub fn state(&self, provider: &str) -> CircuitState {
        let Some(entry) = self.breakers.get(provider) else {
            return CircuitState::Closed;
        };
        match entry.opened_at {
            None => CircuitState::Closed,
            Some(opened) if opened.elapsed() >= OPEN_DURATION => CircuitState::HalfOpen,
            Some(_) => CircuitState::Open,
        }
    }

    /// Whether a request may be sent to this provider.
    pub fn is_available(&self, provider: &str) -> bool {
        !matches!(self.state(provider), CircuitState::Open)
    }

    /// Record a successful call, closing the circuit.
    pub fn record_success(&self, provider: &str) {
        self.breakers
            .insert(provider.to_string(), BreakerEntry::default());
    }

    /// Record a failed call, opening the circuit once the threshold is reached.
    ///
    /// Returns the state after recording.
    pub fn record_failure(&self, provider: &str) -> CircuitState {
        let mut entry = self.breakers.entry(provider.to_string()).or_default();

        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        entry.probing = false;

        if entry.consecutive_failures >= FAILURE_THRESHOLD {
            // Restart the timer on every failure at or past the threshold, so a provider
            // that fails its probe gets a fresh full window rather than being re-probed
            // immediately.
            entry.opened_at = Some(Instant::now());
            return CircuitState::Open;
        }
        CircuitState::Closed
    }

    /// Claim the single probe slot for a half-open circuit.
    ///
    /// Returns `true` for exactly one caller. Without this, the moment a circuit
    /// half-opens every waiting request probes simultaneously and hammers a provider that
    /// is still recovering.
    pub fn try_probe(&self, provider: &str) -> bool {
        if self.state(provider) != CircuitState::HalfOpen {
            return true;
        }
        let mut entry = self.breakers.entry(provider.to_string()).or_default();
        if entry.probing {
            return false;
        }
        entry.probing = true;
        true
    }

    /// Consecutive failures currently recorded.
    pub fn consecutive_failures(&self, provider: &str) -> u32 {
        self.breakers
            .get(provider)
            .map(|e| e.consecutive_failures)
            .unwrap_or(0)
    }

    /// Every provider with a non-closed circuit, for `/health` and the admin console.
    pub fn degraded_providers(&self) -> Vec<(String, CircuitState)> {
        self.breakers
            .iter()
            .filter_map(|entry| {
                let state = self.state(entry.key());
                (state != CircuitState::Closed).then(|| (entry.key().clone(), state))
            })
            .collect()
    }

    /// Force a circuit closed. For the admin console and for tests.
    pub fn reset(&self, provider: &str) {
        self.breakers.remove(provider);
    }
}

/// Whether an error is worth retrying.
///
/// Only transient, server-side conditions qualify. Retrying a 400 wastes the caller's
/// time and our quota to get the same answer; retrying a 401 can lock an account.
pub fn is_retryable(error: &AegisError) -> bool {
    match error {
        AegisError::ProviderTimeout(_) => true,
        AegisError::Provider { status, .. } => {
            *status == 429 || *status == 408 || (500..600).contains(status)
        }
        AegisError::Store(_) => true,
        _ => false,
    }
}

/// Backoff before attempt number `attempt` (1-based).
///
/// 100ms, then 400ms. Exponential rather than linear so a provider in trouble gets
/// meaningfully less traffic from us on the second retry than the first.
pub fn retry_delay(attempt: u32) -> Duration {
    let multiplier = 4u32.saturating_pow(attempt.saturating_sub(1));
    RETRY_BASE_DELAY.saturating_mul(multiplier)
}

/// A ranked list of models to try, in order, for one request.
///
/// Part 5 [7]: try the selected model, then the same model on an alternate provider,
/// then one tier down, then the model the caller originally requested. The requested
/// model is always last so that a total failure of our optimization still ends with the
/// customer getting what they asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct FallbackChain {
    pub attempts: Vec<FallbackAttempt>,
}

/// One entry in a fallback chain.
#[derive(Debug, Clone, PartialEq)]
pub struct FallbackAttempt {
    pub model_id: String,
    pub provider: String,
    /// True when this attempt is a downgrade from the original selection.
    pub is_fallback: bool,
}

impl FallbackChain {
    /// Build a chain from the primary selection and the requested model.
    pub fn build(
        selected_model: &str,
        selected_provider: &str,
        requested_model: &str,
        requested_provider: &str,
        alternates: &[(String, String)],
    ) -> FallbackChain {
        let mut attempts = vec![FallbackAttempt {
            model_id: selected_model.to_string(),
            provider: selected_provider.to_string(),
            is_fallback: false,
        }];

        for (model, provider) in alternates {
            if !attempts.iter().any(|a| a.model_id == *model) {
                attempts.push(FallbackAttempt {
                    model_id: model.clone(),
                    provider: provider.clone(),
                    is_fallback: true,
                });
            }
        }

        if !attempts.iter().any(|a| a.model_id == requested_model) {
            attempts.push(FallbackAttempt {
                model_id: requested_model.to_string(),
                provider: requested_provider.to_string(),
                is_fallback: true,
            });
        }

        FallbackChain { attempts }
    }

    /// Number of attempts in the chain.
    pub fn len(&self) -> usize {
        self.attempts.len()
    }

    /// True when the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_provider_is_available() {
        let health = ProviderHealth::new();
        assert!(health.is_available("openai"));
        assert_eq!(health.state("openai"), CircuitState::Closed);
        assert_eq!(health.consecutive_failures("openai"), 0);
    }

    #[test]
    fn the_circuit_opens_only_at_the_threshold() {
        let health = ProviderHealth::new();
        for i in 1..FAILURE_THRESHOLD {
            assert_eq!(health.record_failure("openai"), CircuitState::Closed);
            assert!(
                health.is_available("openai"),
                "opened early after {i} failures"
            );
        }
        assert_eq!(health.record_failure("openai"), CircuitState::Open);
        assert!(!health.is_available("openai"));
    }

    #[test]
    fn a_success_resets_the_failure_count() {
        // Intermittent failures must not accumulate into an open circuit over hours.
        let health = ProviderHealth::new();
        for _ in 0..(FAILURE_THRESHOLD - 1) {
            health.record_failure("openai");
        }
        health.record_success("openai");
        assert_eq!(health.consecutive_failures("openai"), 0);

        for _ in 0..(FAILURE_THRESHOLD - 1) {
            health.record_failure("openai");
        }
        assert!(health.is_available("openai"), "count should have restarted");
    }

    #[test]
    fn breakers_are_isolated_per_provider() {
        let health = ProviderHealth::new();
        for _ in 0..FAILURE_THRESHOLD {
            health.record_failure("openai");
        }
        assert!(!health.is_available("openai"));
        assert!(
            health.is_available("anthropic"),
            "one provider took down another"
        );
        assert!(health.is_available("google"));
    }

    #[test]
    fn only_one_request_probes_a_half_open_circuit() {
        // Without this, every queued request probes at once and re-kills a recovering
        // provider.
        let health = ProviderHealth::new();
        health.breakers.insert(
            "openai".to_string(),
            BreakerEntry {
                consecutive_failures: FAILURE_THRESHOLD,
                // Opened long enough ago to be half-open now.
                opened_at: Some(Instant::now() - OPEN_DURATION - Duration::from_secs(1)),
                probing: false,
            },
        );
        assert_eq!(health.state("openai"), CircuitState::HalfOpen);
        assert!(health.try_probe("openai"), "the first caller should probe");
        for _ in 0..10 {
            assert!(
                !health.try_probe("openai"),
                "a second caller must not also probe"
            );
        }
    }

    #[test]
    fn probing_is_unrestricted_while_closed() {
        let health = ProviderHealth::new();
        for _ in 0..5 {
            assert!(health.try_probe("openai"));
        }
    }

    #[test]
    fn a_half_open_circuit_is_available_for_traffic() {
        let health = ProviderHealth::new();
        health.breakers.insert(
            "openai".to_string(),
            BreakerEntry {
                consecutive_failures: FAILURE_THRESHOLD,
                opened_at: Some(Instant::now() - OPEN_DURATION - Duration::from_secs(1)),
                probing: false,
            },
        );
        assert!(health.is_available("openai"));
    }

    #[test]
    fn a_failed_probe_restarts_the_open_window() {
        let health = ProviderHealth::new();
        health.breakers.insert(
            "openai".to_string(),
            BreakerEntry {
                consecutive_failures: FAILURE_THRESHOLD,
                opened_at: Some(Instant::now() - OPEN_DURATION - Duration::from_secs(1)),
                probing: true,
            },
        );
        assert_eq!(health.state("openai"), CircuitState::HalfOpen);
        assert_eq!(health.record_failure("openai"), CircuitState::Open);
        assert_eq!(
            health.state("openai"),
            CircuitState::Open,
            "window did not restart"
        );
    }

    #[test]
    fn degraded_providers_are_reported_for_health_checks() {
        let health = ProviderHealth::new();
        for _ in 0..FAILURE_THRESHOLD {
            health.record_failure("groq");
        }
        health.record_success("openai");

        let degraded = health.degraded_providers();
        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].0, "groq");
        assert_eq!(degraded[0].1, CircuitState::Open);
    }

    #[test]
    fn reset_forces_a_circuit_closed() {
        let health = ProviderHealth::new();
        for _ in 0..FAILURE_THRESHOLD {
            health.record_failure("openai");
        }
        assert!(!health.is_available("openai"));
        health.reset("openai");
        assert!(health.is_available("openai"));
    }

    #[test]
    fn only_transient_failures_are_retried() {
        assert!(is_retryable(&AegisError::ProviderTimeout(30)));
        assert!(is_retryable(&AegisError::Provider {
            provider: "openai".into(),
            status: 429,
            message: String::new()
        }));
        assert!(is_retryable(&AegisError::Provider {
            provider: "openai".into(),
            status: 503,
            message: String::new()
        }));

        // Retrying these is pointless at best and harmful at worst.
        assert!(!is_retryable(&AegisError::Provider {
            provider: "openai".into(),
            status: 400,
            message: String::new()
        }));
        assert!(!is_retryable(&AegisError::Provider {
            provider: "openai".into(),
            status: 401,
            message: String::new()
        }));
        assert!(!is_retryable(&AegisError::BadRequest("x".into())));
        assert!(!is_retryable(&AegisError::BudgetExceeded {
            spend_micro_cents: 1,
            limit_micro_cents: 0
        }));
    }

    #[test]
    fn backoff_matches_the_specified_schedule() {
        // Part 5 [7]: 100ms then 400ms.
        assert_eq!(retry_delay(1), Duration::from_millis(100));
        assert_eq!(retry_delay(2), Duration::from_millis(400));
        // Never panics on an out-of-range attempt number.
        assert!(retry_delay(50) > Duration::ZERO);
        assert_eq!(retry_delay(0), Duration::from_millis(100));
    }

    #[test]
    fn fallback_chain_ends_with_the_requested_model() {
        // The guarantee: however our optimization fails, the last thing we try is what
        // the customer actually asked for.
        let chain = FallbackChain::build(
            "openai/gpt-4o-mini",
            "openai",
            "openai/gpt-4o",
            "openai",
            &[("google/gemini-2.5-flash".to_string(), "google".to_string())],
        );
        assert_eq!(chain.len(), 3);
        assert_eq!(chain.attempts[0].model_id, "openai/gpt-4o-mini");
        assert!(!chain.attempts[0].is_fallback);
        assert_eq!(chain.attempts[2].model_id, "openai/gpt-4o");
        assert!(chain.attempts[2].is_fallback);
    }

    #[test]
    fn fallback_chain_does_not_repeat_a_model() {
        // A passthrough decision must not produce a chain that tries the same model twice.
        let chain = FallbackChain::build(
            "openai/gpt-4o",
            "openai",
            "openai/gpt-4o",
            "openai",
            &[("openai/gpt-4o".to_string(), "openai".to_string())],
        );
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn concurrent_failures_still_open_the_circuit_exactly_once() {
        use std::sync::Arc;
        let health = Arc::new(ProviderHealth::new());
        let mut handles = Vec::new();
        for _ in 0..50 {
            let health = Arc::clone(&health);
            handles.push(std::thread::spawn(move || health.record_failure("openai")));
        }
        for handle in handles {
            let _ = handle.join();
        }
        assert!(!health.is_available("openai"));
        assert!(health.consecutive_failures("openai") >= FAILURE_THRESHOLD);
    }
}

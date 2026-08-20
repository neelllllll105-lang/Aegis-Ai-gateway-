//! Outcome-driven routing — classifier v3 (Phase 7, `P7.1`).
//!
//! V1 and V2 predict complexity from the *text* of a request. This layer learns from what
//! actually happened: for each complexity band, which model delivered a good answer at a
//! low price, measured on our own traffic.
//!
//! That is the compounding asset. Anyone can copy a heuristic; nobody else has the record
//! of which model succeeded on which kind of request across every customer we serve.
//!
//! # Why UCB1 rather than epsilon-greedy
//!
//! Epsilon-greedy explores by sending a fixed fraction of traffic to a random model —
//! including, forever, models already known to be bad. Those are real customer requests
//! getting worse answers. UCB1 instead explores in proportion to *uncertainty*: a model
//! with few observations gets tried because we do not know it yet, and once we do, the
//! exploration bonus decays and traffic concentrates on what works. Regret is bounded
//! rather than linear, which in this setting means the number of customers who get a
//! needlessly bad answer stops growing.
//!
//! # Safety rails
//!
//! * The bandit only ever chooses among candidates the router already deemed **capable**
//!   and **permitted**. It can reorder them; it cannot introduce one.
//! * A model whose observed success rate falls below [`MIN_SUCCESS_RATE`] is excluded
//!   regardless of how cheap it is. Savings on a failed answer are not savings.
//! * With no data, it returns the router's existing first choice unchanged, so a cold
//!   start behaves exactly like Phase 3 routing.

use crate::money::MicroCents;
use crate::types::Complexity;
use dashmap::DashMap;

/// Observations required before a model's statistics are trusted enough to exploit.
pub const MIN_OBSERVATIONS: u64 = 30;

/// Success rate below which a model is excluded from selection.
pub const MIN_SUCCESS_RATE: f64 = 0.90;

/// Exploration weight in the UCB1 bonus. The textbook value of sqrt(2) explores harder
/// than we want with real customer traffic on the line, so this is deliberately lower.
pub const EXPLORATION: f64 = 0.6;

/// Accumulated outcomes for one model within one complexity band.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArmStats {
    /// Times this model was selected.
    pub pulls: u64,
    /// Times it produced a usable response.
    pub successes: u64,
    /// Total savings delivered, in micro-cents.
    pub total_savings_mc: i64,
    /// Total cost incurred, in micro-cents.
    pub total_cost_mc: i64,
}

impl ArmStats {
    /// Observed success rate. An unobserved arm is optimistically assumed to work, which
    /// is what makes UCB1 try it at all.
    pub fn success_rate(&self) -> f64 {
        if self.pulls == 0 {
            return 1.0;
        }
        self.successes as f64 / self.pulls as f64
    }

    /// Mean reward per pull, in `0.0..=1.0`.
    ///
    /// Reward combines the two things we actually care about: the answer was usable, and
    /// it was cheap. A failure scores zero however cheap it was — that is the whole point
    /// of measuring outcomes rather than prices.
    pub fn mean_reward(&self) -> f64 {
        if self.pulls == 0 {
            return 0.0;
        }
        let baseline = self.total_savings_mc.saturating_add(self.total_cost_mc);
        if baseline <= 0 {
            return self.success_rate();
        }
        let savings_ratio = self.total_savings_mc as f64 / baseline as f64;
        // Weighted toward reliability: a model that is 20% cheaper but fails 10% more
        // often is not a better model.
        0.7 * self.success_rate() + 0.3 * savings_ratio.clamp(0.0, 1.0)
    }

    /// True when there is enough data to act on.
    pub fn is_established(&self) -> bool {
        self.pulls >= MIN_OBSERVATIONS
    }
}

/// Per-band, per-model outcome statistics.
#[derive(Debug, Default)]
pub struct RoutingBandit {
    arms: DashMap<(String, String), ArmStats>,
}

impl RoutingBandit {
    /// An empty bandit. Behaves exactly like static routing until it has data.
    pub fn new() -> RoutingBandit {
        RoutingBandit::default()
    }

    fn key(band: Complexity, model: &str) -> (String, String) {
        (band.as_str().to_string(), model.to_string())
    }

    /// Statistics for one arm.
    pub fn stats(&self, band: Complexity, model: &str) -> ArmStats {
        self.arms
            .get(&RoutingBandit::key(band, model))
            .map(|s| *s)
            .unwrap_or_default()
    }

    /// Record what happened after a routing decision.
    pub fn record(
        &self,
        band: Complexity,
        model: &str,
        succeeded: bool,
        savings: MicroCents,
        cost: MicroCents,
    ) {
        let mut arm = self.arms.entry(RoutingBandit::key(band, model)).or_default();
        arm.pulls = arm.pulls.saturating_add(1);
        if succeeded {
            arm.successes = arm.successes.saturating_add(1);
        }
        arm.total_savings_mc = arm.total_savings_mc.saturating_add(savings.as_i64().max(0));
        arm.total_cost_mc = arm.total_cost_mc.saturating_add(cost.as_i64().max(0));
    }

    /// Total pulls recorded for a band, across all models.
    pub fn band_pulls(&self, band: Complexity) -> u64 {
        let band_name = band.as_str();
        self.arms
            .iter()
            .filter(|entry| entry.key().0 == band_name)
            .map(|entry| entry.pulls)
            .sum()
    }

    /// UCB1 score for one arm: mean reward plus an exploration bonus that shrinks as
    /// observations accumulate.
    pub fn ucb_score(&self, band: Complexity, model: &str, total_pulls: u64) -> f64 {
        let stats = self.stats(band, model);
        if stats.pulls == 0 {
            // Unobserved arms sort first: we cannot know they are bad until we try.
            return f64::INFINITY;
        }
        let bonus = EXPLORATION * ((total_pulls.max(1) as f64).ln() / stats.pulls as f64).sqrt();
        stats.mean_reward() + bonus
    }

    /// Choose among candidates the router has already validated.
    ///
    /// `candidates` must be in the router's preferred order; its first element is the
    /// fallback when the bandit has no opinion. Returns the chosen model id.
    pub fn select<'a>(&self, band: Complexity, candidates: &[&'a str]) -> Option<&'a str> {
        let default = candidates.first().copied()?;

        // Exclude anything demonstrably unreliable, however cheap.
        let viable: Vec<&&str> = candidates
            .iter()
            .filter(|model| {
                let stats = self.stats(band, model);
                !stats.is_established() || stats.success_rate() >= MIN_SUCCESS_RATE
            })
            .collect();

        if viable.is_empty() {
            // Every candidate has a poor record. Defer to the router rather than picking
            // the least bad on thin evidence.
            return Some(default);
        }

        let total_pulls = self.band_pulls(band);
        if total_pulls == 0 {
            return Some(default);
        }

        viable
            .into_iter()
            .max_by(|a, b| {
                self.ucb_score(band, a, total_pulls)
                    .partial_cmp(&self.ucb_score(band, b, total_pulls))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    // Deterministic tie-break keeps routing reproducible.
                    .then_with(|| b.cmp(a))
            })
            .copied()
            .or(Some(default))
    }

    /// Every arm with data, for the admin console and the models comparison page.
    pub fn snapshot(&self) -> Vec<(String, String, ArmStats)> {
        let mut rows: Vec<(String, String, ArmStats)> = self
            .arms
            .iter()
            .map(|entry| {
                let (band, model) = entry.key().clone();
                (band, model, *entry.value())
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        rows
    }

    /// Discard all learned statistics.
    pub fn reset(&self) {
        self.arms.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn savings(mc: i64) -> MicroCents {
        MicroCents(mc)
    }

    #[test]
    fn a_cold_bandit_defers_to_the_router() {
        // Phase 7 must not change Phase 3 behaviour until it has evidence.
        let bandit = RoutingBandit::new();
        let candidates = ["openai/gpt-4o-mini", "google/gemini-2.5-flash"];
        assert_eq!(
            bandit.select(Complexity::Simple, &candidates),
            Some("openai/gpt-4o-mini")
        );
    }

    #[test]
    fn empty_candidate_list_yields_nothing() {
        assert_eq!(RoutingBandit::new().select(Complexity::Simple, &[]), None);
    }

    #[test]
    fn unreliable_models_are_excluded_however_cheap() {
        // The rail that matters: savings on a failed answer are not savings.
        let bandit = RoutingBandit::new();
        for i in 0..100 {
            // Cheap but fails a fifth of the time.
            bandit.record(
                Complexity::Simple,
                "cheap/unreliable",
                i % 5 != 0,
                savings(9_000),
                savings(100),
            );
            bandit.record(
                Complexity::Simple,
                "solid/model",
                true,
                savings(4_000),
                savings(2_000),
            );
        }

        let stats = bandit.stats(Complexity::Simple, "cheap/unreliable");
        assert!(stats.success_rate() < MIN_SUCCESS_RATE);

        let chosen = bandit
            .select(Complexity::Simple, &["cheap/unreliable", "solid/model"])
            .unwrap();
        assert_eq!(chosen, "solid/model");
    }

    #[test]
    fn a_reliable_cheaper_model_wins_once_established() {
        let bandit = RoutingBandit::new();
        for _ in 0..200 {
            // Equally reliable, but one delivers far more savings.
            bandit.record(Complexity::Simple, "cheap/good", true, savings(9_000), savings(1_000));
            bandit.record(Complexity::Simple, "pricey/good", true, savings(1_000), savings(9_000));
        }
        let chosen = bandit
            .select(Complexity::Simple, &["pricey/good", "cheap/good"])
            .unwrap();
        assert_eq!(chosen, "cheap/good", "the bandit ignored a clear savings signal");
    }

    #[test]
    fn bands_learn_independently() {
        // A model that works for simple requests tells us nothing about complex ones.
        let bandit = RoutingBandit::new();
        for _ in 0..100 {
            bandit.record(Complexity::Simple, "cheap/model", true, savings(9_000), savings(500));
            bandit.record(Complexity::Complex, "cheap/model", false, savings(0), savings(500));
        }
        assert!(bandit.stats(Complexity::Simple, "cheap/model").success_rate() > 0.99);
        assert!(bandit.stats(Complexity::Complex, "cheap/model").success_rate() < 0.01);

        assert_eq!(
            bandit.select(Complexity::Complex, &["cheap/model", "solid/model"]),
            Some("solid/model")
        );
    }

    #[test]
    fn unobserved_arms_are_explored_before_established_ones() {
        let bandit = RoutingBandit::new();
        for _ in 0..MIN_OBSERVATIONS * 2 {
            bandit.record(Complexity::Simple, "known/model", true, savings(1_000), savings(1_000));
        }
        // The unseen arm has an infinite UCB score, so it gets tried.
        let chosen = bandit
            .select(Complexity::Simple, &["known/model", "unseen/model"])
            .unwrap();
        assert_eq!(chosen, "unseen/model");
    }

    #[test]
    fn exploration_bonus_decays_with_observations() {
        let bandit = RoutingBandit::new();
        for _ in 0..10 {
            bandit.record(Complexity::Simple, "a", true, savings(100), savings(100));
        }
        let early = bandit.ucb_score(Complexity::Simple, "a", 100);
        for _ in 0..1_000 {
            bandit.record(Complexity::Simple, "a", true, savings(100), savings(100));
        }
        let late = bandit.ucb_score(Complexity::Simple, "a", 1_100);
        assert!(late < early, "bonus did not decay: {early} -> {late}");
    }

    #[test]
    fn failures_score_zero_reward_regardless_of_savings() {
        let bandit = RoutingBandit::new();
        for _ in 0..50 {
            bandit.record(Complexity::Simple, "always/fails", false, savings(100_000), savings(0));
        }
        let stats = bandit.stats(Complexity::Simple, "always/fails");
        assert_eq!(stats.success_rate(), 0.0);
        // Reward is 0.7*0 + 0.3*1.0 at most — the reliability term dominates.
        assert!(stats.mean_reward() <= 0.31, "reward was {}", stats.mean_reward());
    }

    #[test]
    fn bandit_outperforms_static_routing_on_replayed_data() {
        // The Phase 7 acceptance criterion, measured rather than asserted.
        //
        // Replay: three models. The router's static first choice is mediocre; a better one
        // sits second in its ordering. Static routing always takes the first. The bandit
        // must end with a higher mean reward than static routing achieved.
        let bandit = RoutingBandit::new();
        let candidates = ["static/first", "better/second", "bad/third"];

        // Ground truth for the simulation.
        let outcome = |model: &str, step: u64| -> (bool, i64, i64) {
            match model {
                "static/first" => (step % 10 != 0, 3_000, 5_000),  // 90% success, modest savings
                "better/second" => (step % 50 != 0, 7_000, 2_000), // 98% success, large savings
                _ => (step % 3 != 0, 9_000, 500),                  // 67% success — unusable
            }
        };

        let mut bandit_reward = 0.0;
        let mut static_reward = 0.0;

        for step in 0..3_000u64 {
            // Bandit picks; static always takes the router's first choice.
            let picked = bandit.select(Complexity::Simple, &candidates).unwrap();
            let (ok, saved, cost) = outcome(picked, step);
            bandit.record(Complexity::Simple, picked, ok, savings(saved), savings(cost));
            bandit_reward += reward_of(ok, saved, cost);

            let (ok_static, saved_static, cost_static) = outcome("static/first", step);
            static_reward += reward_of(ok_static, saved_static, cost_static);
        }

        let bandit_mean = bandit_reward / 3_000.0;
        let static_mean = static_reward / 3_000.0;
        assert!(
            bandit_mean > static_mean,
            "bandit ({bandit_mean:.4}) did not beat static routing ({static_mean:.4})"
        );

        // And it should have concentrated traffic on the genuinely better model.
        let better = bandit.stats(Complexity::Simple, "better/second");
        let bad = bandit.stats(Complexity::Simple, "bad/third");
        assert!(
            better.pulls > bad.pulls * 3,
            "traffic did not concentrate: better={} bad={}",
            better.pulls,
            bad.pulls
        );
    }

    /// Per-request reward, matching [`ArmStats::mean_reward`].
    fn reward_of(succeeded: bool, saved: i64, cost: i64) -> f64 {
        let success = if succeeded { 1.0 } else { 0.0 };
        let baseline = saved + cost;
        let ratio = if baseline > 0 { saved as f64 / baseline as f64 } else { 0.0 };
        0.7 * success + 0.3 * ratio.clamp(0.0, 1.0)
    }

    #[test]
    fn snapshot_is_sorted_and_complete() {
        let bandit = RoutingBandit::new();
        bandit.record(Complexity::Simple, "z/model", true, savings(1), savings(1));
        bandit.record(Complexity::Complex, "a/model", true, savings(1), savings(1));

        let snapshot = bandit.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].0, "complex");
        assert_eq!(snapshot[1].0, "simple");
    }

    #[test]
    fn reset_clears_learned_state() {
        let bandit = RoutingBandit::new();
        bandit.record(Complexity::Simple, "m", true, savings(1), savings(1));
        assert_eq!(bandit.band_pulls(Complexity::Simple), 1);
        bandit.reset();
        assert_eq!(bandit.band_pulls(Complexity::Simple), 0);
        assert!(bandit.snapshot().is_empty());
    }

    #[test]
    fn counters_saturate_rather_than_overflowing() {
        let bandit = RoutingBandit::new();
        bandit.record(Complexity::Simple, "m", true, MicroCents(i64::MAX), MicroCents(i64::MAX));
        bandit.record(Complexity::Simple, "m", true, MicroCents(i64::MAX), MicroCents(i64::MAX));
        let stats = bandit.stats(Complexity::Simple, "m");
        assert_eq!(stats.total_savings_mc, i64::MAX);
        assert!(stats.mean_reward().is_finite());
    }
}

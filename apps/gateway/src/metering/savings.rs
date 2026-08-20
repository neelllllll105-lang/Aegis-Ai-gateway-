//! Savings attribution — the calculation the business model rests on.
//!
//! From `MASTER_BUILD.md` Part 0:
//!
//! ```text
//! baseline_cost = cost of serving on the model the user REQUESTED
//! actual_cost   = cost of serving on the model we actually USED (or $0 on a cache hit)
//! gross_savings = baseline_cost - actual_cost
//! our_fee       = gross_savings * savings_share_rate, ONLY when gross_savings > 0
//! customer_net  = gross_savings - our_fee
//! ```
//!
//! Two rules make this defensible to a customer's finance team, and both are enforced by
//! tests below:
//!
//! 1. **We never charge a fee on a loss.** If a routing decision costs more than the
//!    requested model would have, `gross_savings` floors at zero and the fee is zero. The
//!    overspend is ours to absorb, not to bill for.
//! 2. **No rounding before storage.** Every intermediate stays in micro-cents.

use crate::money::MicroCents;

/// A complete, auditable savings attribution for one request.
///
/// Every field is stored on the usage record and shown, unaggregated, in the customer's
/// savings dashboard. Transparency is the product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavingsBreakdown {
    /// What the requested model would have cost.
    pub baseline_cost: MicroCents,
    /// What we actually paid the provider. Zero on a cache hit.
    pub actual_cost: MicroCents,
    /// `baseline - actual`, floored at zero.
    pub gross_savings: MicroCents,
    /// Our share of the savings.
    pub aegis_fee: MicroCents,
    /// What the customer keeps: `gross_savings - aegis_fee`.
    pub customer_net: MicroCents,
}

impl SavingsBreakdown {
    /// Compute the attribution for one request.
    ///
    /// `savings_share_basis_points` comes from the org's plan
    /// ([`crate::money::savings_share_basis_points`]): 2000 = 20%.
    pub fn compute(
        baseline_cost: MicroCents,
        actual_cost: MicroCents,
        savings_share_basis_points: u32,
    ) -> SavingsBreakdown {
        let gross_savings = (baseline_cost - actual_cost).floor_at_zero();

        // The fee exists only when there is a real saving. A zero or negative delta
        // yields a zero fee, never a negative one.
        let aegis_fee = if gross_savings.is_zero() {
            MicroCents::ZERO
        } else {
            gross_savings.mul_basis_points(savings_share_basis_points)
        };

        SavingsBreakdown {
            baseline_cost,
            actual_cost,
            gross_savings,
            aegis_fee,
            customer_net: gross_savings - aegis_fee,
        }
    }

    /// Attribution for a request served straight through with no optimization: no
    /// savings, no fee.
    pub fn passthrough(cost: MicroCents) -> SavingsBreakdown {
        SavingsBreakdown {
            baseline_cost: cost,
            actual_cost: cost,
            gross_savings: MicroCents::ZERO,
            aegis_fee: MicroCents::ZERO,
            customer_net: MicroCents::ZERO,
        }
    }

    /// Attribution for a cache hit: the customer pays nothing, so the entire baseline is
    /// a saving.
    pub fn cache_hit(baseline_cost: MicroCents, savings_share_basis_points: u32) -> SavingsBreakdown {
        SavingsBreakdown::compute(baseline_cost, MicroCents::ZERO, savings_share_basis_points)
    }

    /// Savings as a percentage of baseline, for display only.
    pub fn savings_percent(&self) -> f64 {
        if self.baseline_cost.is_zero() {
            return 0.0;
        }
        (self.gross_savings.as_i64() as f64 / self.baseline_cost.as_i64() as f64) * 100.0
    }

    /// True when the routing decision cost more than the requested model would have.
    ///
    /// Not billable — but very much worth alerting on, because a routing rule that loses
    /// money systematically is a bug, and the customer will never see it in their bill.
    pub fn is_overspend(&self) -> bool {
        self.actual_cost > self.baseline_cost
    }

    /// How much a losing routing decision cost us, if any.
    pub fn overspend_amount(&self) -> MicroCents {
        (self.actual_cost - self.baseline_cost).floor_at_zero()
    }
}

/// Running totals across many requests, for dashboards and invoices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavingsTotals {
    pub requests: u64,
    pub baseline_cost: MicroCents,
    pub actual_cost: MicroCents,
    pub gross_savings: MicroCents,
    pub aegis_fee: MicroCents,
    pub customer_net: MicroCents,
    pub cache_hits: u64,
}

impl SavingsTotals {
    /// Fold one request into the totals.
    pub fn add(&mut self, breakdown: &SavingsBreakdown, cache_hit: bool) {
        self.requests += 1;
        self.baseline_cost += breakdown.baseline_cost;
        self.actual_cost += breakdown.actual_cost;
        self.gross_savings += breakdown.gross_savings;
        self.aegis_fee += breakdown.aegis_fee;
        self.customer_net += breakdown.customer_net;
        if cache_hit {
            self.cache_hits += 1;
        }
    }

    /// Cache hit rate as a percentage.
    pub fn cache_hit_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }
        (self.cache_hits as f64 / self.requests as f64) * 100.0
    }

    /// Overall savings percentage.
    pub fn savings_percent(&self) -> f64 {
        if self.baseline_cost.is_zero() {
            return 0.0;
        }
        (self.gross_savings.as_i64() as f64 / self.baseline_cost.as_i64() as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::savings_share_basis_points;

    const PRO: u32 = 2_000; // 20%

    #[test]
    fn worked_example_matches_the_business_model() {
        // Requested gpt-4o ($0.0075), served gpt-4o-mini ($0.00045).
        let breakdown = SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), PRO);
        assert_eq!(breakdown.gross_savings, MicroCents(7_050));
        assert_eq!(breakdown.aegis_fee, MicroCents(1_410)); // 20% of 7_050
        assert_eq!(breakdown.customer_net, MicroCents(5_640));
        // The three parts must reconstitute the whole, exactly.
        assert_eq!(breakdown.aegis_fee + breakdown.customer_net, breakdown.gross_savings);
    }

    #[test]
    fn cache_hit_saves_the_entire_baseline() {
        let breakdown = SavingsBreakdown::cache_hit(MicroCents(10_000), PRO);
        assert_eq!(breakdown.actual_cost, MicroCents::ZERO);
        assert_eq!(breakdown.gross_savings, MicroCents(10_000));
        assert_eq!(breakdown.aegis_fee, MicroCents(2_000));
        assert_eq!(breakdown.customer_net, MicroCents(8_000));
        assert_eq!(breakdown.savings_percent(), 100.0);
    }

    #[test]
    fn passthrough_produces_no_savings_and_no_fee() {
        let breakdown = SavingsBreakdown::passthrough(MicroCents(5_000));
        assert_eq!(breakdown.gross_savings, MicroCents::ZERO);
        assert_eq!(breakdown.aegis_fee, MicroCents::ZERO);
        assert_eq!(breakdown.customer_net, MicroCents::ZERO);
        assert!(!breakdown.is_overspend());
    }

    #[test]
    fn we_never_charge_a_fee_when_routing_cost_more() {
        // A routing decision that backfired: served model cost more than requested.
        // The customer must not be billed a "savings" fee on a loss.
        let breakdown = SavingsBreakdown::compute(MicroCents(1_000), MicroCents(5_000), PRO);
        assert_eq!(breakdown.gross_savings, MicroCents::ZERO);
        assert_eq!(breakdown.aegis_fee, MicroCents::ZERO);
        assert_eq!(breakdown.customer_net, MicroCents::ZERO);
        assert!(breakdown.is_overspend());
        assert_eq!(breakdown.overspend_amount(), MicroCents(4_000));
    }

    #[test]
    fn free_tier_pays_no_savings_share() {
        let breakdown = SavingsBreakdown::compute(
            MicroCents(10_000),
            MicroCents(1_000),
            savings_share_basis_points("free"),
        );
        assert_eq!(breakdown.gross_savings, MicroCents(9_000));
        assert_eq!(breakdown.aegis_fee, MicroCents::ZERO);
        assert_eq!(breakdown.customer_net, MicroCents(9_000));
    }

    #[test]
    fn every_plan_rate_produces_a_consistent_split() {
        for plan in ["free", "pro", "team", "enterprise", "api"] {
            let rate = savings_share_basis_points(plan);
            let breakdown = SavingsBreakdown::compute(MicroCents(1_000_000), MicroCents(100_000), rate);
            assert_eq!(
                breakdown.aegis_fee + breakdown.customer_net,
                breakdown.gross_savings,
                "split does not reconstitute for plan {plan}"
            );
            assert!(breakdown.aegis_fee <= breakdown.gross_savings, "fee exceeds savings on {plan}");
        }
    }

    #[test]
    fn identical_costs_yield_exactly_zero() {
        let breakdown = SavingsBreakdown::compute(MicroCents(4_242), MicroCents(4_242), PRO);
        assert_eq!(breakdown.gross_savings, MicroCents::ZERO);
        assert_eq!(breakdown.aegis_fee, MicroCents::ZERO);
    }

    #[test]
    fn zero_baseline_does_not_divide_by_zero() {
        let breakdown = SavingsBreakdown::compute(MicroCents::ZERO, MicroCents::ZERO, PRO);
        assert_eq!(breakdown.savings_percent(), 0.0);
        assert!(breakdown.savings_percent().is_finite());
    }

    #[test]
    fn property_fee_never_exceeds_savings_across_the_range() {
        // Exhaustive over a wide grid: the fee must never exceed the saving, the customer
        // net must never go negative, and the parts must always sum to the whole.
        for baseline in (0..=200_000).step_by(4_999) {
            for actual in (0..=200_000).step_by(7_919) {
                for rate in [0, 1_000, 1_500, 2_000, 10_000] {
                    let b = SavingsBreakdown::compute(
                        MicroCents(baseline),
                        MicroCents(actual),
                        rate,
                    );
                    assert!(b.gross_savings >= MicroCents::ZERO);
                    assert!(b.aegis_fee >= MicroCents::ZERO);
                    assert!(b.customer_net >= MicroCents::ZERO);
                    assert!(b.aegis_fee <= b.gross_savings);
                    assert_eq!(b.aegis_fee + b.customer_net, b.gross_savings);
                }
            }
        }
    }

    #[test]
    fn a_hundred_percent_rate_leaves_the_customer_nothing_but_stays_consistent() {
        // Not a real plan, but the maths must not break if one is ever configured.
        let b = SavingsBreakdown::compute(MicroCents(1_000), MicroCents::ZERO, 10_000);
        assert_eq!(b.aegis_fee, MicroCents(1_000));
        assert_eq!(b.customer_net, MicroCents::ZERO);
    }

    #[test]
    fn totals_accumulate_without_drift() {
        // The invoice property: ten thousand small requests must total exactly, with no
        // floating-point accumulation error.
        let mut totals = SavingsTotals::default();
        for _ in 0..10_000 {
            let b = SavingsBreakdown::compute(MicroCents(750), MicroCents(45), PRO);
            totals.add(&b, false);
        }
        assert_eq!(totals.requests, 10_000);
        assert_eq!(totals.baseline_cost, MicroCents(7_500_000));
        assert_eq!(totals.actual_cost, MicroCents(450_000));
        assert_eq!(totals.gross_savings, MicroCents(7_050_000));
        assert_eq!(totals.aegis_fee, MicroCents(1_410_000));
        assert_eq!(totals.customer_net, MicroCents(5_640_000));
        assert_eq!(totals.aegis_fee + totals.customer_net, totals.gross_savings);
    }

    #[test]
    fn totals_track_cache_hit_rate() {
        let mut totals = SavingsTotals::default();
        for i in 0..10 {
            let b = SavingsBreakdown::compute(MicroCents(100), MicroCents(10), PRO);
            totals.add(&b, i < 3);
        }
        assert_eq!(totals.cache_hits, 3);
        assert!((totals.cache_hit_rate() - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_totals_report_zero_rather_than_nan() {
        let totals = SavingsTotals::default();
        assert_eq!(totals.cache_hit_rate(), 0.0);
        assert_eq!(totals.savings_percent(), 0.0);
    }

    #[test]
    fn savings_percentage_is_accurate() {
        // 90% saving: $0.01 baseline down to $0.001.
        let b = SavingsBreakdown::compute(MicroCents(10_000), MicroCents(1_000), PRO);
        assert!((b.savings_percent() - 90.0).abs() < 0.0001);
    }

    #[test]
    fn breakdown_serializes_for_the_dashboard() {
        let b = SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), PRO);
        let json = serde_json::to_value(b).unwrap();
        // Money crosses the API as integer micro-cents; the client formats it.
        assert_eq!(json["gross_savings"], serde_json::json!(7_050));
        assert_eq!(json["aegis_fee"], serde_json::json!(1_410));
        assert!(json["customer_net"].is_i64());
    }
}

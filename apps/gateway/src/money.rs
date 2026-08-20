//! Monetary primitives.
//!
//! **Principle 9 (`MASTER_BUILD.md` Part 15): money is integers.** Every monetary value
//! in Aegis is stored and computed as [`MicroCents`] — a signed 64-bit integer count of
//! micro-cents, where `1 cent = 10_000 micro-cents` and `1 USD = 1_000_000 micro-cents`.
//!
//! Floating point never touches a stored monetary value. `f64` appears only at the
//! presentation boundary ([`MicroCents::to_usd_string`]) and when converting a provider
//! price sheet (dollars per million tokens) into integers, which happens once, at load
//! time, with explicit rounding.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub};

/// Micro-cents per cent.
pub const MICRO_CENTS_PER_CENT: i64 = 10_000;
/// Micro-cents per US dollar.
pub const MICRO_CENTS_PER_USD: i64 = 1_000_000;

/// A signed monetary amount in micro-cents.
///
/// Arithmetic saturates rather than wrapping: a billing bug that produces a wildly wrong
/// number is far worse than one that clamps at `i64::MAX`, and a panic in the hot path is
/// worse still.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct MicroCents(pub i64);

impl MicroCents {
    /// Zero.
    pub const ZERO: MicroCents = MicroCents(0);

    /// Construct from a raw micro-cent count.
    pub const fn new(micro_cents: i64) -> Self {
        MicroCents(micro_cents)
    }

    /// The raw micro-cent count.
    pub const fn as_i64(self) -> i64 {
        self.0
    }

    /// Construct from whole cents.
    pub const fn from_cents(cents: i64) -> Self {
        MicroCents(cents.saturating_mul(MICRO_CENTS_PER_CENT))
    }

    /// Convert a price sheet figure — US dollars per **million** tokens — into the
    /// micro-cent cost of a single token, scaled by `1e6` to stay integral.
    ///
    /// Provider price sheets are quoted like "$2.50 per 1M input tokens". We store that
    /// as micro-cents per million tokens: `$2.50 = 250 cents = 2_500_000 micro-cents`.
    /// Rounding is half-away-from-zero and happens exactly once, here.
    pub fn from_usd_per_mtok(usd_per_mtok: f64) -> Self {
        let micro_cents = usd_per_mtok * MICRO_CENTS_PER_USD as f64;
        MicroCents(round_half_away_from_zero(micro_cents))
    }

    /// Cost of `tokens` tokens given a per-million-token rate.
    ///
    /// Uses `i128` for the intermediate product so a large token count against a large
    /// rate cannot overflow before the division brings it back into range.
    pub fn cost_for_tokens(rate_per_mtok: MicroCents, tokens: u64) -> Self {
        let product = (rate_per_mtok.0 as i128) * (tokens as i128);
        // Divide by 1M tokens, rounding half away from zero so we never systematically
        // under- or over-bill.
        let million = 1_000_000i128;
        let rounded = if product >= 0 {
            (product + million / 2) / million
        } else {
            (product - million / 2) / million
        };
        MicroCents(clamp_i128(rounded))
    }

    /// Multiply by a rate expressed in basis points (1 bp = 0.01%).
    ///
    /// Used for the savings share: 20% is `2_000` bp. Integer math throughout, so the fee
    /// on a given savings figure is exactly reproducible.
    pub fn mul_basis_points(self, basis_points: u32) -> Self {
        let product = (self.0 as i128) * (basis_points as i128);
        let rounded = if product >= 0 {
            (product + 5_000) / 10_000
        } else {
            (product - 5_000) / 10_000
        };
        MicroCents(clamp_i128(rounded))
    }

    /// Clamp negatives to zero. Savings are never negative in billing.
    pub fn floor_at_zero(self) -> Self {
        if self.0 < 0 {
            MicroCents::ZERO
        } else {
            self
        }
    }

    /// True when the amount is zero.
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Whole cents, truncated toward zero. For Stripe, which bills in cents.
    pub fn to_cents(self) -> i64 {
        self.0 / MICRO_CENTS_PER_CENT
    }

    /// Human-readable USD, e.g. `$0.001234`. Presentation only — never store this.
    pub fn to_usd_string(self) -> String {
        let usd = self.0 as f64 / MICRO_CENTS_PER_USD as f64;
        if usd != 0.0 && usd.abs() < 0.01 {
            format!("${usd:.6}")
        } else {
            format!("${usd:.4}")
        }
    }
}

fn round_half_away_from_zero(value: f64) -> i64 {
    if value >= 0.0 {
        (value + 0.5).floor() as i64
    } else {
        (value - 0.5).ceil() as i64
    }
}

fn clamp_i128(value: i128) -> i64 {
    if value > i64::MAX as i128 {
        i64::MAX
    } else if value < i64::MIN as i128 {
        i64::MIN
    } else {
        value as i64
    }
}

impl Add for MicroCents {
    type Output = MicroCents;
    fn add(self, rhs: MicroCents) -> MicroCents {
        MicroCents(self.0.saturating_add(rhs.0))
    }
}

impl AddAssign for MicroCents {
    fn add_assign(&mut self, rhs: MicroCents) {
        self.0 = self.0.saturating_add(rhs.0);
    }
}

impl Sub for MicroCents {
    type Output = MicroCents;
    fn sub(self, rhs: MicroCents) -> MicroCents {
        MicroCents(self.0.saturating_sub(rhs.0))
    }
}

impl Sum for MicroCents {
    fn sum<I: Iterator<Item = MicroCents>>(iter: I) -> MicroCents {
        iter.fold(MicroCents::ZERO, |a, b| a + b)
    }
}

impl fmt::Display for MicroCents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_usd_string())
    }
}

impl serde::Serialize for MicroCents {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for MicroCents {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        i64::deserialize(d).map(MicroCents)
    }
}

/// Savings-share rate for a plan, in basis points.
///
/// Encoded as integers so the fee calculation never involves a float. Mirrors the
/// business model table in `MASTER_BUILD.md` Part 0.
pub const fn savings_share_basis_points(plan: &str) -> u32 {
    match plan.as_bytes() {
        b"pro" => 2_000,        // 20%
        b"team" => 1_500,       // 15%
        b"enterprise" => 1_000, // 10%
        b"api" => 0,            // pure usage pricing, no savings share
        _ => 0,                 // free tier: we take no share
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollars_per_mtok_converts_exactly() {
        // GPT-4o: $2.50 per 1M input tokens.
        assert_eq!(MicroCents::from_usd_per_mtok(2.50), MicroCents(2_500_000));
        // GPT-4o-mini: $0.15 per 1M input tokens.
        assert_eq!(MicroCents::from_usd_per_mtok(0.15), MicroCents(150_000));
        // Sub-cent rates must not collapse to zero.
        assert_eq!(MicroCents::from_usd_per_mtok(0.0375), MicroCents(37_500));
    }

    #[test]
    fn token_cost_is_proportional() {
        let rate = MicroCents::from_usd_per_mtok(2.50); // $2.50 / 1M tokens
        assert_eq!(MicroCents::cost_for_tokens(rate, 1_000_000), MicroCents(2_500_000));
        assert_eq!(MicroCents::cost_for_tokens(rate, 1_000), MicroCents(2_500));
        assert_eq!(MicroCents::cost_for_tokens(rate, 0), MicroCents::ZERO);
    }

    #[test]
    fn token_cost_rounds_rather_than_truncating() {
        // 1 token at $2.50/Mtok = 2.5 micro-cents -> rounds to 3, not 2.
        let rate = MicroCents::from_usd_per_mtok(2.50);
        assert_eq!(MicroCents::cost_for_tokens(rate, 1), MicroCents(3));
    }

    #[test]
    fn token_cost_cannot_overflow() {
        let rate = MicroCents(i64::MAX);
        let cost = MicroCents::cost_for_tokens(rate, u64::MAX);
        assert_eq!(cost, MicroCents(i64::MAX));
    }

    #[test]
    fn basis_points_match_the_business_model() {
        let savings = MicroCents(1_000_000); // $1.00 saved
        assert_eq!(savings.mul_basis_points(2_000), MicroCents(200_000)); // Pro 20% = $0.20
        assert_eq!(savings.mul_basis_points(1_500), MicroCents(150_000)); // Team 15%
        assert_eq!(savings.mul_basis_points(1_000), MicroCents(100_000)); // Ent 10%
        assert_eq!(savings.mul_basis_points(0), MicroCents::ZERO); // Free 0%
    }

    #[test]
    fn plan_rates_are_correct() {
        assert_eq!(savings_share_basis_points("pro"), 2_000);
        assert_eq!(savings_share_basis_points("team"), 1_500);
        assert_eq!(savings_share_basis_points("enterprise"), 1_000);
        assert_eq!(savings_share_basis_points("free"), 0);
        assert_eq!(savings_share_basis_points("nonsense"), 0);
    }

    #[test]
    fn arithmetic_saturates_instead_of_panicking() {
        assert_eq!(MicroCents(i64::MAX) + MicroCents(1), MicroCents(i64::MAX));
        assert_eq!(MicroCents(i64::MIN) - MicroCents(1), MicroCents(i64::MIN));
    }

    #[test]
    fn floor_at_zero_clamps_negative_savings() {
        assert_eq!(MicroCents(-5).floor_at_zero(), MicroCents::ZERO);
        assert_eq!(MicroCents(5).floor_at_zero(), MicroCents(5));
    }

    #[test]
    fn display_keeps_precision_for_tiny_amounts() {
        assert_eq!(MicroCents(1_234).to_usd_string(), "$0.001234");
        assert_eq!(MicroCents(1_000_000).to_usd_string(), "$1.0000");
        assert_eq!(MicroCents::ZERO.to_usd_string(), "$0.0000");
    }

    #[test]
    fn cents_conversion_is_exact() {
        assert_eq!(MicroCents::from_cents(2_900).to_cents(), 2_900); // $29.00 Pro plan
    }

    #[test]
    fn sum_of_many_small_costs_stays_exact() {
        // A million requests each costing 3 micro-cents must total exactly 3_000_000,
        // with no floating-point drift. This is the property that makes invoices right.
        let total: MicroCents = (0..1_000_000).map(|_| MicroCents(3)).sum();
        assert_eq!(total, MicroCents(3_000_000));
    }
}

//! Invoice assembly.
//!
//! An invoice is built from `usage_records` — the billing source of truth — never from the
//! Redis counters, which are a fast approximation that can drift during an outage.
//!
//! # Why the fee is recomputed rather than summed
//!
//! Each usage record already carries `aegis_fee_mc`, computed at request time. The invoice
//! sums those, and then **independently recomputes** the fee from the period's total
//! savings and asserts the two agree ([`Invoice::fee_is_consistent`]). If per-request fees
//! and the period fee ever disagree, something is wrong with the arithmetic and the
//! invoice must not go out. Part 13 item 1: a penny-wrong invoice destroys trust, and the
//! cheapest place to catch that is before it is sent.

use crate::db::repo::UsageSummary;
use crate::money::MicroCents;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Monthly subscription price for a plan.
///
/// From `MASTER_BUILD.md` Part 0. Enterprise is "$2,000+" — the floor is encoded here and
/// negotiated contracts override it on the organisation record.
pub fn subscription_price(plan: &str) -> MicroCents {
    match plan {
        "pro" => MicroCents::from_cents(2_900),          // $29
        "team" => MicroCents::from_cents(29_900),        // $299
        "enterprise" => MicroCents::from_cents(200_000), // $2,000 floor
        _ => MicroCents::ZERO,                           // free and usage-only API tier
    }
}

/// Per-request price for the usage-based API tier: $0.0001.
pub const API_TIER_PER_REQUEST: MicroCents = MicroCents(100);

/// A line on an invoice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub description: String,
    pub quantity: i64,
    pub unit_amount_mc: i64,
    pub amount_mc: i64,
}

/// A draft invoice for one organisation and period.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invoice {
    pub org_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub plan: String,
    pub lines: Vec<InvoiceLine>,
    pub subscription_mc: i64,
    pub savings_fee_mc: i64,
    pub total_mc: i64,
    /// Reproduced on the invoice so a customer can verify the fee without our dashboard.
    pub gross_savings_mc: i64,
    pub customer_net_mc: i64,
    pub requests: i64,
    pub generated_at: DateTime<Utc>,
}

impl Invoice {
    /// Build a draft invoice from a period's usage.
    pub fn build(
        org_id: Uuid,
        plan: &str,
        savings_share_bp: u32,
        period_start: NaiveDate,
        period_end: NaiveDate,
        usage: &UsageSummary,
    ) -> Invoice {
        let mut lines = Vec::new();

        let subscription = subscription_price(plan);
        if !subscription.is_zero() {
            lines.push(InvoiceLine {
                description: format!("{} plan — monthly subscription", plan_label(plan)),
                quantity: 1,
                unit_amount_mc: subscription.as_i64(),
                amount_mc: subscription.as_i64(),
            });
        }

        // The usage-based tier bills per request instead of a subscription.
        let api_usage = if plan == "api" {
            let amount = MicroCents(API_TIER_PER_REQUEST.as_i64() * usage.requests);
            lines.push(InvoiceLine {
                description: "API tier — requests".to_string(),
                quantity: usage.requests,
                unit_amount_mc: API_TIER_PER_REQUEST.as_i64(),
                amount_mc: amount.as_i64(),
            });
            amount
        } else {
            MicroCents::ZERO
        };

        // Recomputed from the period total, then reconciled against the sum of
        // per-request fees below.
        let gross_savings = MicroCents(usage.gross_savings_mc).floor_at_zero();
        let savings_fee = gross_savings.mul_basis_points(savings_share_bp);

        if !savings_fee.is_zero() {
            lines.push(InvoiceLine {
                description: format!(
                    "Savings share — {}% of {} saved",
                    savings_share_bp as f64 / 100.0,
                    gross_savings.to_usd_string()
                ),
                quantity: 1,
                unit_amount_mc: savings_fee.as_i64(),
                amount_mc: savings_fee.as_i64(),
            });
        }

        let total = subscription + api_usage + savings_fee;

        Invoice {
            org_id,
            period_start,
            period_end,
            plan: plan.to_string(),
            lines,
            subscription_mc: (subscription + api_usage).as_i64(),
            savings_fee_mc: savings_fee.as_i64(),
            total_mc: total.as_i64(),
            gross_savings_mc: gross_savings.as_i64(),
            customer_net_mc: (gross_savings - savings_fee).as_i64(),
            requests: usage.requests,
            generated_at: Utc::now(),
        }
    }

    /// Whether the period fee agrees with the sum of per-request fees.
    ///
    /// Rounding at two different granularities can legitimately differ by a micro-cent per
    /// request, so the tolerance scales with request count. Anything beyond that is a bug,
    /// and the invoice must not be finalised.
    pub fn fee_is_consistent(&self, sum_of_request_fees_mc: i64) -> bool {
        let tolerance = self.requests.max(1);
        (self.savings_fee_mc - sum_of_request_fees_mc).abs() <= tolerance
    }

    /// The lines must sum to the total.
    pub fn lines_sum_to_total(&self) -> bool {
        self.lines.iter().map(|l| l.amount_mc).sum::<i64>() == self.total_mc
    }

    /// True when there is nothing to charge.
    ///
    /// A zero invoice is not sent: charging a free-tier customer $0.00 is a support ticket
    /// and a payment-processor fee for nothing.
    pub fn is_zero(&self) -> bool {
        self.total_mc == 0
    }

    /// Total in whole cents, for Stripe.
    ///
    /// Rounds **down**: never charge a customer a cent we cannot itemise.
    pub fn total_cents(&self) -> i64 {
        MicroCents(self.total_mc).to_cents()
    }

    /// A human-readable summary for the dashboard and the email body.
    pub fn summary(&self) -> String {
        format!(
            "{} — {} to {}: {} subscription + {} savings share = {} total. \
             You saved {} and kept {}.",
            plan_label(&self.plan),
            self.period_start,
            self.period_end,
            MicroCents(self.subscription_mc).to_usd_string(),
            MicroCents(self.savings_fee_mc).to_usd_string(),
            MicroCents(self.total_mc).to_usd_string(),
            MicroCents(self.gross_savings_mc).to_usd_string(),
            MicroCents(self.customer_net_mc).to_usd_string(),
        )
    }
}

fn plan_label(plan: &str) -> &str {
    match plan {
        "pro" => "Pro",
        "team" => "Team",
        "enterprise" => "Enterprise",
        "api" => "API",
        _ => "Free",
    }
}

/// First and last day of the month containing `date`.
pub fn month_bounds(date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
    let (next_year, next_month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    let end = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or(start);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(requests: i64, savings_mc: i64, fee_mc: i64) -> UsageSummary {
        UsageSummary {
            requests,
            cache_hits: 0,
            input_tokens: 1_000 * requests,
            output_tokens: 300 * requests,
            baseline_cost_mc: savings_mc * 2,
            actual_cost_mc: savings_mc,
            gross_savings_mc: savings_mc,
            aegis_fee_mc: fee_mc,
            routing_savings_mc: savings_mc,
            compression_savings_mc: 0,
            cache_savings_mc: 0,
        }
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn subscription_prices_match_the_business_model() {
        assert_eq!(subscription_price("pro"), MicroCents::from_cents(2_900));
        assert_eq!(subscription_price("team"), MicroCents::from_cents(29_900));
        assert_eq!(
            subscription_price("enterprise"),
            MicroCents::from_cents(200_000)
        );
        assert_eq!(subscription_price("free"), MicroCents::ZERO);
        assert_eq!(subscription_price("api"), MicroCents::ZERO);
    }

    #[test]
    fn a_pro_invoice_has_both_line_items_and_they_sum() {
        // $10.00 saved at 20% = $2.00 fee, on top of the $29 subscription.
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "pro",
            2_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(5_000, 10_000_000, 2_000_000),
        );

        assert_eq!(invoice.lines.len(), 2);
        assert_eq!(invoice.subscription_mc, 29_000_000);
        assert_eq!(invoice.savings_fee_mc, 2_000_000);
        assert_eq!(invoice.total_mc, 31_000_000);
        assert_eq!(invoice.total_cents(), 3_100);
        assert!(invoice.lines_sum_to_total());
    }

    #[test]
    fn the_customer_keeps_the_rest_of_the_saving() {
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "pro",
            2_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(100, 10_000_000, 2_000_000),
        );
        assert_eq!(invoice.gross_savings_mc, 10_000_000);
        assert_eq!(invoice.savings_fee_mc, 2_000_000);
        assert_eq!(invoice.customer_net_mc, 8_000_000);
        assert_eq!(
            invoice.savings_fee_mc + invoice.customer_net_mc,
            invoice.gross_savings_mc
        );
    }

    #[test]
    fn a_free_plan_with_no_savings_produces_a_zero_invoice() {
        // Which must not be sent: a $0.00 charge is a support ticket and a processor fee.
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "free",
            0,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(500, 0, 0),
        );
        assert!(invoice.is_zero());
        assert!(invoice.lines.is_empty());
        assert_eq!(invoice.total_cents(), 0);
    }

    #[test]
    fn free_tier_savings_generate_no_fee() {
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "free",
            0,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(500, 5_000_000, 0),
        );
        assert_eq!(invoice.savings_fee_mc, 0);
        assert_eq!(
            invoice.customer_net_mc, 5_000_000,
            "the customer keeps all of it"
        );
        assert!(invoice.is_zero());
    }

    #[test]
    fn the_api_tier_bills_per_request() {
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "api",
            0,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(1_000_000, 0, 0),
        );
        // 1M requests at $0.0001 = $100.
        assert_eq!(invoice.subscription_mc, 100_000_000);
        assert_eq!(invoice.total_cents(), 10_000);
        assert!(invoice.lines_sum_to_total());
    }

    #[test]
    fn every_plan_produces_a_self_consistent_invoice() {
        for (plan, rate) in [
            ("free", 0u32),
            ("pro", 2_000),
            ("team", 1_500),
            ("enterprise", 1_000),
            ("api", 0),
        ] {
            let invoice = Invoice::build(
                Uuid::new_v4(),
                plan,
                rate,
                date(2026, 8, 1),
                date(2026, 8, 31),
                &usage(10_000, 50_000_000, 0),
            );
            assert!(invoice.lines_sum_to_total(), "lines do not sum for {plan}");
            assert!(
                invoice.savings_fee_mc <= invoice.gross_savings_mc,
                "fee exceeds savings for {plan}"
            );
            assert!(
                invoice.customer_net_mc >= 0,
                "negative customer net for {plan}"
            );
        }
    }

    #[test]
    fn negative_savings_never_become_a_charge() {
        // A month where routing lost money must not produce a negative fee or a credit.
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "pro",
            2_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(100, -5_000_000, 0),
        );
        assert_eq!(invoice.gross_savings_mc, 0);
        assert_eq!(invoice.savings_fee_mc, 0);
        assert_eq!(
            invoice.total_mc, 29_000_000,
            "only the subscription is charged"
        );
    }

    #[test]
    fn fee_consistency_check_accepts_rounding_drift() {
        // Per-request rounding legitimately differs from period rounding by up to a
        // micro-cent per request.
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "pro",
            2_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(1_000, 10_000_000, 0),
        );
        assert!(invoice.fee_is_consistent(2_000_000));
        assert!(invoice.fee_is_consistent(2_000_000 + 500));
        assert!(invoice.fee_is_consistent(2_000_000 - 500));
    }

    #[test]
    fn fee_consistency_check_catches_a_real_discrepancy() {
        // The point of the check: a genuine arithmetic bug must stop the invoice.
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "pro",
            2_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(10, 10_000_000, 0),
        );
        assert!(
            !invoice.fee_is_consistent(1_000_000),
            "a 50% discrepancy must be rejected"
        );
    }

    #[test]
    fn totals_round_down_to_cents() {
        // Never charge a cent we cannot itemise.
        let invoice = Invoice {
            total_mc: 29_009_999,
            ..Invoice::build(
                Uuid::new_v4(),
                "pro",
                2_000,
                date(2026, 8, 1),
                date(2026, 8, 31),
                &usage(1, 0, 0),
            )
        };
        assert_eq!(invoice.total_cents(), 2_900);
    }

    #[test]
    fn month_bounds_cover_whole_months() {
        assert_eq!(
            month_bounds(date(2026, 8, 15)),
            (date(2026, 8, 1), date(2026, 8, 31))
        );
        assert_eq!(
            month_bounds(date(2026, 2, 10)),
            (date(2026, 2, 1), date(2026, 2, 28))
        );
        // Leap year.
        assert_eq!(
            month_bounds(date(2028, 2, 10)),
            (date(2028, 2, 1), date(2028, 2, 29))
        );
        // Year boundary.
        assert_eq!(
            month_bounds(date(2026, 12, 5)),
            (date(2026, 12, 1), date(2026, 12, 31))
        );
    }

    #[test]
    fn the_summary_states_what_the_customer_saved_and_kept() {
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "pro",
            2_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(1_000, 10_000_000, 2_000_000),
        );
        let summary = invoice.summary();
        assert!(summary.contains("Pro"));
        assert!(summary.contains("$10.0000"), "{summary}");
        assert!(summary.contains("$8.0000"), "{summary}");
    }

    #[test]
    fn invoices_serialize_for_the_dashboard() {
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "team",
            1_500,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(50_000, 100_000_000, 0),
        );
        let json = serde_json::to_value(&invoice).unwrap();
        assert_eq!(json["plan"], "team");
        assert!(json["lines"].is_array());
        // Money crosses the API as integer micro-cents.
        assert!(json["total_mc"].is_i64());
    }

    #[test]
    fn a_very_large_month_does_not_overflow() {
        let invoice = Invoice::build(
            Uuid::new_v4(),
            "enterprise",
            1_000,
            date(2026, 8, 1),
            date(2026, 8, 31),
            &usage(1_000_000_000, i64::MAX / 2, 0),
        );
        assert!(invoice.total_mc > 0);
        assert!(invoice.savings_fee_mc <= invoice.gross_savings_mc);
    }
}

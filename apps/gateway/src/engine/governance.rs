//! Advanced governance — spend anomaly detection, approval flows, and cost centers.
//!
//! `MASTER_BUILD.md` P7.6 and P7.7. Budgets stop spend at a hard line; this module catches
//! the thing budgets miss — spend that is *within* budget but wildly unlike normal.
//!
//! # Why anomaly detection is not just a smaller budget
//!
//! A team with a $10,000 monthly budget that normally spends $200 a day will not trip any
//! limit when a runaway agent loop burns $3,000 in an afternoon. They will find out at
//! month end. A z-score against their own recent history catches it the same day, because
//! the signal is not the absolute number but the departure from their own baseline.
//!
//! The detector is deliberately conservative about small numbers: a team that normally
//! spends $0.10 a day and one day spends $1.00 has a huge z-score and nothing worth
//! waking anyone for.

use crate::money::MicroCents;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Standard deviations from the mean before a day is considered anomalous.
///
/// 3.0 is the conventional bar. Lower produces alerts nobody reads; higher misses the
/// runaway loop this exists to catch.
pub const ANOMALY_Z_THRESHOLD: f64 = 3.0;

/// Days of history required before the detector will make a judgement.
///
/// With fewer, the standard deviation is meaningless and every day looks anomalous.
pub const MIN_HISTORY_DAYS: usize = 7;

/// Absolute floor below which nothing is flagged, whatever the z-score.
///
/// $5.00. A tenfold jump from ten cents to one dollar is statistically dramatic and
/// operationally irrelevant, and alerting on it teaches people to ignore alerts.
pub const ANOMALY_FLOOR: MicroCents = MicroCents(5_000_000);

/// The result of an anomaly check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyReport {
    pub org_id: Uuid,
    /// The spend being judged.
    pub observed_mc: i64,
    /// Mean daily spend over the baseline window.
    pub baseline_mean_mc: i64,
    /// Standard deviation over the baseline window.
    pub baseline_stddev_mc: i64,
    /// How many standard deviations from the mean. Zero when undefined.
    pub z_score: f64,
    pub is_anomalous: bool,
    /// Why the detector reached its conclusion, in words an operator can act on.
    pub explanation: String,
}

impl AnomalyReport {
    /// Multiple of the baseline mean, for a human-readable summary.
    pub fn multiple_of_normal(&self) -> f64 {
        if self.baseline_mean_mc <= 0 {
            return 0.0;
        }
        self.observed_mc as f64 / self.baseline_mean_mc as f64
    }
}

/// Judge one day's spend against a history of daily totals.
///
/// `history` should be recent daily spend, excluding the day being judged.
pub fn detect_spend_anomaly(
    org_id: Uuid,
    observed: MicroCents,
    history: &[MicroCents],
) -> AnomalyReport {
    let insufficient = |reason: &str| AnomalyReport {
        org_id,
        observed_mc: observed.as_i64(),
        baseline_mean_mc: 0,
        baseline_stddev_mc: 0,
        z_score: 0.0,
        is_anomalous: false,
        explanation: reason.to_string(),
    };

    if history.len() < MIN_HISTORY_DAYS {
        return insufficient(&format!(
            "Not enough history: {} of {MIN_HISTORY_DAYS} days needed before a baseline \
             means anything.",
            history.len()
        ));
    }

    let count = history.len() as f64;
    let mean = history.iter().map(|v| v.as_i64() as f64).sum::<f64>() / count;
    let variance = history
        .iter()
        .map(|v| {
            let diff = v.as_i64() as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / count;
    let stddev = variance.sqrt();

    // A flat history has zero deviation, which would make every departure infinitely
    // significant. Fall back to a proportional test instead.
    if stddev < 1.0 {
        let flagged =
            observed.as_i64() as f64 > mean * 3.0 && observed.as_i64() > ANOMALY_FLOOR.as_i64();
        return AnomalyReport {
            org_id,
            observed_mc: observed.as_i64(),
            baseline_mean_mc: mean as i64,
            baseline_stddev_mc: 0,
            z_score: 0.0,
            is_anomalous: flagged,
            explanation: if flagged {
                format!(
                    "Spend of {} is more than triple the perfectly steady daily average of \
                     {}.",
                    observed.to_usd_string(),
                    MicroCents(mean as i64).to_usd_string()
                )
            } else {
                "Spend is consistent with a steady daily baseline.".to_string()
            },
        };
    }

    let z_score = (observed.as_i64() as f64 - mean) / stddev;

    // Only an *upward* departure matters. A quiet day is not an incident.
    let statistically_high = z_score > ANOMALY_Z_THRESHOLD;
    let materially_large = observed.as_i64() > ANOMALY_FLOOR.as_i64();
    let is_anomalous = statistically_high && materially_large;

    let explanation = if is_anomalous {
        format!(
            "Spend of {} is {z_score:.1} standard deviations above the {}-day average of \
             {} — roughly {:.1}x normal. Check for a runaway loop or a routing change.",
            observed.to_usd_string(),
            history.len(),
            MicroCents(mean as i64).to_usd_string(),
            observed.as_i64() as f64 / mean.max(1.0)
        )
    } else if statistically_high {
        format!(
            "Spend of {} is statistically unusual ({z_score:.1} sigma) but below the {} \
             alerting floor, so it is not worth interrupting anyone for.",
            observed.to_usd_string(),
            ANOMALY_FLOOR.to_usd_string()
        )
    } else {
        format!(
            "Spend of {} is within normal range ({z_score:.1} sigma).",
            observed.to_usd_string()
        )
    };

    AnomalyReport {
        org_id,
        observed_mc: observed.as_i64(),
        baseline_mean_mc: mean as i64,
        baseline_stddev_mc: stddev as i64,
        z_score,
        is_anomalous,
        explanation,
    }
}

// ---------------------------------------------------------------------------
// Approval flows
// ---------------------------------------------------------------------------

/// Whether a request needs human approval before it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Proceed.
    Allowed,
    /// Blocked pending approval, with the reason to show the requester.
    RequiresApproval { reason: String },
}

impl ApprovalDecision {
    /// True when the request may proceed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, ApprovalDecision::Allowed)
    }
}

/// Rules governing when a request needs approval.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// Models that always need approval, e.g. the frontier tier.
    #[serde(default)]
    pub models_requiring_approval: Vec<String>,
    /// Projected cost above which a single request needs approval.
    #[serde(default)]
    pub max_unapproved_request_mc: Option<i64>,
    /// Teams exempt from approval entirely.
    #[serde(default)]
    pub exempt_teams: Vec<String>,
}

impl ApprovalPolicy {
    /// Decide whether a request may proceed.
    ///
    /// Exemptions are checked first: an exempt team is exempt from everything, which is
    /// what makes the mechanism usable — an on-call engineer debugging an incident must
    /// not be blocked waiting for approval.
    pub fn evaluate(
        &self,
        model: &str,
        projected_cost: MicroCents,
        team: Option<&str>,
    ) -> ApprovalDecision {
        if let Some(team) = team {
            if self
                .exempt_teams
                .iter()
                .any(|exempt| exempt.eq_ignore_ascii_case(team))
            {
                return ApprovalDecision::Allowed;
            }
        }

        if self
            .models_requiring_approval
            .iter()
            .any(|pattern| matches_model(pattern, model))
        {
            return ApprovalDecision::RequiresApproval {
                reason: format!(
                    "{model} requires approval under your organisation policy. Ask an \
                     admin to approve it, or use a model from a lower tier."
                ),
            };
        }

        if let Some(limit) = self.max_unapproved_request_mc {
            if projected_cost.as_i64() > limit {
                return ApprovalDecision::RequiresApproval {
                    reason: format!(
                        "This request is projected to cost {}, above the {} per-request \
                         approval threshold.",
                        projected_cost.to_usd_string(),
                        MicroCents(limit).to_usd_string()
                    ),
                };
            }
        }

        ApprovalDecision::Allowed
    }
}

/// Match a model against a pattern with an optional trailing `*`.
fn matches_model(pattern: &str, model: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();
    match pattern.strip_suffix('*') {
        Some(prefix) => model.starts_with(prefix),
        None => pattern == model,
    }
}

// ---------------------------------------------------------------------------
// Cost centers and chargeback
// ---------------------------------------------------------------------------

/// One line of a chargeback report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChargebackLine {
    /// The cost center this spend is attributed to.
    pub cost_center: String,
    pub requests: i64,
    pub spend_mc: i64,
    pub savings_mc: i64,
    pub fee_mc: i64,
    /// Share of total organisation spend, as a percentage.
    pub share_percent: f64,
}

/// A chargeback report for a period.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChargebackReport {
    pub org_id: Uuid,
    pub period_start: String,
    pub period_end: String,
    pub lines: Vec<ChargebackLine>,
    pub total_spend_mc: i64,
    /// Spend that could not be attributed to any cost center.
    pub unattributed_mc: i64,
}

impl ChargebackReport {
    /// Build from per-cost-center totals.
    ///
    /// `entries` are `(cost_center, requests, spend, savings, fee)`. Spend with no cost
    /// center is reported separately rather than silently spread across the others —
    /// a finance team needs to know what could not be attributed, not receive a number
    /// that quietly absorbs it.
    pub fn build(
        org_id: Uuid,
        period_start: &str,
        period_end: &str,
        entries: Vec<(Option<String>, i64, MicroCents, MicroCents, MicroCents)>,
    ) -> ChargebackReport {
        let total_spend: i64 = entries
            .iter()
            .map(|(_, _, spend, _, _)| spend.as_i64())
            .sum();

        let unattributed: i64 = entries
            .iter()
            .filter(|(center, _, _, _, _)| center.is_none())
            .map(|(_, _, spend, _, _)| spend.as_i64())
            .sum();

        let mut lines: Vec<ChargebackLine> = entries
            .into_iter()
            .filter_map(|(center, requests, spend, savings, fee)| {
                center.map(|cost_center| ChargebackLine {
                    cost_center,
                    requests,
                    spend_mc: spend.as_i64(),
                    savings_mc: savings.as_i64(),
                    fee_mc: fee.as_i64(),
                    share_percent: if total_spend > 0 {
                        (spend.as_i64() as f64 / total_spend as f64) * 100.0
                    } else {
                        0.0
                    },
                })
            })
            .collect();

        // Largest first: that is the line a finance reviewer looks at.
        lines.sort_by_key(|line| std::cmp::Reverse(line.spend_mc));

        ChargebackReport {
            org_id,
            period_start: period_start.to_string(),
            period_end: period_end.to_string(),
            lines,
            total_spend_mc: total_spend,
            unattributed_mc: unattributed,
        }
    }

    /// Render as CSV for a finance system.
    ///
    /// Plain decimals, no currency symbols — a spreadsheet treats `$1.23` as text and
    /// cannot sum the column.
    pub fn to_csv(&self) -> String {
        let mut csv = String::from(
            "cost_center,requests,spend_usd,savings_usd,aegis_fee_usd,share_percent\n",
        );
        for line in &self.lines {
            csv.push_str(&format!(
                "{},{},{:.6},{:.6},{:.6},{:.2}\n",
                escape_csv(&line.cost_center),
                line.requests,
                line.spend_mc as f64 / 1_000_000.0,
                line.savings_mc as f64 / 1_000_000.0,
                line.fee_mc as f64 / 1_000_000.0,
                line.share_percent,
            ));
        }
        if self.unattributed_mc > 0 {
            csv.push_str(&format!(
                "(unattributed),0,{:.6},0.000000,0.000000,0.00\n",
                self.unattributed_mc as f64 / 1_000_000.0
            ));
        }
        csv
    }
}

/// Quote a CSV field when it contains a delimiter, quote, or newline.
fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    /// Fourteen days of steady spend around $10/day, with realistic jitter.
    fn steady_history() -> Vec<MicroCents> {
        vec![
            MicroCents(10_000_000),
            MicroCents(10_500_000),
            MicroCents(9_800_000),
            MicroCents(10_200_000),
            MicroCents(9_500_000),
            MicroCents(10_100_000),
            MicroCents(10_300_000),
            MicroCents(9_900_000),
            MicroCents(10_400_000),
            MicroCents(9_700_000),
            MicroCents(10_000_000),
            MicroCents(10_200_000),
            MicroCents(9_600_000),
            MicroCents(10_100_000),
        ]
    }

    #[test]
    fn normal_spend_is_not_flagged() {
        let report = detect_spend_anomaly(org(), MicroCents(10_300_000), &steady_history());
        assert!(!report.is_anomalous);
        assert!(report.z_score.abs() < ANOMALY_Z_THRESHOLD);
        assert!(report.explanation.contains("within normal range"));
    }

    #[test]
    fn a_runaway_loop_is_caught_the_same_day() {
        // The exact case budgets miss: $300 in a day against a $10 baseline, still well
        // inside a $10,000 monthly budget.
        let report = detect_spend_anomaly(org(), MicroCents(300_000_000), &steady_history());

        assert!(report.is_anomalous);
        assert!(report.z_score > ANOMALY_Z_THRESHOLD);
        assert!(report.multiple_of_normal() > 25.0);
        assert!(
            report.explanation.contains("runaway loop"),
            "the explanation should tell an operator where to look: {}",
            report.explanation
        );
    }

    #[test]
    fn a_quiet_day_is_never_an_incident() {
        // Only upward departures matter.
        let report = detect_spend_anomaly(org(), MicroCents(100_000), &steady_history());
        assert!(!report.is_anomalous);
        assert!(report.z_score < 0.0);
    }

    #[test]
    fn small_absolute_amounts_are_never_flagged() {
        // A tenfold jump from $0.10 to $1.00 is dramatic statistically and irrelevant
        // operationally. Alerting on it teaches people to ignore alerts.
        let tiny_history: Vec<MicroCents> = (0..14).map(|_| MicroCents(100_000)).collect();
        let report = detect_spend_anomaly(org(), MicroCents(1_000_000), &tiny_history);

        assert!(!report.is_anomalous, "{}", report.explanation);
    }

    #[test]
    fn a_large_jump_above_the_floor_is_flagged_even_on_a_flat_baseline() {
        // Perfectly flat history means zero deviation, which would make z-score infinite.
        let flat: Vec<MicroCents> = (0..14).map(|_| MicroCents(10_000_000)).collect();
        let report = detect_spend_anomaly(org(), MicroCents(100_000_000), &flat);

        assert!(report.is_anomalous);
        assert_eq!(report.baseline_stddev_mc, 0);
        assert!(report.explanation.contains("triple"));
    }

    #[test]
    fn a_flat_baseline_does_not_flag_a_normal_day() {
        let flat: Vec<MicroCents> = (0..14).map(|_| MicroCents(10_000_000)).collect();
        let report = detect_spend_anomaly(org(), MicroCents(11_000_000), &flat);
        assert!(!report.is_anomalous);
    }

    #[test]
    fn insufficient_history_produces_no_judgement() {
        // With three days of data every day looks anomalous, so the detector must decline
        // rather than guess.
        let short = vec![MicroCents(10_000_000); 3];
        let report = detect_spend_anomaly(org(), MicroCents(500_000_000), &short);

        assert!(!report.is_anomalous);
        assert!(report.explanation.contains("Not enough history"));
        assert!(report.explanation.contains("3 of 7"));
    }

    #[test]
    fn empty_history_does_not_panic() {
        let report = detect_spend_anomaly(org(), MicroCents(1_000_000), &[]);
        assert!(!report.is_anomalous);
        assert_eq!(report.multiple_of_normal(), 0.0);
    }

    #[test]
    fn reports_serialize_for_alerting() {
        let report = detect_spend_anomaly(org(), MicroCents(300_000_000), &steady_history());
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["is_anomalous"], true);
        assert!(json["explanation"].is_string());
        assert!(json["z_score"].is_number());
    }

    // --- Approvals ------------------------------------------------------------

    fn policy() -> ApprovalPolicy {
        ApprovalPolicy {
            models_requiring_approval: vec!["anthropic/claude-opus-*".into()],
            max_unapproved_request_mc: Some(1_000_000),
            exempt_teams: vec!["oncall".into()],
        }
    }

    #[test]
    fn an_ordinary_request_is_allowed() {
        let decision = policy().evaluate("openai/gpt-4o-mini", MicroCents(5_000), Some("eng"));
        assert!(decision.is_allowed());
    }

    #[test]
    fn a_restricted_model_requires_approval() {
        let decision = policy().evaluate("anthropic/claude-opus-5", MicroCents(5_000), Some("eng"));
        assert!(!decision.is_allowed());

        match decision {
            ApprovalDecision::RequiresApproval { reason } => {
                // The message must tell the requester what to do next.
                assert!(reason.contains("Ask an admin"), "{reason}");
                assert!(reason.contains("lower tier"), "{reason}");
            }
            ApprovalDecision::Allowed => panic!("should have required approval"),
        }
    }

    #[test]
    fn an_expensive_request_requires_approval() {
        let decision = policy().evaluate("openai/gpt-4o", MicroCents(5_000_000), Some("eng"));
        assert!(!decision.is_allowed());
    }

    #[test]
    fn an_exempt_team_bypasses_every_rule() {
        // An on-call engineer debugging an incident must not wait for approval. If this
        // ever regresses, the approval mechanism becomes something teams route around.
        let decision = policy().evaluate(
            "anthropic/claude-opus-5",
            MicroCents(50_000_000),
            Some("oncall"),
        );
        assert!(decision.is_allowed());
    }

    #[test]
    fn team_exemption_is_case_insensitive() {
        let decision = policy().evaluate("anthropic/claude-opus-5", MicroCents(1), Some("OnCall"));
        assert!(decision.is_allowed());
    }

    #[test]
    fn an_empty_policy_allows_everything() {
        let decision = ApprovalPolicy::default().evaluate(
            "anthropic/claude-opus-5",
            MicroCents(999_000_000),
            None,
        );
        assert!(decision.is_allowed());
    }

    #[test]
    fn model_patterns_support_a_trailing_wildcard() {
        assert!(matches_model(
            "anthropic/claude-opus-*",
            "anthropic/claude-opus-5"
        ));
        assert!(matches_model(
            "anthropic/claude-opus-*",
            "ANTHROPIC/CLAUDE-OPUS-4-5"
        ));
        assert!(!matches_model(
            "anthropic/claude-opus-*",
            "anthropic/claude-sonnet-5"
        ));
        assert!(matches_model("openai/gpt-4o", "openai/gpt-4o"));
        assert!(!matches_model("openai/gpt-4o", "openai/gpt-4o-mini"));
    }

    // --- Chargeback -----------------------------------------------------------

    #[test]
    fn chargeback_attributes_spend_by_cost_center() {
        let report = ChargebackReport::build(
            org(),
            "2026-08-01",
            "2026-08-31",
            vec![
                (
                    Some("engineering".into()),
                    5_000,
                    MicroCents(60_000_000),
                    MicroCents(40_000_000),
                    MicroCents(8_000_000),
                ),
                (
                    Some("support".into()),
                    2_000,
                    MicroCents(30_000_000),
                    MicroCents(10_000_000),
                    MicroCents(2_000_000),
                ),
                (
                    Some("marketing".into()),
                    500,
                    MicroCents(10_000_000),
                    MicroCents(5_000_000),
                    MicroCents(1_000_000),
                ),
            ],
        );

        assert_eq!(report.lines.len(), 3);
        assert_eq!(report.total_spend_mc, 100_000_000);
        // Sorted largest first.
        assert_eq!(report.lines[0].cost_center, "engineering");
        assert!((report.lines[0].share_percent - 60.0).abs() < 0.01);
        assert!((report.lines[2].share_percent - 10.0).abs() < 0.01);
    }

    #[test]
    fn unattributed_spend_is_reported_not_absorbed() {
        // A finance team needs to know what could not be attributed rather than receive
        // a number that quietly spreads it across the others.
        let report = ChargebackReport::build(
            org(),
            "2026-08-01",
            "2026-08-31",
            vec![
                (
                    Some("engineering".into()),
                    100,
                    MicroCents(60_000_000),
                    MicroCents::ZERO,
                    MicroCents::ZERO,
                ),
                (
                    None,
                    50,
                    MicroCents(40_000_000),
                    MicroCents::ZERO,
                    MicroCents::ZERO,
                ),
            ],
        );

        assert_eq!(report.lines.len(), 1, "unattributed spend is not a line");
        assert_eq!(report.unattributed_mc, 40_000_000);
        assert_eq!(report.total_spend_mc, 100_000_000);
        assert!(report.to_csv().contains("(unattributed)"));
    }

    #[test]
    fn chargeback_csv_is_numeric_so_a_spreadsheet_can_sum_it() {
        let report = ChargebackReport::build(
            org(),
            "2026-08-01",
            "2026-08-31",
            vec![(
                Some("engineering".into()),
                100,
                MicroCents(1_500_000),
                MicroCents(500_000),
                MicroCents(100_000),
            )],
        );

        let csv = report.to_csv();
        assert!(csv.starts_with("cost_center,requests,spend_usd"));
        assert!(csv.contains("engineering,100,1.500000,0.500000,0.100000"));
        assert!(!csv.contains('$'));
    }

    #[test]
    fn cost_center_names_containing_commas_are_quoted() {
        let report = ChargebackReport::build(
            org(),
            "2026-08-01",
            "2026-08-31",
            vec![(
                Some("Engineering, EMEA".into()),
                1,
                MicroCents(1_000_000),
                MicroCents::ZERO,
                MicroCents::ZERO,
            )],
        );
        assert!(report.to_csv().contains("\"Engineering, EMEA\""));
    }

    #[test]
    fn an_empty_period_produces_an_empty_report_rather_than_dividing_by_zero() {
        let report = ChargebackReport::build(org(), "2026-08-01", "2026-08-31", vec![]);
        assert!(report.lines.is_empty());
        assert_eq!(report.total_spend_mc, 0);
        assert_eq!(report.to_csv().lines().count(), 1, "header only");
    }
}

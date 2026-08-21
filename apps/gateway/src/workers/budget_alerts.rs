//! Budget alerts and notification delivery.
//!
//! Budgets that only reject at 100% are a bad product: the first a customer hears about
//! overspend is a broken deployment. Thresholds at 50/80/90/100% turn that into a warning
//! they can act on.
//!
//! # Delivery
//!
//! Email via Resend, Slack via incoming webhook, or an arbitrary webhook. When no provider
//! is configured — development, or a self-hosted instance without email — messages are
//! logged instead of dropped, so the flow is still observable.

use crate::error::Result;
use crate::money::MicroCents;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Thresholds, as percentages of the budget.
pub const THRESHOLDS: &[u32] = &[50, 80, 90, 100];

/// Minimum gap between repeat alerts for the same budget and threshold.
///
/// Without this, an organisation sitting at 81% generates an alert on every reconciliation
/// pass, and the customer learns to ignore them.
pub const ALERT_COOLDOWN: Duration = Duration::from_secs(6 * 3_600);

/// Where an alert is delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Email,
    Slack,
    Webhook,
}

impl Channel {
    /// Parse from the database representation.
    pub fn parse(raw: &str) -> Channel {
        match raw.to_ascii_lowercase().as_str() {
            "slack" => Channel::Slack,
            "webhook" => Channel::Webhook,
            _ => Channel::Email,
        }
    }
}

/// A rendered alert.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alert {
    pub subject: String,
    pub body: String,
    pub threshold_pct: u32,
    pub spend_mc: i64,
    pub limit_mc: i64,
    /// True when the budget is hard and requests are now being rejected.
    pub blocking: bool,
}

/// The highest threshold crossed by `spend` against `limit`.
///
/// Returns `None` below the lowest threshold. Only the highest is returned so crossing
/// from 45% to 95% produces one alert rather than three.
pub fn crossed_threshold(spend: MicroCents, limit: MicroCents) -> Option<u32> {
    if limit.as_i64() <= 0 {
        return None;
    }
    let percent = (spend.as_i64() as f64 / limit.as_i64() as f64) * 100.0;
    THRESHOLDS
        .iter()
        .rev()
        .find(|threshold| percent >= **threshold as f64)
        .copied()
}

/// Render an alert.
pub fn render(
    org_name: &str,
    scope: &str,
    threshold_pct: u32,
    spend: MicroCents,
    limit: MicroCents,
    hard_limit: bool,
) -> Alert {
    let blocking = hard_limit && threshold_pct >= 100;

    let subject = if blocking {
        format!("[Aegis] {org_name}: {scope} budget exceeded — requests are being rejected")
    } else {
        format!("[Aegis] {org_name}: {scope} budget at {threshold_pct}%")
    };

    let mut body = format!(
        "Your {scope} budget has reached {threshold_pct}% of its limit.\n\n\
         Spent this period: {}\n\
         Budget limit:      {}\n\
         Remaining:         {}\n",
        spend.to_usd_string(),
        limit.to_usd_string(),
        (limit - spend).floor_at_zero().to_usd_string(),
    );

    if blocking {
        // Say plainly what is happening and how to stop it. An alert that only states a
        // number leaves the reader to work out that their production traffic is down.
        body.push_str(
            "\nRequests are currently being rejected with HTTP 402. To resume service, \
             raise the budget or disable the hard limit in Settings → Budgets.\n",
        );
    } else {
        body.push_str("\nNo action is required yet. This is an early warning.\n");
    }

    Alert {
        subject,
        body,
        threshold_pct,
        spend_mc: spend.as_i64(),
        limit_mc: limit.as_i64(),
        blocking,
    }
}

/// Deliver an alert.
pub async fn deliver(
    http: &reqwest::Client,
    config: &crate::config::Config,
    channel: &Channel,
    destination: Option<&str>,
    alert: &Alert,
) -> Result<bool> {
    match channel {
        Channel::Email => {
            let Some(to) = destination else {
                tracing::warn!("email alert has no destination");
                return Ok(false);
            };
            send_email(http, config, to, &alert.subject, &alert.body).await
        }
        Channel::Slack => {
            let Some(webhook) = destination else {
                tracing::warn!("slack alert has no webhook url");
                return Ok(false);
            };
            let payload = serde_json::json!({
                "text": format!("*{}*\n```{}```", alert.subject, alert.body),
            });
            post_webhook(http, webhook, &payload).await
        }
        Channel::Webhook => {
            let Some(url) = destination else {
                tracing::warn!("webhook alert has no url");
                return Ok(false);
            };
            post_webhook(http, url, &serde_json::json!(alert)).await
        }
    }
}

/// Send a transactional email through Resend.
///
/// With no API key configured the message is logged rather than sent, so development and
/// self-hosted instances still show the flow instead of failing silently.
pub async fn send_email(
    http: &reqwest::Client,
    config: &crate::config::Config,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<bool> {
    let Some(api_key) = &config.resend_api_key else {
        tracing::info!(
            to,
            subject,
            "email not sent (no provider configured):\n{body}"
        );
        return Ok(false);
    };

    let response = http
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "from": config.email_from,
            "to": [to],
            "subject": subject,
            "text": body,
        }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| crate::error::AegisError::Internal(format!("email send failed: {e}")))?;

    let ok = response.status().is_success();
    if !ok {
        tracing::warn!(status = %response.status(), "email provider rejected the message");
    }
    Ok(ok)
}

async fn post_webhook(
    http: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
) -> Result<bool> {
    let response = http
        .post(url)
        .json(payload)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| crate::error::AegisError::Internal(format!("webhook post failed: {e}")))?;
    Ok(response.status().is_success())
}

/// The weekly digest body (Phase 5).
pub fn render_weekly_digest(
    org_name: &str,
    requests: i64,
    spend: MicroCents,
    savings: MicroCents,
    fee: MicroCents,
    cache_hit_rate: f64,
) -> Alert {
    let net = (savings - fee).floor_at_zero();
    Alert {
        subject: format!(
            "[Aegis] {org_name}: you saved {} last week",
            net.to_usd_string()
        ),
        body: format!(
            "Weekly summary for {org_name}\n\n\
             Requests:        {requests}\n\
             Spend:           {}\n\
             Gross savings:   {}\n\
             Aegis fee:       {}\n\
             Your net saving: {}\n\
             Cache hit rate:  {cache_hit_rate:.1}%\n\n\
             Full breakdown: https://app.aegis.dev/savings\n",
            spend.to_usd_string(),
            savings.to_usd_string(),
            fee.to_usd_string(),
            net.to_usd_string(),
        ),
        threshold_pct: 0,
        spend_mc: spend.as_i64(),
        limit_mc: 0,
        blocking: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_threshold_is_crossed_below_fifty_percent() {
        assert_eq!(crossed_threshold(MicroCents(0), MicroCents(1_000)), None);
        assert_eq!(crossed_threshold(MicroCents(499), MicroCents(1_000)), None);
    }

    #[test]
    fn each_threshold_is_detected() {
        assert_eq!(
            crossed_threshold(MicroCents(500), MicroCents(1_000)),
            Some(50)
        );
        assert_eq!(
            crossed_threshold(MicroCents(800), MicroCents(1_000)),
            Some(80)
        );
        assert_eq!(
            crossed_threshold(MicroCents(900), MicroCents(1_000)),
            Some(90)
        );
        assert_eq!(
            crossed_threshold(MicroCents(1_000), MicroCents(1_000)),
            Some(100)
        );
    }

    #[test]
    fn only_the_highest_threshold_is_returned() {
        // Jumping from 45% to 95% must produce one alert, not three.
        assert_eq!(
            crossed_threshold(MicroCents(950), MicroCents(1_000)),
            Some(90)
        );
        assert_eq!(
            crossed_threshold(MicroCents(5_000), MicroCents(1_000)),
            Some(100)
        );
    }

    #[test]
    fn a_zero_or_absent_limit_never_alerts() {
        assert_eq!(crossed_threshold(MicroCents(1_000), MicroCents::ZERO), None);
        assert_eq!(crossed_threshold(MicroCents(1_000), MicroCents(-5)), None);
    }

    #[test]
    fn a_warning_alert_says_no_action_is_needed() {
        let alert = render(
            "Acme",
            "organization",
            80,
            MicroCents(800_000),
            MicroCents(1_000_000),
            true,
        );
        assert!(!alert.blocking);
        assert!(alert.subject.contains("80%"));
        assert!(alert.body.contains("No action is required"));
        assert!(!alert.body.contains("rejected"));
    }

    #[test]
    fn a_blocking_alert_explains_what_broke_and_how_to_fix_it() {
        // The reader needs to learn that production traffic is down and what to do.
        let alert = render(
            "Acme",
            "organization",
            100,
            MicroCents(1_100_000),
            MicroCents(1_000_000),
            true,
        );
        assert!(alert.blocking);
        assert!(alert.subject.contains("exceeded"));
        assert!(alert.body.contains("402"), "{}", alert.body);
        assert!(alert.body.contains("raise the budget"), "{}", alert.body);
    }

    #[test]
    fn a_soft_limit_at_one_hundred_percent_does_not_claim_to_be_blocking() {
        let alert = render(
            "Acme",
            "team",
            100,
            MicroCents(1_000_000),
            MicroCents(1_000_000),
            false,
        );
        assert!(!alert.blocking);
        assert!(!alert.body.contains("402"));
    }

    #[test]
    fn alerts_show_the_remaining_budget() {
        let alert = render(
            "Acme",
            "key",
            80,
            MicroCents(800_000),
            MicroCents(1_000_000),
            true,
        );
        assert!(alert.body.contains("$0.2000"), "{}", alert.body);
    }

    #[test]
    fn remaining_budget_never_renders_negative() {
        let alert = render(
            "Acme",
            "key",
            100,
            MicroCents(5_000_000),
            MicroCents(1_000_000),
            true,
        );
        assert!(alert.body.contains("$0.0000"), "{}", alert.body);
        assert!(!alert.body.contains("-$"), "{}", alert.body);
    }

    #[test]
    fn channels_parse_with_a_safe_default() {
        assert_eq!(Channel::parse("slack"), Channel::Slack);
        assert_eq!(Channel::parse("webhook"), Channel::Webhook);
        assert_eq!(Channel::parse("email"), Channel::Email);
        // An unrecognised channel falls back to email rather than dropping the alert.
        assert_eq!(Channel::parse("carrier-pigeon"), Channel::Email);
    }

    #[test]
    fn the_weekly_digest_leads_with_the_net_saving() {
        // The number the customer cares about is what *they* kept, not what we charged.
        let digest = render_weekly_digest(
            "Acme",
            12_500,
            MicroCents(2_000_000),
            MicroCents(10_000_000),
            MicroCents(2_000_000),
            42.5,
        );
        assert!(digest.subject.contains("$8.0000"), "{}", digest.subject);
        assert!(digest.body.contains("12500"));
        assert!(digest.body.contains("42.5%"));
        assert!(digest.body.contains("Aegis fee"));
    }

    #[test]
    fn alerts_serialize_for_webhook_delivery() {
        let alert = render(
            "Acme",
            "organization",
            90,
            MicroCents(900),
            MicroCents(1_000),
            true,
        );
        let json = serde_json::to_value(&alert).unwrap();
        assert_eq!(json["threshold_pct"], 90);
        assert!(json["subject"].is_string());
        assert!(json["blocking"].is_boolean());
    }

    #[tokio::test]
    async fn email_without_a_provider_is_logged_rather_than_failing() {
        // Development and self-hosted instances must still show the flow.
        let config = crate::config::Config::for_tests();
        let sent = send_email(
            &reqwest::Client::new(),
            &config,
            "user@example.com",
            "subject",
            "body",
        )
        .await
        .unwrap();
        assert!(
            !sent,
            "no provider configured, so nothing was actually sent"
        );
    }

    #[tokio::test]
    async fn delivery_without_a_destination_is_a_no_op_not_an_error() {
        let config = crate::config::Config::for_tests();
        let http = reqwest::Client::new();
        let alert = render(
            "Acme",
            "organization",
            50,
            MicroCents(500),
            MicroCents(1_000),
            true,
        );

        for channel in [Channel::Email, Channel::Slack, Channel::Webhook] {
            let delivered = deliver(&http, &config, &channel, None, &alert)
                .await
                .unwrap();
            assert!(
                !delivered,
                "{channel:?} should not claim delivery without a destination"
            );
        }
    }
}

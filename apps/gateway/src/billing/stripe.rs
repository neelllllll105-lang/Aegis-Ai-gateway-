//! Stripe integration (Phase 4, `P4.3`).
//!
//! # Webhook signature verification is the whole security story
//!
//! A Stripe webhook endpoint is a public URL that, if trusted blindly, lets anyone POST
//! `checkout.session.completed` and upgrade themselves to Enterprise for free. Verification
//! is therefore not optional and not best-effort:
//!
//! * The signature covers `timestamp.payload`, so the **raw body** must be verified before
//!   it is parsed. Re-serializing parsed JSON changes the bytes and the signature no
//!   longer matches — which is exactly the mistake that makes people disable verification.
//! * The timestamp is checked against a tolerance, so a captured webhook cannot be
//!   replayed a week later.
//! * Comparison is constant-time.
//!
//! When no webhook secret is configured, verification **fails closed**. An unconfigured
//! secret must never mean "accept everything".

use crate::error::{AegisError, Result};
use crate::money::MicroCents;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Maximum age of a webhook, in seconds. Stripe's own default.
pub const SIGNATURE_TOLERANCE_SECONDS: i64 = 300;

/// Events we act on.
pub const HANDLED_EVENTS: &[&str] = &[
    "checkout.session.completed",
    "customer.subscription.updated",
    "customer.subscription.deleted",
    "invoice.paid",
    "invoice.payment_failed",
];

/// A verified webhook event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: serde_json::Value,
}

impl WebhookEvent {
    /// Whether this is an event we act on.
    pub fn is_handled(&self) -> bool {
        HANDLED_EVENTS.contains(&self.event_type.as_str())
    }

    /// The Stripe customer id, if the event carries one.
    pub fn customer_id(&self) -> Option<String> {
        self.data
            .pointer("/object/customer")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Our organisation id, passed through `client_reference_id` at checkout.
    ///
    /// This is how a Stripe customer is tied back to an Aegis organisation without
    /// trusting anything the caller supplies later.
    pub fn org_reference(&self) -> Option<uuid::Uuid> {
        self.data
            .pointer("/object/client_reference_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .or_else(|| {
                self.data
                    .pointer("/object/metadata/aegis_org_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
            })
    }

    /// The plan this event implies, from subscription metadata.
    pub fn plan(&self) -> Option<String> {
        self.data
            .pointer("/object/metadata/aegis_plan")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
    }
}

/// Parsed `Stripe-Signature` header.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureHeader {
    pub timestamp: i64,
    /// Every `v1` signature present. Stripe sends more than one during secret rotation,
    /// and accepting any of them is what makes rotation possible without downtime.
    pub signatures: Vec<String>,
}

/// Parse a `Stripe-Signature` header: `t=1614556800,v1=abc...,v1=def...`.
pub fn parse_signature_header(header: &str) -> Option<SignatureHeader> {
    let mut timestamp = None;
    let mut signatures = Vec::new();

    for part in header.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        match key.trim() {
            "t" => timestamp = value.trim().parse::<i64>().ok(),
            "v1" => signatures.push(value.trim().to_string()),
            _ => {}
        }
    }

    let timestamp = timestamp?;
    (!signatures.is_empty()).then_some(SignatureHeader {
        timestamp,
        signatures,
    })
}

/// Verify a webhook signature against the **raw** request body.
///
/// `payload` must be the exact bytes Stripe sent. Passing re-serialized JSON produces a
/// signature mismatch for a perfectly genuine event.
pub fn verify_signature(
    payload: &[u8],
    signature_header: &str,
    secret: Option<&str>,
    now: i64,
) -> Result<()> {
    // Fail closed. An unconfigured secret must never mean "accept everything" — that
    // would leave the endpoint open to anyone who finds the URL.
    let Some(secret) = secret else {
        return Err(AegisError::Unauthorized(
            "webhook signature verification is not configured".into(),
        ));
    };

    let Some(parsed) = parse_signature_header(signature_header) else {
        return Err(AegisError::Unauthorized(
            "malformed Stripe-Signature header".into(),
        ));
    };

    // Reject stale webhooks so a captured request cannot be replayed later.
    if (now - parsed.timestamp).abs() > SIGNATURE_TOLERANCE_SECONDS {
        return Err(AegisError::Unauthorized(
            "webhook timestamp is outside the tolerance window".into(),
        ));
    }

    let signed_payload = [parsed.timestamp.to_string().as_bytes(), b".", payload].concat();

    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return Err(AegisError::Crypto);
    };
    mac.update(&signed_payload);
    let expected = hex::encode(mac.finalize().into_bytes());

    let matched = parsed
        .signatures
        .iter()
        .any(|candidate| crate::crypto::secure_compare(&expected, candidate));

    if matched {
        Ok(())
    } else {
        Err(AegisError::Unauthorized(
            "webhook signature does not match".into(),
        ))
    }
}

/// Verify and parse a webhook in one step.
pub fn parse_webhook(
    payload: &[u8],
    signature_header: &str,
    secret: Option<&str>,
    now: i64,
) -> Result<WebhookEvent> {
    // Verification happens against the raw bytes, before parsing. The order matters.
    verify_signature(payload, signature_header, secret, now)?;

    serde_json::from_slice(payload)
        .map_err(|e| AegisError::BadRequest(format!("unparseable webhook body: {e}")))
}

/// The plan change a verified event implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanChange {
    /// Move the organisation onto this plan.
    Upgrade { plan: String },
    /// Return the organisation to the free plan.
    Downgrade,
    /// Payment failed; warn but do not downgrade yet.
    PaymentFailed,
    /// Nothing to do.
    None,
}

/// Decide what a webhook means for an organisation's plan.
pub fn plan_change_for(event: &WebhookEvent) -> PlanChange {
    match event.event_type.as_str() {
        "checkout.session.completed" | "customer.subscription.updated" => match event.plan() {
            Some(plan) if is_valid_plan(&plan) => PlanChange::Upgrade { plan },
            _ => PlanChange::None,
        },
        "customer.subscription.deleted" => PlanChange::Downgrade,
        // Deliberately not a downgrade. Stripe retries failed payments for days, and
        // cutting off a customer on the first failure — often an expired card — is a
        // worse outcome than carrying them through the retry window.
        "invoice.payment_failed" => PlanChange::PaymentFailed,
        _ => PlanChange::None,
    }
}

/// Whether a plan name is one we sell.
pub fn is_valid_plan(plan: &str) -> bool {
    matches!(plan, "free" | "pro" | "team" | "enterprise" | "api")
}

/// Build a Stripe Checkout session request body.
pub fn checkout_session_body(
    org_id: uuid::Uuid,
    plan: &str,
    price_id: &str,
    success_url: &str,
    cancel_url: &str,
) -> Vec<(String, String)> {
    // Stripe expects form encoding, not JSON.
    vec![
        ("mode".to_string(), "subscription".to_string()),
        ("line_items[0][price]".to_string(), price_id.to_string()),
        ("line_items[0][quantity]".to_string(), "1".to_string()),
        // Both are set: `client_reference_id` survives on the session, and the metadata
        // copy propagates to the subscription, where later events can still find it.
        ("client_reference_id".to_string(), org_id.to_string()),
        ("metadata[aegis_org_id]".to_string(), org_id.to_string()),
        ("metadata[aegis_plan]".to_string(), plan.to_string()),
        (
            "subscription_data[metadata][aegis_org_id]".to_string(),
            org_id.to_string(),
        ),
        (
            "subscription_data[metadata][aegis_plan]".to_string(),
            plan.to_string(),
        ),
        ("success_url".to_string(), success_url.to_string()),
        ("cancel_url".to_string(), cancel_url.to_string()),
    ]
}

/// Stripe's own API root. A parameter, not a constant, only so a future test can point it
/// at a local server — nothing in this codebase overrides it today.
pub const API_BASE: &str = "https://api.stripe.com/v1";

fn provider_error(status: u16, message: String) -> AegisError {
    AegisError::Provider {
        provider: "stripe".to_string(),
        status,
        message,
    }
}

/// Extract a top-level string field from a Stripe JSON response, or turn a missing one into
/// the same `Provider` error a malformed response gets — a field Stripe stopped sending is
/// exactly as actionable to a caller as a field it never sent.
fn require_str_field(body: &serde_json::Value, field: &str) -> Result<String> {
    body.get(field)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            provider_error(
                502,
                format!("Stripe response had no \"{field}\" field: {body}"),
            )
        })
}

/// Turn a non-2xx Stripe response into a `Provider` error carrying whatever Stripe's own
/// `error.message` says — the same "surface the upstream's own words" rule every LLM
/// provider adapter already follows, not a generic "Stripe request failed".
async fn stripe_error(response: reqwest::Response) -> AegisError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or(body);
    provider_error(status, message)
}

/// `POST /v1/customers` — create a Stripe customer for an organisation that has never
/// checked out before. Called at most once per organisation; the id it returns is stored on
/// `organizations.stripe_customer_id` and every later checkout reuses it.
pub async fn create_customer(
    http: &reqwest::Client,
    secret_key: &str,
    org_id: uuid::Uuid,
    org_name: &str,
    email: Option<&str>,
) -> Result<String> {
    let mut form = vec![
        ("name".to_string(), org_name.to_string()),
        ("metadata[aegis_org_id]".to_string(), org_id.to_string()),
    ];
    if let Some(email) = email {
        form.push(("email".to_string(), email.to_string()));
    }

    let response = http
        .post(format!("{API_BASE}/customers"))
        .basic_auth(secret_key, None::<&str>)
        .form(&form)
        .send()
        .await
        .map_err(|e| provider_error(502, format!("could not reach Stripe: {e}")))?;

    if !response.status().is_success() {
        return Err(stripe_error(response).await);
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| provider_error(502, format!("unparseable Stripe response: {e}")))?;
    require_str_field(&body, "id")
}

/// `POST /v1/checkout/sessions` — start a Checkout session and return its hosted URL, which
/// the dashboard redirects the browser to. `body` is `checkout_session_body`'s output, with
/// `customer` appended by the caller once the organisation's Stripe customer id is known.
pub async fn create_checkout_session(
    http: &reqwest::Client,
    secret_key: &str,
    body: Vec<(String, String)>,
) -> Result<String> {
    let response = http
        .post(format!("{API_BASE}/checkout/sessions"))
        .basic_auth(secret_key, None::<&str>)
        .form(&body)
        .send()
        .await
        .map_err(|e| provider_error(502, format!("could not reach Stripe: {e}")))?;

    if !response.status().is_success() {
        return Err(stripe_error(response).await);
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| provider_error(502, format!("unparseable Stripe response: {e}")))?;
    require_str_field(&parsed, "url")
}

/// Build an invoice item for the savings-share fee.
///
/// Stripe bills in whole cents, so the micro-cent figure is floored. Rounding up would
/// charge a cent we cannot itemise on the invoice.
pub fn savings_fee_invoice_item(
    customer_id: &str,
    amount: MicroCents,
    description: &str,
) -> Vec<(String, String)> {
    vec![
        ("customer".to_string(), customer_id.to_string()),
        ("amount".to_string(), amount.to_cents().max(0).to_string()),
        ("currency".to_string(), "usd".to_string()),
        ("description".to_string(), description.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test_secret";
    const NOW: i64 = 1_800_000_000;

    /// Produce a valid signature header for a payload, as Stripe would.
    fn sign(payload: &[u8], timestamp: i64, secret: &str) -> String {
        let signed = [timestamp.to_string().as_bytes(), b".", payload].concat();
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&signed);
        format!(
            "t={timestamp},v1={}",
            hex::encode(mac.finalize().into_bytes())
        )
    }

    fn event_payload(event_type: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": "evt_123",
            "type": event_type,
            "data": {
                "object": {
                    "customer": "cus_123",
                    "client_reference_id": "11111111-1111-1111-1111-111111111111",
                    "metadata": {"aegis_plan": "pro"}
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn a_genuine_webhook_verifies() {
        let payload = event_payload("checkout.session.completed");
        let header = sign(&payload, NOW, SECRET);
        assert!(verify_signature(&payload, &header, Some(SECRET), NOW).is_ok());
    }

    #[test]
    fn an_unsigned_webhook_is_rejected() {
        // Without this, anyone who finds the URL can upgrade themselves for free.
        let payload = event_payload("checkout.session.completed");
        let err = verify_signature(&payload, "t=1,v1=deadbeef", Some(SECRET), 1).unwrap_err();
        assert_eq!(err.error_type(), "unauthorized");
    }

    #[test]
    fn verification_fails_closed_when_no_secret_is_configured() {
        // An unconfigured secret must never mean "accept everything".
        let payload = event_payload("checkout.session.completed");
        let header = sign(&payload, NOW, SECRET);
        let err = verify_signature(&payload, &header, None, NOW).unwrap_err();
        assert!(format!("{err}").contains("not configured"));
    }

    #[test]
    fn a_tampered_payload_fails_verification() {
        let payload = event_payload("checkout.session.completed");
        let header = sign(&payload, NOW, SECRET);

        let mut tampered = payload.clone();
        tampered.extend_from_slice(b" ");
        assert!(verify_signature(&tampered, &header, Some(SECRET), NOW).is_err());
    }

    #[test]
    fn a_signature_from_another_secret_fails() {
        let payload = event_payload("checkout.session.completed");
        let header = sign(&payload, NOW, "whsec_someone_elses_secret");
        assert!(verify_signature(&payload, &header, Some(SECRET), NOW).is_err());
    }

    #[test]
    fn a_replayed_webhook_is_rejected_once_it_is_stale() {
        let payload = event_payload("invoice.paid");
        let header = sign(&payload, NOW, SECRET);

        // Genuine at the time.
        assert!(verify_signature(&payload, &header, Some(SECRET), NOW).is_ok());
        // Replayed an hour later, the same bytes must be refused.
        let err = verify_signature(&payload, &header, Some(SECRET), NOW + 3_600).unwrap_err();
        assert!(format!("{err}").contains("tolerance"));
    }

    #[test]
    fn the_tolerance_window_is_symmetric() {
        let payload = event_payload("invoice.paid");
        let header = sign(&payload, NOW, SECRET);

        assert!(verify_signature(&payload, &header, Some(SECRET), NOW + 299).is_ok());
        assert!(verify_signature(&payload, &header, Some(SECRET), NOW - 299).is_ok());
        assert!(verify_signature(&payload, &header, Some(SECRET), NOW + 301).is_err());
    }

    #[test]
    fn multiple_signatures_are_accepted_for_secret_rotation() {
        // Stripe sends several v1 signatures while a secret is being rotated. Accepting
        // any of them is what makes rotation possible without an outage.
        let payload = event_payload("invoice.paid");
        let valid = sign(&payload, NOW, SECRET);
        let valid_signature = valid.split("v1=").nth(1).unwrap();
        let header = format!("t={NOW},v1=0000000000000000,v1={valid_signature}");

        assert!(verify_signature(&payload, &header, Some(SECRET), NOW).is_ok());
    }

    #[test]
    fn malformed_signature_headers_are_rejected() {
        let payload = event_payload("invoice.paid");
        for header in ["", "garbage", "t=abc,v1=def", "v1=nothingelse", "t=123"] {
            assert!(
                verify_signature(&payload, header, Some(SECRET), NOW).is_err(),
                "accepted {header:?}"
            );
        }
    }

    #[test]
    fn signature_headers_parse() {
        let parsed = parse_signature_header("t=1614556800,v1=abc123,v0=ignored").unwrap();
        assert_eq!(parsed.timestamp, 1_614_556_800);
        assert_eq!(parsed.signatures, vec!["abc123"]);
    }

    #[test]
    fn a_verified_webhook_parses_into_an_event() {
        let payload = event_payload("checkout.session.completed");
        let header = sign(&payload, NOW, SECRET);

        let event = parse_webhook(&payload, &header, Some(SECRET), NOW).unwrap();
        assert_eq!(event.id, "evt_123");
        assert_eq!(event.event_type, "checkout.session.completed");
        assert!(event.is_handled());
        assert_eq!(event.customer_id().as_deref(), Some("cus_123"));
        assert_eq!(
            event.org_reference().map(|id| id.to_string()).as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
    }

    #[test]
    fn a_checkout_completion_upgrades_the_plan() {
        let payload = event_payload("checkout.session.completed");
        let header = sign(&payload, NOW, SECRET);
        let event = parse_webhook(&payload, &header, Some(SECRET), NOW).unwrap();

        assert_eq!(
            plan_change_for(&event),
            PlanChange::Upgrade {
                plan: "pro".to_string()
            }
        );
    }

    #[test]
    fn a_cancelled_subscription_downgrades() {
        let payload = event_payload("customer.subscription.deleted");
        let header = sign(&payload, NOW, SECRET);
        let event = parse_webhook(&payload, &header, Some(SECRET), NOW).unwrap();
        assert_eq!(plan_change_for(&event), PlanChange::Downgrade);
    }

    #[test]
    fn a_failed_payment_does_not_immediately_downgrade() {
        // Stripe retries for days. Cutting a customer off on the first failure — usually
        // an expired card — is worse than carrying them through the retry window.
        let payload = event_payload("invoice.payment_failed");
        let header = sign(&payload, NOW, SECRET);
        let event = parse_webhook(&payload, &header, Some(SECRET), NOW).unwrap();

        assert_eq!(plan_change_for(&event), PlanChange::PaymentFailed);
        assert_ne!(plan_change_for(&event), PlanChange::Downgrade);
    }

    #[test]
    fn an_unknown_plan_in_metadata_changes_nothing() {
        // Metadata is attacker-influenceable if a webhook secret ever leaks; an unknown
        // plan must not become a free upgrade.
        let payload = serde_json::to_vec(&serde_json::json!({
            "id": "evt_1",
            "type": "checkout.session.completed",
            "data": {"object": {"metadata": {"aegis_plan": "unlimited-everything"}}}
        }))
        .unwrap();
        let header = sign(&payload, NOW, SECRET);
        let event = parse_webhook(&payload, &header, Some(SECRET), NOW).unwrap();

        assert_eq!(plan_change_for(&event), PlanChange::None);
    }

    #[test]
    fn only_plans_we_sell_are_valid() {
        for plan in ["free", "pro", "team", "enterprise", "api"] {
            assert!(is_valid_plan(plan));
        }
        for plan in ["", "unlimited", "PRO", "enterprise-plus"] {
            assert!(!is_valid_plan(plan), "accepted {plan:?}");
        }
    }

    #[test]
    fn unhandled_event_types_are_ignored() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "id": "evt_1", "type": "customer.created", "data": {"object": {}}
        }))
        .unwrap();
        let header = sign(&payload, NOW, SECRET);
        let event = parse_webhook(&payload, &header, Some(SECRET), NOW).unwrap();

        assert!(!event.is_handled());
        assert_eq!(plan_change_for(&event), PlanChange::None);
    }

    #[test]
    fn the_checkout_body_carries_the_org_reference_in_both_places() {
        // The session reference is lost by the time subscription events arrive, so the
        // metadata copy is what ties later events back to an organisation.
        let org_id = uuid::Uuid::new_v4();
        let body = checkout_session_body(
            org_id,
            "pro",
            "price_123",
            "https://app.aegis.dev/billing?ok=1",
            "https://app.aegis.dev/billing",
        );
        let map: std::collections::HashMap<_, _> = body.into_iter().collect();

        assert_eq!(map.get("client_reference_id").unwrap(), &org_id.to_string());
        assert_eq!(
            map.get("subscription_data[metadata][aegis_org_id]")
                .unwrap(),
            &org_id.to_string()
        );
        assert_eq!(map.get("mode").unwrap(), "subscription");
    }

    #[test]
    fn invoice_items_are_denominated_in_whole_cents() {
        // Stripe bills in cents; rounding up would charge a cent we cannot itemise.
        let item = savings_fee_invoice_item("cus_123", MicroCents(2_009_999), "Savings share");
        let map: std::collections::HashMap<_, _> = item.into_iter().collect();

        assert_eq!(map.get("amount").unwrap(), "200");
        assert_eq!(map.get("currency").unwrap(), "usd");
    }

    #[test]
    fn required_string_fields_extract_cleanly() {
        let body = serde_json::json!({"id": "cus_abc123", "object": "customer"});
        assert_eq!(require_str_field(&body, "id").unwrap(), "cus_abc123");
    }

    #[test]
    fn a_missing_required_field_is_a_provider_error_not_a_panic() {
        // A field Stripe stopped sending must surface as an ordinary 502, not a crash —
        // this is exactly the shape a breaking API change on Stripe's side would take.
        let body = serde_json::json!({"object": "checkout.session"});
        let err = require_str_field(&body, "url").unwrap_err();
        assert_eq!(err.error_type(), "provider_error");
        assert!(format!("{err}").contains("url"));
    }

    #[test]
    fn a_negative_fee_never_becomes_a_charge() {
        let item = savings_fee_invoice_item("cus_123", MicroCents(-5_000), "Savings share");
        let map: std::collections::HashMap<_, _> = item.into_iter().collect();
        assert_eq!(map.get("amount").unwrap(), "0");
    }
}

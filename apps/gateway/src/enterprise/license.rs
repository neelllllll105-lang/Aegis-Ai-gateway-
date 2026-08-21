//! Self-hosted license validation (Phase 6, `P6.3`).
//!
//! The enterprise differentiator is that Aegis runs inside a customer's own VPC. That
//! creates a licensing problem with an unusual constraint: **the licensed instance may
//! have no route to us at all.** A network-checked license would take a bank's production
//! gateway offline the moment their egress rules changed, which is a far worse outcome
//! than a few days of unlicensed use.
//!
//! So licenses are **offline-verifiable**: a signed payload the customer holds, checked
//! locally against a public verification secret compiled into the binary. No callback, no
//! phone-home, no dependency on our uptime for their uptime.
//!
//! # Expiry is a grace period, not a kill switch
//!
//! Past `expires_at` the license enters a [`GRACE_PERIOD`] during which everything keeps
//! working and the deployment warns loudly. Only after that does it degrade — and even
//! then it degrades to a *warning state*, never to a hard stop. A licensing dispute must
//! never be the reason a customer's production traffic fails.

use crate::error::{AegisError, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// How long a license keeps working past its expiry.
pub const GRACE_PERIOD_DAYS: i64 = 30;

/// The signed contents of a license.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LicensePayload {
    /// Customer organisation name, shown in the admin console.
    pub customer: String,
    /// Licensed plan tier.
    pub plan: String,
    /// Maximum seats, or `None` for unlimited.
    #[serde(default)]
    pub max_seats: Option<u32>,
    /// Maximum requests per month, or `None` for unlimited.
    #[serde(default)]
    pub max_monthly_requests: Option<u64>,
    /// Unix timestamp after which the grace period begins.
    pub expires_at: i64,
    /// Issue timestamp.
    pub issued_at: i64,
    /// Opaque license id, for support and revocation lists.
    pub license_id: String,
}

impl LicensePayload {
    /// Seconds until expiry; negative once expired.
    pub fn seconds_until_expiry(&self, now: i64) -> i64 {
        self.expires_at - now
    }

    /// Days past expiry; zero while still valid.
    pub fn days_expired(&self, now: i64) -> i64 {
        ((now - self.expires_at).max(0)) / 86_400
    }
}

/// The state a license is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseState {
    /// Valid and current.
    Valid,
    /// Past expiry but inside the grace period.
    Grace { days_remaining: i64 },
    /// Past expiry and past the grace period.
    Expired { days_expired: i64 },
    /// Signature did not verify, or the payload could not be read.
    Invalid { reason: String },
}

impl LicenseState {
    /// Whether the deployment should serve traffic.
    ///
    /// True for everything except a forged license. An expired license still serves — it
    /// warns. Refusing to serve on a licensing technicality turns our commercial problem
    /// into the customer's outage.
    pub fn should_serve(&self) -> bool {
        !matches!(self, LicenseState::Invalid { .. })
    }

    /// Whether the admin console should show a warning banner.
    pub fn needs_attention(&self) -> bool {
        !matches!(self, LicenseState::Valid)
    }

    /// A message for the operator.
    pub fn message(&self) -> String {
        match self {
            LicenseState::Valid => "License valid.".to_string(),
            LicenseState::Grace { days_remaining } => format!(
                "License expired. Service continues for {days_remaining} more days. \
                 Contact support to renew."
            ),
            LicenseState::Expired { days_expired } => format!(
                "License expired {days_expired} days ago and the grace period has ended. \
                 Service continues, but please renew."
            ),
            LicenseState::Invalid { reason } => {
                format!("License could not be verified: {reason}")
            }
        }
    }
}

/// A license as issued: `base64(payload).base64(signature)`.
///
/// Two dot-separated base64 segments, like a JWT but without the algorithm-negotiation
/// header that has caused JWT so many `alg: none` vulnerabilities. The algorithm here is
/// fixed and not caller-selectable.
pub fn issue(payload: &LicensePayload, secret: &str) -> Result<String> {
    let json = serde_json::to_vec(payload)
        .map_err(|e| AegisError::Internal(format!("license encode: {e}")))?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json);
    let signature = sign(&encoded, secret);
    Ok(format!("{encoded}.{signature}"))
}

/// Verify a license and determine its state.
pub fn verify(license: &str, secret: &str, now: i64) -> LicenseState {
    let Some((encoded, signature)) = license.trim().split_once('.') else {
        return LicenseState::Invalid {
            reason: "malformed license format".to_string(),
        };
    };

    let expected = sign(encoded, secret);
    // Constant-time comparison: a byte-by-byte check leaks, through timing, how much of a
    // forged signature was correct, which is enough to forge one given enough attempts.
    if !crate::crypto::secure_compare(&expected, signature) {
        return LicenseState::Invalid {
            reason: "signature mismatch".to_string(),
        };
    }

    let Ok(json) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded) else {
        return LicenseState::Invalid {
            reason: "payload is not valid base64".to_string(),
        };
    };
    let Ok(payload) = serde_json::from_slice::<LicensePayload>(&json) else {
        return LicenseState::Invalid {
            reason: "payload could not be parsed".to_string(),
        };
    };

    if payload.expires_at >= now {
        return LicenseState::Valid;
    }

    let days_expired = payload.days_expired(now);
    if days_expired <= GRACE_PERIOD_DAYS {
        LicenseState::Grace {
            days_remaining: GRACE_PERIOD_DAYS - days_expired,
        }
    } else {
        LicenseState::Expired { days_expired }
    }
}

/// Read the payload without checking the signature.
///
/// For displaying licence details in the admin console *after* [`verify`] has passed.
/// Never use this to make an authorisation decision.
pub fn read_payload_unverified(license: &str) -> Option<LicensePayload> {
    let (encoded, _) = license.trim().split_once('.')?;
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    serde_json::from_slice(&json).ok()
}

fn sign(data: &str, secret: &str) -> String {
    // `new_from_slice` only fails for key lengths HMAC cannot accept, which SHA-256 does
    // not have — it accepts any length. The fallback keeps the function total.
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(data.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-license-signing-secret";
    const DAY: i64 = 86_400;

    fn payload(expires_at: i64) -> LicensePayload {
        LicensePayload {
            customer: "Acme Corp".to_string(),
            plan: "enterprise".to_string(),
            max_seats: Some(500),
            max_monthly_requests: Some(10_000_000),
            expires_at,
            issued_at: 1_700_000_000,
            license_id: "lic_abc123".to_string(),
        }
    }

    #[test]
    fn a_freshly_issued_license_verifies() {
        let now = 1_800_000_000;
        let license = issue(&payload(now + 365 * DAY), SECRET).unwrap();
        assert_eq!(verify(&license, SECRET, now), LicenseState::Valid);
    }

    #[test]
    fn a_license_verifies_with_no_network_access() {
        // The whole point: a customer VPC with no egress must still validate.
        let now = 1_800_000_000;
        let license = issue(&payload(now + DAY), SECRET).unwrap();
        let state = verify(&license, SECRET, now);
        assert!(state.should_serve());
        assert!(!state.needs_attention());
    }

    #[test]
    fn a_forged_signature_is_rejected() {
        let now = 1_800_000_000;
        let license = issue(&payload(now + DAY), SECRET).unwrap();
        let (encoded, _) = license.split_once('.').unwrap();
        let forged = format!("{encoded}.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");

        let state = verify(&forged, SECRET, now);
        assert!(matches!(state, LicenseState::Invalid { .. }));
        assert!(!state.should_serve(), "a forged license must not serve");
    }

    #[test]
    fn a_tampered_payload_is_rejected() {
        // Editing the seat count must invalidate the signature.
        let now = 1_800_000_000;
        let license = issue(&payload(now + DAY), SECRET).unwrap();
        let (_, signature) = license.split_once('.').unwrap();

        let mut tampered_payload = payload(now + DAY);
        tampered_payload.max_seats = Some(1_000_000);
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&tampered_payload).unwrap());

        let tampered = format!("{encoded}.{signature}");
        assert!(matches!(
            verify(&tampered, SECRET, now),
            LicenseState::Invalid { .. }
        ));
    }

    #[test]
    fn a_license_signed_with_another_secret_is_rejected() {
        let now = 1_800_000_000;
        let license = issue(&payload(now + DAY), "someone-elses-secret").unwrap();
        assert!(matches!(
            verify(&license, SECRET, now),
            LicenseState::Invalid { .. }
        ));
    }

    #[test]
    fn malformed_licenses_are_rejected_without_panicking() {
        let now = 1_800_000_000;
        for bad in ["", "no-dot", "....", "!!!.???", "a.b"] {
            let state = verify(bad, SECRET, now);
            assert!(
                matches!(state, LicenseState::Invalid { .. }),
                "accepted malformed license: {bad:?}"
            );
        }
    }

    #[test]
    fn an_expired_license_enters_grace_and_keeps_serving() {
        // A licensing dispute must never be the cause of a customer outage.
        let expiry = 1_800_000_000;
        let license = issue(&payload(expiry), SECRET).unwrap();

        let state = verify(&license, SECRET, expiry + 5 * DAY);
        assert_eq!(state, LicenseState::Grace { days_remaining: 25 });
        assert!(state.should_serve());
        assert!(state.needs_attention());
        assert!(state.message().contains("25 more days"));
    }

    #[test]
    fn the_grace_boundary_is_inclusive() {
        let expiry = 1_800_000_000;
        let license = issue(&payload(expiry), SECRET).unwrap();

        assert_eq!(
            verify(&license, SECRET, expiry + GRACE_PERIOD_DAYS * DAY),
            LicenseState::Grace { days_remaining: 0 }
        );
        assert!(matches!(
            verify(&license, SECRET, expiry + (GRACE_PERIOD_DAYS + 1) * DAY),
            LicenseState::Expired { .. }
        ));
    }

    #[test]
    fn even_a_long_expired_license_keeps_serving() {
        let expiry = 1_800_000_000;
        let license = issue(&payload(expiry), SECRET).unwrap();

        let state = verify(&license, SECRET, expiry + 500 * DAY);
        assert!(matches!(state, LicenseState::Expired { .. }));
        assert!(
            state.should_serve(),
            "an expired license degrades to a warning, never to an outage"
        );
        assert!(state.needs_attention());
    }

    #[test]
    fn expiry_arithmetic_is_correct() {
        let now = 1_800_000_000;
        let license = payload(now + 10 * DAY);
        assert_eq!(license.seconds_until_expiry(now), 10 * DAY);
        assert_eq!(license.days_expired(now), 0);

        let expired = payload(now - 3 * DAY);
        assert!(expired.seconds_until_expiry(now) < 0);
        assert_eq!(expired.days_expired(now), 3);
    }

    #[test]
    fn payload_details_survive_the_round_trip() {
        let now = 1_800_000_000;
        let original = payload(now + DAY);
        let license = issue(&original, SECRET).unwrap();

        let recovered = read_payload_unverified(&license).unwrap();
        assert_eq!(recovered, original);
        assert_eq!(recovered.customer, "Acme Corp");
        assert_eq!(recovered.max_seats, Some(500));
    }

    #[test]
    fn unverified_reads_do_not_imply_validity() {
        // The function exists for display only. It must not be mistaken for verification.
        let now = 1_800_000_000;
        let license = issue(&payload(now + DAY), "wrong-secret").unwrap();

        assert!(read_payload_unverified(&license).is_some());
        assert!(
            matches!(verify(&license, SECRET, now), LicenseState::Invalid { .. }),
            "reading a payload must not mean the license verifies"
        );
    }

    #[test]
    fn licenses_carry_no_algorithm_field_to_confuse() {
        // JWT-style `alg` negotiation is the source of the `alg: none` family of
        // vulnerabilities. There is no algorithm field here to attack.
        let now = 1_800_000_000;
        let license = issue(&payload(now + DAY), SECRET).unwrap();
        assert_eq!(license.matches('.').count(), 1, "expected exactly two segments");
        assert!(!license.contains("alg"));
    }

    #[test]
    fn state_messages_are_actionable() {
        assert!(LicenseState::Valid.message().contains("valid"));
        assert!(LicenseState::Grace { days_remaining: 10 }
            .message()
            .contains("Contact support"));
        assert!(LicenseState::Expired { days_expired: 60 }
            .message()
            .contains("Service continues"));
        assert!(LicenseState::Invalid {
            reason: "signature mismatch".into()
        }
        .message()
        .contains("signature mismatch"));
    }
}

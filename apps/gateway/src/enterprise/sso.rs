//! SSO: OIDC and SAML assertion validation (Phase 6, `P6.1`).
//!
//! This module handles the *validation* half of SSO — deciding whether an assertion an
//! identity provider sent us is genuine, current, and actually intended for us. The
//! redirect dance itself is ordinary OAuth and lives in the route layer.
//!
//! # The checks that are load-bearing
//!
//! Every published SAML and OIDC vulnerability of the last decade comes from skipping one
//! of these, so each is implemented explicitly and tested for the bypass it prevents:
//!
//! * **Audience** — an assertion issued for a different service provider must be rejected,
//!   or any IdP customer can log into any tenant.
//! * **Issuer** — must match the connection the organisation configured.
//! * **Expiry and not-before** — with a small clock-skew allowance, because IdP and SP
//!   clocks genuinely differ by seconds.
//! * **Replay** — an assertion id may be used exactly once.
//! * **Email domain** — the organisation pins which domains its IdP may assert, so a
//!   compromised IdP cannot claim `ceo@another-company.com`.

use crate::error::{AegisError, Result};
use crate::store::KvStore;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Tolerated clock difference between us and an identity provider.
pub const CLOCK_SKEW_SECONDS: i64 = 120;

/// How long a consumed assertion id is remembered for replay detection.
///
/// Comfortably longer than any assertion lifetime, so a replay cannot simply wait out the
/// window.
pub const REPLAY_WINDOW: Duration = Duration::from_secs(3_600);

/// The claims an assertion carries, normalised across OIDC and SAML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SsoAssertion {
    /// Unique assertion id, for replay detection.
    pub id: String,
    /// The identity provider that issued it.
    pub issuer: String,
    /// Who it was issued for. Must be us.
    pub audience: String,
    /// The authenticated subject.
    pub subject: String,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Unix timestamp when the assertion expires.
    pub expires_at: i64,
    /// Unix timestamp before which it is not valid.
    #[serde(default)]
    pub not_before: Option<i64>,
}

/// The configuration an organisation registered for its identity provider.
#[derive(Debug, Clone, PartialEq)]
pub struct SsoConnection {
    pub org_id: uuid::Uuid,
    pub protocol: String,
    pub expected_issuer: String,
    pub expected_audience: String,
    /// Domains this IdP may assert. Empty means any.
    pub allowed_email_domains: Vec<String>,
    pub is_active: bool,
}

/// Validate an assertion against a connection.
///
/// Stateless checks only; replay detection needs the store and is
/// [`check_and_record_replay`].
pub fn validate(assertion: &SsoAssertion, connection: &SsoConnection, now: i64) -> Result<()> {
    if !connection.is_active {
        return Err(AegisError::Forbidden(
            "single sign-on is not enabled for this organisation".into(),
        ));
    }

    // Issuer: the assertion must come from the IdP this org configured.
    if assertion.issuer.trim() != connection.expected_issuer.trim() {
        return Err(AegisError::Unauthorized(
            "assertion issuer does not match the configured identity provider".into(),
        ));
    }

    // Audience: without this check, an assertion an IdP issued for *any other* service
    // provider would be accepted here. This is the single most important check in SSO.
    if assertion.audience.trim() != connection.expected_audience.trim() {
        return Err(AegisError::Unauthorized(
            "assertion audience does not match this service provider".into(),
        ));
    }

    if assertion.expires_at + CLOCK_SKEW_SECONDS < now {
        return Err(AegisError::Unauthorized("assertion has expired".into()));
    }

    if let Some(not_before) = assertion.not_before {
        if not_before - CLOCK_SKEW_SECONDS > now {
            return Err(AegisError::Unauthorized(
                "assertion is not yet valid".into(),
            ));
        }
    }

    if assertion.email.trim().is_empty() || !assertion.email.contains('@') {
        return Err(AegisError::Unauthorized(
            "assertion does not carry a usable email address".into(),
        ));
    }

    // Domain pinning: a compromised or misconfigured IdP must not be able to assert an
    // identity outside the domains this organisation controls.
    if !connection.allowed_email_domains.is_empty() {
        let domain = assertion
            .email
            .rsplit('@')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let permitted = connection
            .allowed_email_domains
            .iter()
            .any(|allowed| allowed.trim().to_ascii_lowercase() == domain);
        if !permitted {
            return Err(AegisError::Forbidden(format!(
                "email domain {domain} is not permitted for this connection"
            )));
        }
    }

    Ok(())
}

/// Redis key for a consumed assertion.
fn replay_key(org_id: uuid::Uuid, assertion_id: &str) -> String {
    format!("aegis:sso:seen:{org_id}:{assertion_id}")
}

/// Record an assertion id and reject a repeat.
///
/// An assertion is a bearer credential: anyone who captures one from a browser redirect
/// can present it again unless it is single-use.
pub async fn check_and_record_replay(
    store: &dyn KvStore,
    org_id: uuid::Uuid,
    assertion_id: &str,
) -> Result<()> {
    let key = replay_key(org_id, assertion_id);

    if store.get(&key).await?.is_some() {
        return Err(AegisError::Unauthorized(
            "this assertion has already been used".into(),
        ));
    }

    store.set_ex(&key, "1", REPLAY_WINDOW).await?;
    Ok(())
}

/// Build the OIDC authorisation URL.
pub fn authorization_url(
    issuer: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    nonce: &str,
) -> String {
    format!(
        "{}/authorize?response_type=code&scope=openid%20email%20profile\
         &client_id={}&redirect_uri={}&state={}&nonce={}",
        issuer.trim_end_matches('/'),
        urlencode(client_id),
        urlencode(redirect_uri),
        urlencode(state),
        urlencode(nonce),
    )
}

/// Percent-encode a query parameter value.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Extract claims from a decoded OIDC id token payload.
///
/// Signature verification against the provider JWKS happens before this; the function
/// only maps claims onto our normalised shape.
pub fn assertion_from_id_token(
    claims: &serde_json::Value,
    fallback_audience: &str,
) -> Option<SsoAssertion> {
    let email = claims.get("email").and_then(|v| v.as_str())?;
    let subject = claims.get("sub").and_then(|v| v.as_str())?;
    let issuer = claims.get("iss").and_then(|v| v.as_str())?;

    // `aud` is a string or an array of strings, depending on the provider.
    let audience = match claims.get("aud") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(values)) => values
            .first()
            .and_then(|v| v.as_str())
            .unwrap_or(fallback_audience)
            .to_string(),
        _ => fallback_audience.to_string(),
    };

    Some(SsoAssertion {
        // `jti` when present, otherwise a subject/issued-at pair, which is unique enough
        // to catch a replay of the same token.
        id: claims
            .get("jti")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                format!(
                    "{subject}:{}",
                    claims.get("iat").and_then(|v| v.as_i64()).unwrap_or(0)
                )
            }),
        issuer: issuer.to_string(),
        audience,
        subject: subject.to_string(),
        email: email.to_string(),
        name: claims
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        expires_at: claims.get("exp").and_then(|v| v.as_i64()).unwrap_or(0),
        not_before: claims.get("nbf").and_then(|v| v.as_i64()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use uuid::Uuid;

    const NOW: i64 = 1_800_000_000;

    fn connection() -> SsoConnection {
        SsoConnection {
            org_id: Uuid::new_v4(),
            protocol: "oidc".to_string(),
            expected_issuer: "https://acme.okta.com".to_string(),
            expected_audience: "https://api.aegis.dev".to_string(),
            allowed_email_domains: vec!["acme.com".to_string()],
            is_active: true,
        }
    }

    fn assertion() -> SsoAssertion {
        SsoAssertion {
            id: "assertion-1".to_string(),
            issuer: "https://acme.okta.com".to_string(),
            audience: "https://api.aegis.dev".to_string(),
            subject: "00u1abc".to_string(),
            email: "jane@acme.com".to_string(),
            name: Some("Jane Doe".to_string()),
            expires_at: NOW + 300,
            not_before: Some(NOW - 60),
        }
    }

    #[test]
    fn a_valid_assertion_passes() {
        assert!(validate(&assertion(), &connection(), NOW).is_ok());
    }

    #[test]
    fn an_assertion_for_another_service_provider_is_rejected() {
        // Without the audience check, any customer of the same IdP could log into any
        // tenant. This is the most important test in the module.
        let mut forged = assertion();
        forged.audience = "https://some-other-saas.example.com".to_string();

        let err = validate(&forged, &connection(), NOW).unwrap_err();
        assert_eq!(err.error_type(), "unauthorized");
        assert!(format!("{err}").contains("audience"));
    }

    #[test]
    fn an_assertion_from_the_wrong_issuer_is_rejected() {
        let mut forged = assertion();
        forged.issuer = "https://attacker.example.com".to_string();
        assert!(validate(&forged, &connection(), NOW).is_err());
    }

    #[test]
    fn an_expired_assertion_is_rejected() {
        let mut expired = assertion();
        expired.expires_at = NOW - 3_600;
        let err = validate(&expired, &connection(), NOW).unwrap_err();
        assert!(format!("{err}").contains("expired"));
    }

    #[test]
    fn clock_skew_is_tolerated_in_both_directions() {
        // IdP and SP clocks genuinely differ by seconds; rejecting on that produces
        // intermittent login failures nobody can reproduce.
        let mut just_expired = assertion();
        just_expired.expires_at = NOW - 60;
        assert!(validate(&just_expired, &connection(), NOW).is_ok());

        let mut just_issued = assertion();
        just_issued.not_before = Some(NOW + 60);
        assert!(validate(&just_issued, &connection(), NOW).is_ok());
    }

    #[test]
    fn skew_tolerance_has_a_limit() {
        let mut long_expired = assertion();
        long_expired.expires_at = NOW - CLOCK_SKEW_SECONDS - 60;
        assert!(validate(&long_expired, &connection(), NOW).is_err());

        let mut far_future = assertion();
        far_future.not_before = Some(NOW + CLOCK_SKEW_SECONDS + 60);
        assert!(validate(&far_future, &connection(), NOW).is_err());
    }

    #[test]
    fn email_domains_are_pinned_to_the_organisation() {
        // A compromised IdP must not be able to assert an identity outside the domains
        // the organisation controls.
        let mut outside = assertion();
        outside.email = "attacker@evil.com".to_string();

        let err = validate(&outside, &connection(), NOW).unwrap_err();
        assert_eq!(err.error_type(), "forbidden");
        assert!(format!("{err}").contains("evil.com"));
    }

    #[test]
    fn domain_matching_is_case_insensitive() {
        let mut mixed_case = assertion();
        mixed_case.email = "Jane@ACME.com".to_string();
        assert!(validate(&mixed_case, &connection(), NOW).is_ok());
    }

    #[test]
    fn an_empty_domain_list_permits_any_domain() {
        let mut open = connection();
        open.allowed_email_domains = vec![];

        let mut assertion = assertion();
        assertion.email = "someone@anywhere.com".to_string();
        assert!(validate(&assertion, &open, NOW).is_ok());
    }

    #[test]
    fn an_assertion_without_a_usable_email_is_rejected() {
        for email in ["", "   ", "not-an-email"] {
            let mut broken = assertion();
            broken.email = email.to_string();
            assert!(validate(&broken, &connection(), NOW).is_err(), "{email:?}");
        }
    }

    #[test]
    fn a_disabled_connection_rejects_everything() {
        let mut disabled = connection();
        disabled.is_active = false;
        assert!(validate(&assertion(), &disabled, NOW).is_err());
    }

    #[tokio::test]
    async fn an_assertion_can_only_be_used_once() {
        // Assertions are bearer credentials captured from a browser redirect.
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();

        assert!(check_and_record_replay(&store, org_id, "assertion-1")
            .await
            .is_ok());

        let err = check_and_record_replay(&store, org_id, "assertion-1")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("already been used"));
    }

    #[tokio::test]
    async fn replay_records_are_scoped_per_organisation() {
        let store = MemoryStore::new();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();

        check_and_record_replay(&store, first, "shared-id")
            .await
            .unwrap();
        // A different tenant using a coincidentally identical id must not be blocked.
        assert!(check_and_record_replay(&store, second, "shared-id")
            .await
            .is_ok());
    }

    #[test]
    fn an_oidc_id_token_maps_onto_the_normalised_shape() {
        let claims = serde_json::json!({
            "iss": "https://acme.okta.com",
            "aud": "aegis-client-id",
            "sub": "00u1abc",
            "email": "jane@acme.com",
            "name": "Jane Doe",
            "exp": NOW + 300,
            "iat": NOW,
            "jti": "token-123"
        });

        let assertion = assertion_from_id_token(&claims, "fallback").unwrap();
        assert_eq!(assertion.id, "token-123");
        assert_eq!(assertion.audience, "aegis-client-id");
        assert_eq!(assertion.email, "jane@acme.com");
        assert_eq!(assertion.name.as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn an_array_audience_is_handled() {
        // Some providers send `aud` as an array; treating it as a string loses the check.
        let claims = serde_json::json!({
            "iss": "https://acme.okta.com",
            "aud": ["aegis-client-id", "other"],
            "sub": "00u1abc",
            "email": "jane@acme.com",
            "exp": NOW + 300
        });
        let assertion = assertion_from_id_token(&claims, "fallback").unwrap();
        assert_eq!(assertion.audience, "aegis-client-id");
    }

    #[test]
    fn a_token_without_jti_still_gets_a_replay_id() {
        let claims = serde_json::json!({
            "iss": "https://acme.okta.com",
            "aud": "aegis",
            "sub": "00u1abc",
            "email": "jane@acme.com",
            "exp": NOW + 300,
            "iat": NOW
        });
        let assertion = assertion_from_id_token(&claims, "aegis").unwrap();
        assert!(assertion.id.contains("00u1abc"));
        assert!(assertion.id.contains(&NOW.to_string()));
    }

    #[test]
    fn a_token_missing_required_claims_is_rejected() {
        for claims in [
            serde_json::json!({"iss": "x", "sub": "y"}),
            serde_json::json!({"email": "a@b.com", "sub": "y"}),
            serde_json::json!({"email": "a@b.com", "iss": "x"}),
        ] {
            assert!(assertion_from_id_token(&claims, "aegis").is_none());
        }
    }

    #[test]
    fn the_authorization_url_is_correctly_encoded() {
        let url = authorization_url(
            "https://acme.okta.com/",
            "client id with spaces",
            "https://api.aegis.dev/api/auth/sso/callback",
            "state&value",
            "nonce123",
        );
        assert!(url.starts_with("https://acme.okta.com/authorize?"));
        assert!(url.contains("client%20id%20with%20spaces"));
        assert!(url.contains("state%26value"), "{url}");
        assert!(url.contains("scope=openid%20email%20profile"));
        // The redirect must be encoded, not interpolated raw.
        assert!(!url.contains("redirect_uri=https://"), "{url}");
    }

    #[test]
    fn url_encoding_preserves_unreserved_characters() {
        assert_eq!(urlencode("abcXYZ012-_.~"), "abcXYZ012-_.~");
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("a/b?c=d"), "a%2Fb%3Fc%3Dd");
    }
}

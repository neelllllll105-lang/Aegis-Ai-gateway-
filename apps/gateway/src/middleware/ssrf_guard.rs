//! Blocks BYOK `base_url` values that point at internal infrastructure.
//!
//! # Why this exists
//!
//! Every provider adapter honours `Credential.base_url` as a literal override — that is
//! the entire point of the `custom` provider, and every other adapter inherits the same
//! mechanism for self-hosted/proxy endpoints. Nothing previously stopped an organisation
//! from setting it to `http://169.254.169.254/latest/meta-data/iam/security-credentials/`
//! (the cloud metadata service, on every major cloud) or an RFC1918 address reachable from
//! the gateway's own network position. Any authenticated org — including a brand-new
//! free-tier signup, which requires no review or plan upgrade — could set this and then
//! trigger it with `POST /api/providers/{id}/test`, turning the gateway into a
//! server-side request forgery proxy against its own infrastructure. Found in the
//! enterprise-readiness audit; this module is the fix, checked at credential-creation
//! time, which is where the attack chain actually starts.
//!
//! # What this does not fully close
//!
//! This resolves the hostname **once, at validation time**, and rejects it if it resolves
//! to disallowed space. A hostname that resolves to a public IP now and a private one at
//! *request* time (classic DNS rebinding) would pass this check and still be a live SSRF
//! vector, because the actual HTTP client resolves independently when the request is
//! eventually sent. Closing that fully needs either a custom resolver wired into
//! `reqwest`'s connector (so every real connection re-validates the IP it is about to
//! open a socket to, not just the one seen here) or a connect-time IP allowlist — a larger
//! change than the credential-creation gate this module provides, deliberately not
//! attempted alongside it. Treat this as closing the demonstrated, zero-effort attack
//! path — direct use of a literal internal address or hostname — not as a complete SSRF
//! solution.

use crate::error::{AegisError, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Reject a BYOK `base_url` that resolves to internal or link-local address space.
///
/// Called once, at credential-creation time (`POST /api/providers`), which is the point
/// in the actual exploit chain where an attacker-controlled URL first enters the system —
/// closing it here means the far more numerous call sites that later *use* a stored
/// `base_url` (every provider's `chat`/`chat_stream`) never need to re-implement this
/// check to be safe from the credential that passed it.
pub async fn validate_base_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url)
        .map_err(|e| AegisError::BadRequest(format!("base_url is not a valid URL: {e}")))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AegisError::BadRequest(format!(
                "base_url must be http or https, not {other:?}"
            )))
        }
    }

    let Some(host) = parsed.host_str() else {
        return Err(AegisError::BadRequest(
            "base_url must include a host".into(),
        ));
    };

    // Reject the handful of hostnames that mean "this machine" without ever needing DNS —
    // cheap, and covers a case a resolver lookup could behave inconsistently on across
    // platforms.
    if host.eq_ignore_ascii_case("localhost") {
        return Err(disallowed(host));
    }

    let port = parsed.port_or_known_default().unwrap_or(443);

    // Resolve exactly as the real HTTP client eventually will, so a hostname is judged by
    // where it actually points rather than by string pattern-matching its spelling.
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|e| {
        AegisError::BadRequest(format!("base_url host {host:?} could not be resolved: {e}"))
    })?;

    let mut resolved_any = false;
    for socket_addr in addrs {
        resolved_any = true;
        if is_disallowed(socket_addr.ip()) {
            return Err(disallowed(host));
        }
    }

    if !resolved_any {
        return Err(AegisError::BadRequest(format!(
            "base_url host {host:?} did not resolve to any address"
        )));
    }

    Ok(())
}

fn disallowed(host: &str) -> AegisError {
    AegisError::BadRequest(format!(
        "base_url host {host:?} resolves to an internal or link-local address, which is \
         not permitted. This includes cloud metadata services (169.254.169.254), \
         loopback, and private network ranges."
    ))
}

/// Whether `ip` is internal, loopback, or link-local space a customer-supplied endpoint
/// must never be allowed to reach from the gateway's own network position.
fn is_disallowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_v4(v4),
        IpAddr::V6(v6) => is_disallowed_v6(v6),
    }
}

fn is_disallowed_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        // 169.254.0.0/16 — link-local, and specifically where every major cloud serves
        // instance metadata (AWS/GCP/Azure all use 169.254.169.254 for this).
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        // 100.64.0.0/10 — carrier-grade NAT space, used internally by several cloud
        // providers for their own infrastructure (notably part of AWS's internal
        // networking). Not covered by `is_private()`, which only knows RFC1918.
        || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

fn is_disallowed_v6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        // fe80::/10 — link-local.
        || (ip.segments()[0] & 0xffc0) == 0xfe80
        // fc00::/7 — unique local (IPv6's equivalent of RFC1918).
        || (ip.segments()[0] & 0xfe00) == 0xfc00
        // ::ffff:0:0/96 — an IPv4-mapped IPv6 address. Must be unwrapped and checked
        // against the v4 rules, or `::ffff:169.254.169.254` sails straight through every
        // check above that only knows how to read a `Ipv6Addr`.
        || ip
            .to_ipv4_mapped()
            .is_some_and(is_disallowed_v4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_metadata_addresses_are_disallowed() {
        assert!(is_disallowed("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn rfc1918_private_ranges_are_disallowed() {
        for ip in ["10.0.0.1", "172.16.0.1", "172.31.255.255", "192.168.1.1"] {
            assert!(
                is_disallowed(ip.parse().unwrap()),
                "{ip} should be disallowed"
            );
        }
    }

    #[test]
    fn loopback_is_disallowed() {
        assert!(is_disallowed("127.0.0.1".parse().unwrap()));
        assert!(is_disallowed("127.0.0.53".parse().unwrap()));
        assert!(is_disallowed("::1".parse().unwrap()));
    }

    #[test]
    fn carrier_grade_nat_space_is_disallowed() {
        // Used internally by AWS and others; not covered by RFC1918 alone.
        assert!(is_disallowed("100.64.0.1".parse().unwrap()));
        assert!(is_disallowed("100.100.100.200".parse().unwrap()));
        // 100.128.0.0 and above is public space again — must not be over-blocked.
        assert!(!is_disallowed("100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn ipv6_link_local_and_unique_local_are_disallowed() {
        assert!(is_disallowed("fe80::1".parse().unwrap()));
        assert!(is_disallowed("fc00::1".parse().unwrap()));
        assert!(is_disallowed("fd12:3456:789a::1".parse().unwrap()));
    }

    #[test]
    fn an_ipv4_mapped_ipv6_metadata_address_is_still_caught() {
        // The exact bypass that "only check Ipv6Addr's own is_* methods" would miss.
        assert!(is_disallowed("::ffff:169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn ordinary_public_addresses_are_allowed() {
        assert!(!is_disallowed("8.8.8.8".parse().unwrap()));
        assert!(!is_disallowed("1.1.1.1".parse().unwrap()));
        assert!(!is_disallowed("2606:4700:4700::1111".parse().unwrap())); // Cloudflare DNS
    }

    #[tokio::test]
    async fn a_literal_metadata_ip_is_rejected_with_no_dns_lookup_needed() {
        let err = validate_base_url("http://169.254.169.254/latest/meta-data/")
            .await
            .unwrap_err();
        assert!(matches!(err, AegisError::BadRequest(_)));
    }

    #[tokio::test]
    async fn localhost_is_rejected_by_name_without_needing_to_resolve_it() {
        let err = validate_base_url("http://localhost:8080/v1")
            .await
            .unwrap_err();
        assert!(matches!(err, AegisError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_literal_private_ip_is_rejected() {
        let err = validate_base_url("http://192.168.1.1/v1/chat/completions")
            .await
            .unwrap_err();
        assert!(matches!(err, AegisError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_non_http_scheme_is_rejected_before_any_dns_lookup() {
        let err = validate_base_url("file:///etc/passwd").await.unwrap_err();
        assert!(matches!(err, AegisError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_genuine_public_provider_endpoint_is_accepted() {
        // api.openai.com resolves to real public IPs; this is what every legitimate
        // custom-endpoint or self-hosted-proxy configuration looks like.
        let result = validate_base_url("https://api.openai.com/v1").await;
        assert!(
            result.is_ok(),
            "a genuine public endpoint must not be rejected: {result:?}"
        );
    }
}

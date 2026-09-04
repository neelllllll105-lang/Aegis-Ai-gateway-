//! Security response headers — `MASTER_BUILD.md` Part 9 item 9.
//!
//! Applied to every response. The API surface returns JSON to programmatic clients, so the
//! policy can be far stricter than a normal web application needs: nothing is framed,
//! nothing is scripted, nothing is loaded from anywhere.

use axum::http::{HeaderName, HeaderValue};

/// Content Security Policy for API responses.
///
/// `default-src 'none'` is the correct policy for an endpoint that only ever returns JSON:
/// there is no legitimate resource for a browser to fetch, so any attempt is an attack or
/// a bug.
pub const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'";

/// HSTS: two years, subdomains included, preload-eligible.
pub const HSTS: &str = "max-age=63072000; includeSubDomains; preload";

/// The header set applied to every response.
pub fn headers(is_production: bool) -> Vec<(HeaderName, HeaderValue)> {
    let mut headers = vec![
        (
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ),
        (
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ),
        (
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ),
        (
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(API_CSP),
        ),
        (
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ),
        (
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ),
    ];

    // HSTS only over TLS. Sending it from a plain-HTTP development server would pin
    // localhost to https in the developer browser, which is a genuinely annoying thing to
    // undo.
    if is_production {
        headers.push((
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static(HSTS),
        ));
    }

    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_names(is_production: bool) -> Vec<String> {
        headers(is_production)
            .into_iter()
            .map(|(name, _)| name.as_str().to_string())
            .collect()
    }

    #[test]
    fn every_required_header_is_present() {
        let names = header_names(true);
        for required in [
            "x-content-type-options",
            "x-frame-options",
            "referrer-policy",
            "content-security-policy",
            "strict-transport-security",
        ] {
            assert!(names.contains(&required.to_string()), "missing {required}");
        }
    }

    #[test]
    fn hsts_is_withheld_in_development() {
        // Sending HSTS from a local plain-HTTP server pins localhost to https in the
        // developer browser, which is unpleasant to reverse.
        assert!(!header_names(false).contains(&"strict-transport-security".to_string()));
    }

    #[test]
    fn framing_is_denied() {
        let values: Vec<String> = headers(true)
            .into_iter()
            .map(|(_, value)| value.to_str().unwrap_or_default().to_string())
            .collect();
        assert!(values.iter().any(|v| v == "DENY"));
        assert!(API_CSP.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn the_api_policy_forbids_loading_anything() {
        assert!(API_CSP.starts_with("default-src 'none'"));
        assert!(!API_CSP.contains("unsafe-inline"));
        assert!(!API_CSP.contains("unsafe-eval"));
    }

    #[test]
    fn hsts_is_long_enough_for_preload_submission() {
        assert!(HSTS.contains("max-age=63072000"));
        assert!(HSTS.contains("includeSubDomains"));
        assert!(HSTS.contains("preload"));
    }
}

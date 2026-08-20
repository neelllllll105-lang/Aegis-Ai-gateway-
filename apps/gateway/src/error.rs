//! Unified error type.
//!
//! Every failure surfaced to a client is one of these variants, rendered as
//! `{"error": {"type": "...", "message": "...", "docs_url": "..."}}` per
//! `MASTER_BUILD.md` Part 12. The `type` field is machine-readable and stable —
//! clients branch on it, so renaming a variant is a breaking API change.

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Base URL for error documentation. Every client-facing error links to its own anchor.
const DOCS_BASE: &str = "https://docs.aegis.dev/errors";

/// The one error type crossing every module boundary in the gateway.
#[derive(Debug, thiserror::Error)]
pub enum AegisError {
    // ---- 4xx: caller's problem ----
    /// Missing, malformed, revoked, or expired credentials.
    #[error("{0}")]
    Unauthorized(String),

    /// Authenticated, but not permitted — including cross-tenant access attempts.
    #[error("{0}")]
    Forbidden(String),

    /// Resource does not exist, or does not exist *for this org*.
    #[error("{0}")]
    NotFound(String),

    /// Request body or parameters failed validation.
    #[error("{0}")]
    BadRequest(String),

    /// Rate limit exceeded. Carries the retry hint in seconds.
    #[error("rate limit exceeded")]
    RateLimited { retry_after_secs: u64, limit: u32, scope: &'static str },

    /// A hard budget was exceeded. HTTP 402 — the caller must raise the budget or pay.
    #[error("budget exceeded")]
    BudgetExceeded { spend_micro_cents: i64, limit_micro_cents: i64 },

    /// The requested model is not available to this org (plan gate or allowlist).
    #[error("{0}")]
    ModelNotAllowed(String),

    /// Payload exceeded the size cap.
    #[error("request body too large")]
    PayloadTooLarge,

    // ---- 5xx / upstream: our problem or the provider's ----
    /// An upstream provider returned an error we could not recover from.
    #[error("provider error: {message}")]
    Provider { provider: String, status: u16, message: String },

    /// Every candidate provider failed or was circuit-open.
    #[error("all providers unavailable: {0}")]
    AllProvidersFailed(String),

    /// The upstream provider timed out.
    #[error("provider timeout after {0}s")]
    ProviderTimeout(u64),

    /// Database failure.
    #[error("database error")]
    Database(#[from] sqlx::Error),

    /// Cache/store failure.
    #[error("store error: {0}")]
    Store(String),

    /// Encryption or decryption failure. Never includes key material.
    #[error("cryptography error")]
    Crypto,

    /// Configuration is invalid — surfaced at startup, not to clients.
    #[error("configuration error: {0}")]
    Config(String),

    /// Anything genuinely unexpected.
    #[error("internal error")]
    Internal(String),
}

impl AegisError {
    /// Stable, machine-readable error type. Clients branch on this string.
    pub fn error_type(&self) -> &'static str {
        match self {
            AegisError::Unauthorized(_) => "unauthorized",
            AegisError::Forbidden(_) => "forbidden",
            AegisError::NotFound(_) => "not_found",
            AegisError::BadRequest(_) => "invalid_request",
            AegisError::RateLimited { .. } => "rate_limit_exceeded",
            AegisError::BudgetExceeded { .. } => "budget_exceeded",
            AegisError::ModelNotAllowed(_) => "model_not_allowed",
            AegisError::PayloadTooLarge => "payload_too_large",
            AegisError::Provider { .. } => "provider_error",
            AegisError::AllProvidersFailed(_) => "all_providers_failed",
            AegisError::ProviderTimeout(_) => "provider_timeout",
            AegisError::Database(_) => "internal_error",
            AegisError::Store(_) => "internal_error",
            AegisError::Crypto => "internal_error",
            AegisError::Config(_) => "internal_error",
            AegisError::Internal(_) => "internal_error",
        }
    }

    /// HTTP status for this error.
    pub fn status(&self) -> StatusCode {
        match self {
            AegisError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AegisError::Forbidden(_) => StatusCode::FORBIDDEN,
            AegisError::NotFound(_) => StatusCode::NOT_FOUND,
            AegisError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AegisError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            AegisError::BudgetExceeded { .. } => StatusCode::PAYMENT_REQUIRED,
            AegisError::ModelNotAllowed(_) => StatusCode::FORBIDDEN,
            AegisError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            AegisError::Provider { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            AegisError::AllProvidersFailed(_) => StatusCode::SERVICE_UNAVAILABLE,
            AegisError::ProviderTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Whether the message is safe to show a client verbatim.
    ///
    /// Internal errors (database, crypto, config) are deliberately opaque: their detail
    /// goes to our logs, never to a caller who might be probing us.
    fn client_message(&self) -> String {
        match self {
            AegisError::Database(_) | AegisError::Crypto | AegisError::Config(_) => {
                "An internal error occurred. If this persists, contact support with the \
                 request id."
                    .to_string()
            }
            AegisError::Store(_) => {
                "A transient storage error occurred. Please retry.".to_string()
            }
            AegisError::Internal(_) => {
                "An internal error occurred. If this persists, contact support with the \
                 request id."
                    .to_string()
            }
            AegisError::RateLimited { retry_after_secs, limit, scope } => format!(
                "Rate limit of {limit} requests/minute for this {scope} exceeded. Retry \
                 in {retry_after_secs}s."
            ),
            AegisError::BudgetExceeded { spend_micro_cents, limit_micro_cents } => {
                let spend = crate::money::MicroCents(*spend_micro_cents).to_usd_string();
                let limit = crate::money::MicroCents(*limit_micro_cents).to_usd_string();
                format!(
                    "Budget exceeded: {spend} spent against a {limit} limit. Raise the \
                     budget or upgrade your plan to continue."
                )
            }
            other => other.to_string(),
        }
    }

    /// Link to the documentation anchor for this error type.
    pub fn docs_url(&self) -> String {
        format!("{DOCS_BASE}#{}", self.error_type())
    }

    /// True when this error should be logged at `error` level rather than `warn`.
    /// Client mistakes are not our incidents; upstream and internal failures are.
    pub fn is_our_fault(&self) -> bool {
        matches!(
            self,
            AegisError::Database(_)
                | AegisError::Crypto
                | AegisError::Config(_)
                | AegisError::Internal(_)
                | AegisError::Store(_)
                | AegisError::AllProvidersFailed(_)
        )
    }
}

/// The JSON envelope every error is rendered into.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

/// Error detail. `type` is stable; `message` is human-facing.
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub message: String,
    pub docs_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_url: Option<String>,
}

impl IntoResponse for AegisError {
    fn into_response(self) -> Response {
        let status = self.status();

        // Log with full internal detail. The client sees only `client_message()`.
        if self.is_our_fault() {
            tracing::error!(error.type = self.error_type(), error.detail = %self, "request failed");
        } else {
            tracing::warn!(error.type = self.error_type(), error.detail = %self, "request rejected");
        }

        let upgrade_url = matches!(self, AegisError::BudgetExceeded { .. })
            .then(|| "https://app.aegis.dev/billing".to_string());

        let body = ErrorBody {
            error: ErrorDetail {
                error_type: self.error_type(),
                message: self.client_message(),
                docs_url: self.docs_url(),
                upgrade_url,
            },
        };

        let mut headers = HeaderMap::new();
        if let AegisError::RateLimited { retry_after_secs, limit, .. } = &self {
            insert_num(&mut headers, "retry-after", *retry_after_secs);
            insert_num(&mut headers, "x-ratelimit-limit", *limit as u64);
            insert_num(&mut headers, "x-ratelimit-remaining", 0);
        }
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        (status, headers, axum::Json(body)).into_response()
    }
}

fn insert_num(headers: &mut HeaderMap, name: &'static str, value: u64) {
    if let Ok(v) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, v);
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, AegisError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_match_the_spec() {
        assert_eq!(AegisError::Unauthorized("x".into()).status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            AegisError::RateLimited { retry_after_secs: 1, limit: 60, scope: "key" }.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        // Budget exceeded is 402 Payment Required — Part 5 stage [3].
        assert_eq!(
            AegisError::BudgetExceeded { spend_micro_cents: 1, limit_micro_cents: 0 }.status(),
            StatusCode::PAYMENT_REQUIRED
        );
        assert_eq!(
            AegisError::AllProvidersFailed("x".into()).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn error_types_are_stable_strings() {
        assert_eq!(
            AegisError::BudgetExceeded { spend_micro_cents: 0, limit_micro_cents: 0 }.error_type(),
            "budget_exceeded"
        );
        assert_eq!(AegisError::BadRequest("x".into()).error_type(), "invalid_request");
    }

    #[test]
    fn internal_errors_do_not_leak_detail_to_clients() {
        // A database error message could contain table names, query fragments, or worse.
        let err = AegisError::Internal("connection string postgres://user:hunter2@db".into());
        let msg = err.client_message();
        assert!(!msg.contains("hunter2"), "internal detail leaked: {msg}");
        assert!(!msg.contains("postgres://"), "internal detail leaked: {msg}");
        assert_eq!(err.error_type(), "internal_error");
    }

    #[test]
    fn client_errors_do_surface_their_message() {
        let err = AegisError::BadRequest("messages must not be empty".into());
        assert!(err.client_message().contains("messages must not be empty"));
    }

    #[test]
    fn budget_message_formats_money_correctly() {
        let err = AegisError::BudgetExceeded {
            spend_micro_cents: 5_000_000,
            limit_micro_cents: 1_000_000,
        };
        let msg = err.client_message();
        assert!(msg.contains("$5.0000"), "{msg}");
        assert!(msg.contains("$1.0000"), "{msg}");
    }

    #[test]
    fn every_error_has_a_docs_link() {
        let errors = [
            AegisError::Unauthorized("x".into()),
            AegisError::NotFound("x".into()),
            AegisError::PayloadTooLarge,
            AegisError::ProviderTimeout(30),
        ];
        for e in errors {
            assert!(e.docs_url().starts_with(DOCS_BASE));
            assert!(e.docs_url().ends_with(e.error_type()));
        }
    }

    #[test]
    fn fault_attribution_separates_client_and_server_errors() {
        assert!(!AegisError::BadRequest("x".into()).is_our_fault());
        assert!(!AegisError::Unauthorized("x".into()).is_our_fault());
        assert!(AegisError::Crypto.is_our_fault());
        assert!(AegisError::AllProvidersFailed("x".into()).is_our_fault());
    }
}

//! Google Vertex AI adapter — Gemini models served through GCP's enterprise surface.
//!
//! # Why this is a separate provider from `google.rs`, not a flag on it
//!
//! Vertex serves the same Gemini model family as AI Studio (the `google` provider), with
//! the same `generateContent` request and response JSON — that part is genuinely shared,
//! and this module calls straight into `super::google::{build_body, parse_response,
//! parse_stream_chunk}` rather than re-deriving it. What is *not* shared is everything
//! about identity:
//!
//! * AI Studio authenticates with a bare API key, checked synchronously.
//! * Vertex authenticates with a GCP service account: a JSON key is exchanged for a
//!   short-lived OAuth2 access token by POSTing a signed JWT assertion to Google's token
//!   endpoint — an async network call with its own caching and failure modes.
//! * AI Studio's URL is fixed. Vertex's URL is scoped to a customer's own GCP project and
//!   a region they choose, both of which live inside the credential, not in config.
//!
//! Trying to force that into `google.rs` behind a branch would leak Vertex's auth
//! machinery into the one adapter every other test and every other reader already
//! understands. Two small, honest modules beat one module with a personality disorder.
//!
//! # Scope
//!
//! Gemini models under Vertex's `publishers/google` namespace only. Vertex's model
//! garden also serves Anthropic, Llama, and other third-party models on the same
//! infrastructure, each with its own request shape — out of scope here, see
//! `docs/adr/0007-vertex-ai-jwt-signing.md`.
//!
//! # What "the API key" means for this provider
//!
//! A customer's BYOK credential for `vertex` is not a short string — it is the *entire
//! contents* of a GCP service account JSON key file, pasted as text into the same
//! `Credential.api_key` field every other provider uses for a bare key. This provider is
//! the only consumer of that string; nothing about how it is stored, encrypted, or
//! displayed (a four-character hint, never returned in full) changes for it.
//!
//! The region defaults to `us-central1` and can be overridden by adding a non-standard
//! `"aegis_region"` field to that same pasted JSON — deliberately not by repurposing
//! `Credential.base_url`, whose meaning ("a literal endpoint override, e.g. for a private
//! VPC-SC restricted Vertex endpoint") stays identical to what every other provider
//! already uses it for.

use super::google;
use super::{ChunkStream, Credential, Provider};
use crate::error::{AegisError, Result};
use crate::types::{NormalizedRequest, NormalizedResponse, StreamChunk};
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Gemini models available through Vertex's `publishers/google` namespace.
///
/// Kept as its own list rather than reused from `google::GOOGLE_MODELS` even though the
/// two are identical today: they are allowed to diverge (Vertex sometimes gets a model a
/// release ahead or behind AI Studio) without either provider silently changing what the
/// other accepts.
const VERTEX_MODELS: &[&str] = &[
    "gemini-2.5-flash",
    "gemini-2.5-pro",
    "gemini-2.0-flash",
    "gemini-1.5-flash",
    "gemini-1.5-pro",
    "gemini-2.5-flash-lite",
    "gemini-3.6-flash",
    "gemini-3.1-pro-preview",
];

/// Default region when the credential does not specify `aegis_region`.
///
/// `us-central1` is Google's own default in most of their client libraries and docs, and
/// has full Gemini availability, so it is the least surprising choice for a customer who
/// has not thought about region yet.
const DEFAULT_REGION: &str = "us-central1";

/// The audience Google's OAuth2 token endpoint expects in the assertion.
const TOKEN_AUD: &str = "https://oauth2.googleapis.com/token";

/// The scope requested. `cloud-platform` is broader than strictly necessary for
/// `aiplatform.generateContent` alone, but it is the scope Google's own documentation
/// recommends for Vertex service accounts, and requesting a narrower one that Google
/// changes later without notice is a worse failure mode than a broad, stable one.
const TOKEN_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// A GCP service account key, exactly as downloaded from the Cloud Console.
///
/// `#[serde(deny_unknown_fields)]` is deliberately *not* set here: Google has added
/// fields to this JSON shape before (`universe_domain` is a relatively recent one), and a
/// customer pasting a key with a field this struct does not know about should not have
/// their whole credential rejected over it. The one field we ourselves add,
/// `aegis_region`, rides in the same JSON for exactly that flexibility.
#[derive(Debug, Clone, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    project_id: String,
    #[serde(default)]
    aegis_region: Option<String>,
}

/// Claims for the self-signed JWT assertion. RFC 7523.
#[derive(Debug, Serialize)]
struct AssertionClaims {
    iss: String,
    scope: String,
    aud: String,
    /// Issued-at, Unix seconds.
    iat: i64,
    /// Expiry, Unix seconds. Google rejects an assertion lifetime over one hour.
    exp: i64,
}

/// Build and RS256-sign the JWT assertion for `sa`, valid from `now` for one hour.
///
/// Pure aside from the signature itself — no network call, no clock read beyond the `now`
/// passed in — specifically so it can be unit-tested without mocking anything.
fn build_assertion(sa: &ServiceAccountKey, now: chrono::DateTime<Utc>) -> Result<String> {
    let claims = AssertionClaims {
        iss: sa.client_email.clone(),
        scope: TOKEN_SCOPE.to_string(),
        aud: TOKEN_AUD.to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(60)).timestamp(),
    };

    let key =
        EncodingKey::from_rsa_pem(sa.private_key.as_bytes()).map_err(|e| AegisError::Provider {
            provider: "vertex".to_string(),
            status: 401,
            message: format!("the vertex credential's private_key is not a valid RSA PEM key: {e}"),
        })?;

    encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(|e| AegisError::Provider {
        provider: "vertex".to_string(),
        status: 401,
        message: format!("could not sign the vertex service-account assertion: {e}"),
    })
}

/// An access token and when it stops being safe to use.
#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    /// Cached with a safety margin already subtracted — see `get_access_token`.
    usable_until: chrono::DateTime<Utc>,
}

/// Exchange a signed assertion for an OAuth2 access token.
///
/// Split from `get_access_token` so the cache-hit path — which is what every request
/// after the first for a given service account takes — never has to be reasoned about
/// alongside HTTP error handling.
async fn exchange_token(http: &reqwest::Client, assertion: &str) -> Result<CachedToken> {
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        expires_in: i64,
    }

    let response = http
        .post(TOKEN_AUD)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion),
        ])
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AegisError::Provider {
            provider: "vertex".to_string(),
            status: 502,
            message: format!("token exchange with Google failed: {e}"),
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(AegisError::Provider {
            provider: "vertex".to_string(),
            status: status.as_u16(),
            message: format!(
                "Google rejected the service-account assertion: {}",
                super::extract_provider_error(&body)
            ),
        });
    }

    let parsed: TokenResponse = serde_json::from_str(&body).map_err(|e| AegisError::Provider {
        provider: "vertex".to_string(),
        status: 502,
        message: format!("unparseable token response: {e}"),
    })?;

    Ok(CachedToken {
        access_token: parsed.access_token,
        // Subtract a safety margin so a token is never handed out with, say, three
        // seconds of life left — enough for it to expire mid-request on a slow upstream
        // call and turn a cache *hit* into an avoidable 401.
        usable_until: Utc::now() + chrono::Duration::seconds(parsed.expires_in)
            - chrono::Duration::seconds(60),
    })
}

/// Google Vertex AI provider.
#[derive(Default)]
pub struct VertexProvider {
    /// One cached access token per service account email. A `DashMap` rather than a
    /// `Mutex<HashMap<..>>` because token refreshes for different organisations' service
    /// accounts must never serialise behind each other — that would turn one slow token
    /// exchange into added latency for every other tenant's concurrent request.
    tokens: DashMap<String, CachedToken>,
}

impl VertexProvider {
    pub fn new() -> VertexProvider {
        VertexProvider::default()
    }

    /// Parse the credential's opaque string as a service-account key.
    fn parse_credential(credential: &Credential) -> Result<ServiceAccountKey> {
        serde_json::from_str(&credential.api_key).map_err(|e| AegisError::Provider {
            provider: "vertex".to_string(),
            status: 401,
            message: format!(
                "the vertex credential is not a valid GCP service-account JSON key: {e}"
            ),
        })
    }

    /// A cached token for `sa`, minting one if there is none or the cached one is stale.
    async fn get_access_token(
        &self,
        http: &reqwest::Client,
        sa: &ServiceAccountKey,
    ) -> Result<String> {
        if let Some(cached) = self.tokens.get(&sa.client_email) {
            if cached.usable_until > Utc::now() {
                return Ok(cached.access_token.clone());
            }
        }

        let assertion = build_assertion(sa, Utc::now())?;
        let fresh = exchange_token(http, &assertion).await?;
        let access_token = fresh.access_token.clone();
        self.tokens.insert(sa.client_email.clone(), fresh);
        Ok(access_token)
    }

    /// The `generateContent` (or `streamGenerateContent`) URL for one request.
    ///
    /// Honours `credential.base_url` as a literal full override first — same meaning
    /// every other provider gives it, e.g. a VPC-SC restricted private endpoint — before
    /// falling back to the standard public regional endpoint built from the service
    /// account's own project and region.
    fn url(
        credential: &Credential,
        sa: &ServiceAccountKey,
        model: &str,
        streaming: bool,
    ) -> String {
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        let method = if streaming {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };

        if let Some(base) = credential.base_url.as_deref() {
            return format!("{}/models/{bare}:{method}", base.trim_end_matches('/'));
        }

        let region = sa.aegis_region.as_deref().unwrap_or(DEFAULT_REGION);
        format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{}/locations/{region}/publishers/google/models/{bare}:{method}",
            sa.project_id
        )
    }
}

#[async_trait]
impl Provider for VertexProvider {
    fn id(&self) -> &'static str {
        "vertex"
    }

    fn supported_models(&self) -> &[&'static str] {
        VERTEX_MODELS
    }

    /// Unused. Vertex's real base URL depends on the service account's project and
    /// region, which are only known once the credential is parsed — see `Self::url`,
    /// which both overridden methods below call directly instead of going through the
    /// trait's default `chat()`/`auth_headers()` flow. Kept non-empty so a log line that
    /// happens to print it is not misleading about what provider it belongs to.
    fn default_base_url(&self) -> &'static str {
        "https://aiplatform.googleapis.com"
    }

    fn build_body(&self, request: &NormalizedRequest, model: &str) -> serde_json::Value {
        google::build_body(request, model)
    }

    fn parse_response(&self, body: &serde_json::Value) -> Result<NormalizedResponse> {
        google::parse_response(body)
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Option<StreamChunk>> {
        google::parse_stream_chunk(data)
    }

    /// Unused — see `default_base_url`. `chat_path` alone cannot express Vertex's URL
    /// because it never receives the credential, only the model name.
    fn chat_path(&self, model: &str) -> String {
        let bare = model.split_once('/').map(|(_, m)| m).unwrap_or(model);
        format!("/publishers/google/models/{bare}:generateContent")
    }

    /// Unused — see `default_base_url`. Real auth is an async OAuth2 exchange
    /// (`get_access_token`), which cannot happen inside this synchronous, trait-required
    /// method; both `chat` and `chat_stream` are fully overridden below and never call
    /// this or the trait's default `chat()` implementation that would have called it.
    fn auth_headers(&self, _credential: &Credential) -> Vec<(String, String)> {
        Vec::new()
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
        idempotency_key: Option<&str>,
    ) -> Result<NormalizedResponse> {
        let sa = Self::parse_credential(credential)?;
        let token = self.get_access_token(http, &sa).await?;
        let url = Self::url(credential, &sa, model, false);

        let mut request_builder = http
            .post(&url)
            .timeout(timeout)
            .bearer_auth(&token)
            .json(&self.build_body(request, model));
        // Vertex's generateContent has no documented idempotency mechanism, unlike
        // OpenAI/Anthropic — sent anyway for uniformity across adapters and in case
        // Google adds support; an unrecognised header is harmless.
        if let Some(key) = idempotency_key {
            request_builder = request_builder.header("Idempotency-Key", key);
        }
        let response = request_builder.send().await.map_err(|e| {
            if e.is_timeout() {
                AegisError::ProviderTimeout(timeout.as_secs())
            } else {
                AegisError::Provider {
                    provider: "vertex".to_string(),
                    status: 502,
                    message: e.to_string(),
                }
            }
        })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(AegisError::Provider {
                provider: "vertex".to_string(),
                status: status.as_u16(),
                message: super::extract_provider_error(&body),
            });
        }

        let json: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| AegisError::Provider {
                provider: "vertex".to_string(),
                status: 502,
                message: format!("unparseable response: {e}"),
            })?;

        self.parse_response(&json)
    }

    async fn chat_stream(
        &self,
        http: &reqwest::Client,
        request: &NormalizedRequest,
        model: &str,
        credential: &Credential,
        timeout: Duration,
        idempotency_key: Option<&str>,
    ) -> Result<ChunkStream> {
        let sa = Self::parse_credential(credential)?;
        let token = self.get_access_token(http, &sa).await?;
        let url = Self::url(credential, &sa, model, true);

        super::openai::open_stream(
            http,
            &url,
            self.build_body(request, model),
            super::with_idempotency_key(
                vec![("Authorization".to_string(), format!("Bearer {token}"))],
                idempotency_key,
            ),
            "vertex",
            timeout,
            parse_stream_chunk_free,
        )
        .await
    }
}

/// Free-function wrapper so `open_stream`'s `fn(&str) -> Result<Option<StreamChunk>>`
/// function-pointer parameter can point at it — it cannot point at a trait method.
fn parse_stream_chunk_free(data: &str) -> Result<Option<StreamChunk>> {
    google::parse_stream_chunk(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use chrono::TimeZone;

    /// A syntactically valid but obviously-fake RSA key, generated solely for these
    /// tests. Not connected to any real Google Cloud project or credential.
    const TEST_PRIVATE_KEY: &str = include_str!("../../testdata/vertex_test_key.pem");

    fn test_sa() -> ServiceAccountKey {
        ServiceAccountKey {
            client_email: "aegis-test@my-project.iam.gserviceaccount.com".to_string(),
            private_key: TEST_PRIVATE_KEY.to_string(),
            project_id: "my-project".to_string(),
            aegis_region: None,
        }
    }

    #[test]
    fn the_assertion_carries_the_right_issuer_audience_and_scope() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let jwt = build_assertion(&test_sa(), now).expect("a valid key must sign cleanly");

        // Decode without verifying the signature — that would need the public key, which
        // this test does not have; it only asserts the *claims* are correct, which is
        // the part a bug in this module could actually get wrong.
        let payload = jwt.split('.').nth(1).expect("a JWT has three segments");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("the payload segment must be valid base64url");
        let claims: serde_json::Value =
            serde_json::from_slice(&decoded).expect("the payload must be JSON");

        assert_eq!(
            claims["iss"],
            "aegis-test@my-project.iam.gserviceaccount.com"
        );
        assert_eq!(claims["aud"], TOKEN_AUD);
        assert_eq!(claims["scope"], TOKEN_SCOPE);
        assert_eq!(claims["iat"], now.timestamp());
    }

    #[test]
    fn the_assertion_expires_one_hour_after_issue() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let jwt = build_assertion(&test_sa(), now).unwrap();
        let payload = jwt.split('.').nth(1).unwrap();
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&decoded).unwrap();

        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            3600,
            "Google rejects an assertion lifetime over one hour — this must never drift \
             past it"
        );
    }

    #[test]
    fn a_malformed_private_key_is_a_clear_provider_error_not_a_panic() {
        let mut sa = test_sa();
        sa.private_key = "not a real PEM key".to_string();

        let err = build_assertion(&sa, Utc::now()).unwrap_err();
        assert_eq!(err.status().as_u16(), 401);
        assert!(!format!("{err}").is_empty());
    }

    #[test]
    fn credential_parsing_rejects_a_bare_api_key_with_an_actionable_message() {
        // The single most likely mistake a customer makes: pasting an AI-Studio-style
        // API key into the Vertex credential field instead of a service-account JSON.
        let credential = Credential::new("AIzaSyD-not-a-service-account-key");
        let err = VertexProvider::parse_credential(&credential).unwrap_err();
        assert_eq!(err.status().as_u16(), 401);
        assert!(
            format!("{err}").contains("service-account"),
            "the message should point at what is actually expected"
        );
    }

    #[test]
    fn credential_parsing_accepts_a_real_shaped_service_account_json() {
        let json = serde_json::json!({
            "type": "service_account",
            "project_id": "my-project",
            "private_key": TEST_PRIVATE_KEY,
            "client_email": "aegis@my-project.iam.gserviceaccount.com",
            // A field this struct does not model. Must not cause rejection — Google has
            // added fields to this shape before without warning.
            "universe_domain": "googleapis.com",
        })
        .to_string();

        let credential = Credential::new(json);
        let sa = VertexProvider::parse_credential(&credential).expect("must parse");
        assert_eq!(sa.project_id, "my-project");
        assert_eq!(sa.client_email, "aegis@my-project.iam.gserviceaccount.com");
        assert_eq!(sa.aegis_region, None);
    }

    #[test]
    fn the_default_region_is_used_when_none_is_specified() {
        let sa = test_sa();
        let credential = Credential::new("unused-for-this-test");
        let url = VertexProvider::url(&credential, &sa, "vertex/gemini-2.5-flash", false);
        assert!(url.contains("us-central1-aiplatform.googleapis.com"));
        assert!(url.contains("/projects/my-project/"));
        assert!(url.contains("/locations/us-central1/"));
        assert!(url.ends_with(":generateContent"));
    }

    #[test]
    fn a_region_in_the_credential_json_overrides_the_default() {
        let mut sa = test_sa();
        sa.aegis_region = Some("europe-west4".to_string());
        let credential = Credential::new("unused-for-this-test");
        let url = VertexProvider::url(&credential, &sa, "vertex/gemini-2.5-flash", false);
        assert!(url.contains("europe-west4-aiplatform.googleapis.com"));
        assert!(url.contains("/locations/europe-west4/"));
    }

    #[test]
    fn streaming_uses_the_streamgeneratecontent_method_with_sse() {
        let sa = test_sa();
        let credential = Credential::new("unused-for-this-test");
        let url = VertexProvider::url(&credential, &sa, "vertex/gemini-2.5-flash", true);
        assert!(url.ends_with(":streamGenerateContent?alt=sse"));
    }

    #[test]
    fn an_explicit_base_url_override_wins_over_the_computed_endpoint() {
        // Same contract as every other provider: base_url is an escape hatch for a
        // private or VPC-SC restricted endpoint, and it must win unconditionally.
        let sa = test_sa();
        let credential = Credential::with_base_url(
            "unused-for-this-test",
            "https://private.example.internal/v1",
        );
        let url = VertexProvider::url(&credential, &sa, "vertex/gemini-2.5-flash", false);
        assert_eq!(
            url,
            "https://private.example.internal/v1/models/gemini-2.5-flash:generateContent"
        );
    }

    #[test]
    fn a_canonical_or_bare_model_id_resolves_to_the_same_bare_name_in_the_url() {
        let sa = test_sa();
        let credential = Credential::new("unused-for-this-test");
        let canonical = VertexProvider::url(&credential, &sa, "vertex/gemini-2.5-pro", false);
        let bare = VertexProvider::url(&credential, &sa, "gemini-2.5-pro", false);
        assert_eq!(canonical, bare);
    }

    #[test]
    fn supports_recognises_a_vertex_model_by_bare_or_canonical_name() {
        let provider = VertexProvider::new();
        assert!(provider.supports("vertex/gemini-2.5-flash"));
        assert!(provider.supports("gemini-2.5-flash"));
        assert!(!provider.supports("gpt-4o"));
    }

    #[test]
    fn the_provider_id_is_vertex_not_google() {
        // Guards against the two adapters ever being registered under the same id, which
        // would make ProviderRegistry::register silently overwrite one with the other.
        let provider = VertexProvider::new();
        assert_eq!(provider.id(), "vertex");
        assert_ne!(provider.id(), super::super::google::GoogleProvider.id());
    }

    #[tokio::test]
    async fn a_cached_token_is_reused_without_a_new_signature_or_exchange() {
        let provider = VertexProvider::new();
        let expires_soon = Utc::now() + chrono::Duration::minutes(30);
        provider.tokens.insert(
            "aegis-test@my-project.iam.gserviceaccount.com".to_string(),
            CachedToken {
                access_token: "cached-token-value".to_string(),
                usable_until: expires_soon,
            },
        );

        let http = reqwest::Client::new();
        let token = provider
            .get_access_token(&http, &test_sa())
            .await
            .expect("a live, unexpired cache entry must be returned without any I/O");
        assert_eq!(token, "cached-token-value");
    }

    #[test]
    fn a_request_body_and_response_parse_identically_to_the_google_adapter() {
        // The one thing this module must never do is quietly diverge from AI Studio's
        // wire format — they serve the same underlying models and are meant to share it.
        use crate::types::{Message, NormalizedRequest, Role};

        let request = NormalizedRequest {
            messages: vec![Message::text(Role::User, "2+2?")],
            ..NormalizedRequest::simple("vertex/gemini-2.5-flash", "")
        };

        let vertex_body = VertexProvider::new().build_body(&request, "vertex/gemini-2.5-flash");
        let google_body = google::build_body(&request, "google/gemini-2.5-flash");
        assert_eq!(vertex_body, google_body);
    }
}

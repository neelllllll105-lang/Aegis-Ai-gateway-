//! Enterprise HTTP surface — SCIM 2.0 provisioning and SSO.
//!
//! The logic lives in [`crate::enterprise`]; this module is the wire layer.
//!
//! # SCIM authentication is separate from everything else
//!
//! Identity providers cannot hold a session cookie and will not mint an Aegis API key.
//! They authenticate with a long-lived bearer token issued per organisation and stored
//! hashed in `scim_tokens`. That token is powerful — it can deprovision every user in the
//! org — so it is scoped to the SCIM endpoints only and never accepted anywhere else.

use crate::crypto;
use crate::db::repo;
use crate::enterprise::scim::{PatchRequest, ScimError, ScimListResponse, ScimUser};
use crate::enterprise::sso;
use crate::error::{AegisError, Result};
use crate::middleware::auth;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

/// SCIM content type, per RFC 7644.
///
/// Some providers reject `application/json` here, so it is set explicitly on every
/// response rather than relying on the axum default.
const SCIM_CONTENT_TYPE: &str = "application/scim+json";

/// Render a SCIM response with the right content type.
fn scim_response<T: serde::Serialize>(status: StatusCode, body: T) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(SCIM_CONTENT_TYPE),
    );
    response
}

/// Render a SCIM error.
fn scim_error(status: StatusCode, error: ScimError) -> Response {
    scim_response(status, error)
}

/// Resolve the organisation from a SCIM bearer token.
///
/// Deliberately does not fall back to session or API-key auth. A SCIM token can
/// deprovision every user in an organisation, so it lives in its own namespace and is
/// accepted only here.
async fn authenticate_scim(state: &AppState, headers: &HeaderMap) -> Result<Uuid> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim())
        .ok_or_else(|| AegisError::Unauthorized("SCIM requires a bearer token".into()))?;

    let pool = state.db()?;
    repo::find_org_by_scim_token(pool, &crypto::hash_token(token))
        .await?
        .ok_or_else(|| AegisError::Unauthorized("invalid SCIM token".into()))
}

/// SCIM list filter, e.g. `userName eq "jane@acme.com"`.
#[derive(Debug, Deserialize)]
pub struct ScimQuery {
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default, rename = "startIndex")]
    pub start_index: Option<usize>,
    #[serde(default)]
    pub count: Option<usize>,
}

impl ScimQuery {
    /// Extract the email from a `userName eq "..."` filter.
    ///
    /// Only equality on `userName` is supported. That is the filter identity providers
    /// actually send during provisioning; anything else returns the full list rather than
    /// silently returning nothing, which would look like the user does not exist and
    /// cause the provider to create a duplicate.
    pub fn username_filter(&self) -> Option<String> {
        let filter = self.filter.as_ref()?;
        let lowered = filter.to_ascii_lowercase();
        if !lowered.starts_with("username eq") {
            return None;
        }
        filter
            .split('"')
            .nth(1)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}

/// `GET /scim/v2/Users`
pub async fn scim_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ScimQuery>,
) -> Response {
    match async {
        let org_id = authenticate_scim(&state, &headers).await?;
        let pool = state.db()?;
        let base_url = &state.config.base_url;

        let members = repo::list_members(pool, org_id).await?;

        let filtered: Vec<ScimUser> = match query.username_filter() {
            Some(email) => members
                .into_iter()
                .filter(|m| m.email.eq_ignore_ascii_case(&email))
                .map(|m| {
                    ScimUser::from_user(m.user_id, &m.email, m.name.as_deref(), true, base_url)
                })
                .collect(),
            None => members
                .into_iter()
                .map(|m| {
                    ScimUser::from_user(m.user_id, &m.email, m.name.as_deref(), true, base_url)
                })
                .collect(),
        };

        Ok::<_, AegisError>(scim_response(
            StatusCode::OK,
            ScimListResponse::new(filtered, query.start_index.unwrap_or(1)),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => scim_error(
            e.status(),
            ScimError::new(e.status().as_u16(), None, &e.to_string()),
        ),
    }
}

/// `GET /scim/v2/Users/{id}`
pub async fn scim_get_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> Response {
    match async {
        let org_id = authenticate_scim(&state, &headers).await?;
        let pool = state.db()?;

        // Scoped by organisation: a SCIM token for one tenant must not resolve another
        // tenant's user, even given a valid id.
        let member = repo::list_members(pool, org_id)
            .await?
            .into_iter()
            .find(|m| m.user_id == user_id);

        match member {
            Some(m) => Ok::<_, AegisError>(scim_response(
                StatusCode::OK,
                ScimUser::from_user(
                    m.user_id,
                    &m.email,
                    m.name.as_deref(),
                    true,
                    &state.config.base_url,
                ),
            )),
            None => Ok(scim_error(
                StatusCode::NOT_FOUND,
                ScimError::not_found("user not found in this organisation"),
            )),
        }
    }
    .await
    {
        Ok(response) => response,
        Err(e) => scim_error(
            e.status(),
            ScimError::new(e.status().as_u16(), None, &e.to_string()),
        ),
    }
}

/// `POST /scim/v2/Users` — provision a user.
pub async fn scim_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ScimUser>,
) -> Response {
    match async {
        let org_id = authenticate_scim(&state, &headers).await?;
        let pool = state.db()?;

        let Some(email) = payload.provisioning_email() else {
            return Ok::<_, AegisError>(scim_error(
                StatusCode::BAD_REQUEST,
                ScimError::new(
                    400,
                    Some("invalidValue"),
                    "no usable email address in the payload",
                ),
            ));
        };

        // Idempotent: an identity provider retrying a create must not produce a duplicate,
        // and must not fail either — it should converge on the same membership.
        let user = match repo::find_user_by_email(pool, &email).await? {
            Some(existing) => existing,
            None => {
                repo::create_user(pool, &email, None, payload.display_name().as_deref()).await?
            }
        };

        repo::add_member(pool, org_id, user.id, "member", None).await?;

        let _ = repo::write_audit_log(
            pool,
            org_id,
            None,
            "scim.user_provisioned",
            "user",
            Some(user.id),
            Some(serde_json::json!({"email": email})),
        )
        .await;

        Ok(scim_response(
            StatusCode::CREATED,
            ScimUser::from_user(
                user.id,
                &user.email,
                user.name.as_deref(),
                true,
                &state.config.base_url,
            ),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => scim_error(
            e.status(),
            ScimError::new(e.status().as_u16(), None, &e.to_string()),
        ),
    }
}

/// `PATCH /scim/v2/Users/{id}` — the deprovisioning path.
///
/// This is the endpoint that actually matters commercially: when someone is terminated,
/// their access must disappear within minutes without anyone remembering to log in here.
pub async fn scim_patch_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
    Json(patch): Json<PatchRequest>,
) -> Response {
    match async {
        let org_id = authenticate_scim(&state, &headers).await?;
        let pool = state.db()?;

        let Some(active) = patch.active_change() else {
            // Nothing we act on — a name change, say. Return the current state rather than
            // an error, so the provider does not retry forever.
            return scim_get_user(State(state.clone()), headers.clone(), Path(user_id))
                .await
                .pipe_ok();
        };

        if active {
            repo::add_member(pool, org_id, user_id, "member", None).await?;
        } else {
            // Deactivate the membership, not the row. Usage records must keep resolving,
            // and an audit asking who ran a request in March needs the user to still exist.
            repo::remove_member(pool, org_id, user_id).await?;
            repo::revoke_keys_for_user(pool, org_id, user_id).await?;
        }

        let _ = repo::write_audit_log(
            pool,
            org_id,
            None,
            if active {
                "scim.user_activated"
            } else {
                "scim.user_deprovisioned"
            },
            "user",
            Some(user_id),
            None,
        )
        .await;

        let member = repo::list_members(pool, org_id)
            .await?
            .into_iter()
            .find(|m| m.user_id == user_id);

        Ok::<_, AegisError>(scim_response(
            StatusCode::OK,
            match member {
                Some(m) => ScimUser::from_user(
                    m.user_id,
                    &m.email,
                    m.name.as_deref(),
                    active,
                    &state.config.base_url,
                ),
                None => ScimUser::from_user(user_id, "", None, false, &state.config.base_url),
            },
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => scim_error(
            e.status(),
            ScimError::new(e.status().as_u16(), None, &e.to_string()),
        ),
    }
}

/// `DELETE /scim/v2/Users/{id}` — deactivate, never delete.
pub async fn scim_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> Response {
    match async {
        let org_id = authenticate_scim(&state, &headers).await?;
        let pool = state.db()?;

        // Remove the membership and revoke keys. The user row survives so historical
        // usage records still resolve.
        let _ = repo::remove_member(pool, org_id, user_id).await;
        repo::revoke_keys_for_user(pool, org_id, user_id).await?;

        let _ = repo::write_audit_log(
            pool,
            org_id,
            None,
            "scim.user_deprovisioned",
            "user",
            Some(user_id),
            None,
        )
        .await;

        // 204 with an empty body is what RFC 7644 specifies.
        Ok::<_, AegisError>(StatusCode::NO_CONTENT.into_response())
    }
    .await
    {
        Ok(response) => response,
        Err(e) => scim_error(
            e.status(),
            ScimError::new(e.status().as_u16(), None, &e.to_string()),
        ),
    }
}

/// `GET /scim/v2/ServiceProviderConfig` — capability discovery.
///
/// Identity providers fetch this before provisioning anything. Declaring a capability we
/// do not have causes confusing downstream failures, so this is deliberately modest.
pub async fn scim_service_provider_config() -> Response {
    scim_response(
        StatusCode::OK,
        serde_json::json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
            "documentationUri": "https://docs.aegis.dev/scim",
            "patch": {"supported": true},
            "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
            "filter": {"supported": true, "maxResults": 200},
            "changePassword": {"supported": false},
            "sort": {"supported": false},
            "etag": {"supported": false},
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "OAuth Bearer Token",
                "description": "Long-lived bearer token issued per organisation.",
            }],
        }),
    )
}

// ---------------------------------------------------------------------------
// SSO
// ---------------------------------------------------------------------------

/// `GET /api/auth/sso/start?domain=acme.com`
///
/// Resolves the organisation from the email domain and redirects to its identity
/// provider. Domain-based discovery is what lets a user click "Sign in with SSO" without
/// first knowing their organisation id.
#[derive(Debug, Deserialize)]
pub struct SsoStartQuery {
    pub domain: String,
}

/// `GET /api/auth/sso/start`
pub async fn sso_start(
    State(state): State<AppState>,
    Query(query): Query<SsoStartQuery>,
) -> Response {
    match async {
        let pool = state.db()?;
        let connection = repo::find_sso_connection_by_domain(pool, &query.domain)
            .await?
            .ok_or_else(|| {
                AegisError::NotFound(format!(
                    "no single sign-on connection is configured for {}",
                    query.domain
                ))
            })?;

        if !connection.is_active {
            return Err(AegisError::Forbidden(
                "single sign-on is disabled for this organisation".into(),
            ));
        }

        // State and nonce are single-use CSRF protection. Without state, an attacker can
        // complete their own login flow in a victim's browser.
        let state_token = crypto::generate_session_token();
        let nonce = crypto::generate_session_token();

        state
            .store
            .set_ex(
                &format!("aegis:sso:state:{}", state_token.hash),
                &connection.org_id.to_string(),
                std::time::Duration::from_secs(600),
            )
            .await?;

        let redirect_uri = format!("{}/api/auth/sso/callback", state.config.base_url);
        let url = sso::authorization_url(
            &connection.issuer,
            connection.client_id.as_deref().unwrap_or_default(),
            &redirect_uri,
            &state_token.plaintext,
            &nonce.plaintext,
        );

        Ok::<_, AegisError>(
            (StatusCode::FOUND, [(axum::http::header::LOCATION, url)]).into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/auth/sso/callback?code=...&state=...`
///
/// The other half of `sso_start`, and the one that did not exist until now.
/// `sso_start` has always constructed a `redirect_uri` pointing here; nothing was
/// registered at this path, so no SSO login could ever complete — a customer's identity
/// provider would redirect the browser to a 404. Found in the enterprise readiness audit.
///
/// # What this proves and does not prove
///
/// The full OIDC authorization-code flow is implemented: state/CSRF verification, a code
/// exchange against the provider's token endpoint, discovery of the provider's signing
/// keys, signature verification, and the same [`sso::validate`] business checks (audience,
/// issuer, expiry, domain pinning) plus replay detection that were already tested against
/// synthetic assertions. What it has **not** been run against is a real identity provider
/// — that remains true of this feature exactly as the audit found it, and is not a claim
/// this comment or this code makes otherwise.
///
/// SAML connections are explicitly rejected here rather than silently mishandled: SAML's
/// response binding (a POST of base64-encoded, XML-signed content to a different endpoint
/// shape) is a materially different implementation, not a variant of this one, and building
/// it correctly is out of scope for closing this specific gap.
#[derive(Debug, Deserialize)]
pub struct SsoCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    /// Set by the identity provider instead of `code` when the user cancels or the IdP
    /// itself refuses the request (e.g. `access_denied`).
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn sso_callback(
    State(state): State<AppState>,
    Query(query): Query<SsoCallbackQuery>,
) -> Response {
    match do_sso_callback(&state, query).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn do_sso_callback(state: &AppState, query: SsoCallbackQuery) -> Result<Response> {
    if let Some(error) = query.error {
        return Err(AegisError::Unauthorized(format!(
            "identity provider declined the request: {error}{}",
            query
                .error_description
                .map(|d| format!(" ({d})"))
                .unwrap_or_default()
        )));
    }
    let (Some(code), Some(state_param)) = (query.code, query.state) else {
        return Err(AegisError::BadRequest(
            "callback is missing code or state".into(),
        ));
    };

    // The state token is single-use: read it and delete it in the same step, so a
    // captured or retried callback URL cannot complete a second login.
    let state_key = format!("aegis:sso:state:{}", crypto::hash_token(&state_param));
    let org_id_raw = state.store.get(&state_key).await?.ok_or_else(|| {
        AegisError::Unauthorized("SSO state has expired or was already used".into())
    })?;
    state.store.del(&state_key).await?;
    let org_id = Uuid::parse_str(&org_id_raw)
        .map_err(|_| AegisError::Internal("stored SSO state was not a valid org id".into()))?;

    let pool = state.db()?;
    let connection = repo::find_sso_connection(pool, org_id)
        .await?
        .ok_or_else(|| AegisError::NotFound("no single sign-on connection is configured".into()))?;
    if !connection.is_active {
        return Err(AegisError::Forbidden(
            "single sign-on is disabled for this organisation".into(),
        ));
    }
    if connection.protocol != "oidc" {
        return Err(AegisError::BadRequest(
            "this SSO connection uses SAML, which this endpoint does not yet support; \
             contact support"
                .into(),
        ));
    }
    let client_id = connection
        .client_id
        .clone()
        .ok_or_else(|| AegisError::Internal("OIDC connection is missing a client_id".into()))?;
    let client_secret = match &connection.client_secret_encrypted {
        Some(ciphertext) => {
            let plaintext =
                crypto::decrypt(&state.config.master_key, ciphertext).map_err(|_| {
                    AegisError::Internal("SSO client secret could not be decrypted".into())
                })?;
            String::from_utf8(plaintext).map_err(|_| AegisError::Crypto)?
        }
        None => String::new(),
    };

    // ---- Code exchange ------------------------------------------------------------
    let discovery = sso::discover(&state.http, &connection.issuer).await?;
    let redirect_uri = format!("{}/api/auth/sso/callback", state.config.base_url);

    let token_response: serde_json::Value = state
        .http
        .post(&discovery.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AegisError::Provider {
            provider: "sso".into(),
            status: 502,
            message: format!("token exchange with identity provider failed: {e}"),
        })?
        .json()
        .await
        .map_err(|e| AegisError::Provider {
            provider: "sso".into(),
            status: 502,
            message: format!("identity provider's token response was not valid JSON: {e}"),
        })?;

    let id_token = token_response
        .get("id_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AegisError::Unauthorized(
                "identity provider's token response did not include an id_token".into(),
            )
        })?;

    // ---- Signature verification -----------------------------------------------------
    let header = jsonwebtoken::decode_header(id_token)
        .map_err(|e| AegisError::Unauthorized(format!("id token header was malformed: {e}")))?;
    let decoding_key =
        sso::fetch_decoding_key(&state.http, &discovery.jwks_uri, header.kid.as_deref()).await?;
    let claims = sso::verify_id_token(id_token, &decoding_key)?;

    // ---- Business validation ----------------------------------------------------------
    let assertion = sso::assertion_from_id_token(&claims, &client_id)
        .ok_or_else(|| AegisError::Unauthorized("id token was missing required claims".into()))?;

    let expected = sso::SsoConnection {
        org_id: connection.org_id,
        protocol: connection.protocol.clone(),
        expected_issuer: connection.issuer.clone(),
        expected_audience: client_id.clone(),
        allowed_email_domains: connection.email_domain.clone().into_iter().collect(),
        is_active: connection.is_active,
    };
    sso::validate(&assertion, &expected, chrono::Utc::now().timestamp())?;
    sso::check_and_record_replay(state.store.as_ref(), org_id, &assertion.id).await?;

    // ---- Session creation --------------------------------------------------------------
    // SSO authenticates an *existing* Aegis account; it does not create one. Provisioning
    // is SCIM's job — an IdP asserting an email is not by itself authorization to create
    // an account and grant it organisation membership, and conflating the two would let
    // anyone who can get one email accepted by the IdP mint themselves a seat.
    let user = repo::find_user_by_email(pool, &assertion.email)
        .await?
        .ok_or_else(|| {
            AegisError::Forbidden(format!(
                "{} is not provisioned in Aegis; ask an administrator to add this account \
                 (directly or via SCIM) before signing in with SSO",
                assertion.email
            ))
        })?;
    let member_orgs = repo::list_orgs_for_user(pool, user.id).await?;
    if !member_orgs.iter().any(|o| o.id == org_id) {
        return Err(AegisError::Forbidden(
            "this account is not a member of the organisation this SSO connection belongs \
             to"
            .into(),
        ));
    }
    if !user.is_active() {
        return Err(AegisError::Forbidden(
            "this account has been disabled".into(),
        ));
    }

    let session = crypto::generate_session_token();
    repo::create_session(
        pool,
        user.id,
        &session.hash,
        chrono::Utc::now() + chrono::Duration::seconds(auth::SESSION_DURATION.as_secs() as i64),
    )
    .await?;

    // Redirect into the dashboard rather than returning JSON: this request is a browser
    // navigation from the identity provider, not an API call a client library parses.
    let mut response = (
        StatusCode::FOUND,
        [(
            axum::http::header::LOCATION,
            format!("{}/dashboard", state.config.app_url),
        )],
    )
        .into_response();
    if let Ok(cookie) = auth::session_cookie(&session.plaintext, &state.config).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::SET_COOKIE, cookie);
    }
    Ok(response)
}

/// `GET /api/auth/sso/connections` — what SSO is configured for the acting org.
pub async fn sso_connections(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = crate::middleware::auth::authenticate_management(&state, &headers).await?;
        let pool = state.db()?;

        let connection = repo::find_sso_connection(pool, context.org_id).await?;

        Ok::<_, AegisError>(
            Json(serde_json::json!({
                "connection": connection.map(|c| serde_json::json!({
                    "protocol": c.protocol,
                    "issuer": c.issuer,
                    "email_domain": c.email_domain,
                    "is_active": c.is_active,
                    // The client secret is never returned, in any form.
                })),
                "scim_endpoint": format!("{}/scim/v2", state.config.base_url),
            }))
            .into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// Small helper so a nested handler call can be returned from the outer `async` block.
trait PipeOk {
    fn pipe_ok(self) -> Result<Response>;
}

impl PipeOk for Response {
    fn pipe_ok(self) -> Result<Response> {
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_filters_are_parsed() {
        let query = ScimQuery {
            filter: Some(r#"userName eq "jane@acme.com""#.to_string()),
            start_index: None,
            count: None,
        };
        assert_eq!(query.username_filter().as_deref(), Some("jane@acme.com"));
    }

    #[test]
    fn username_filter_parsing_is_case_insensitive_on_the_attribute() {
        // Providers disagree on capitalisation of the attribute name.
        for filter in [
            r#"userName eq "a@b.com""#,
            r#"username eq "a@b.com""#,
            r#"USERNAME EQ "a@b.com""#,
        ] {
            let query = ScimQuery {
                filter: Some(filter.to_string()),
                start_index: None,
                count: None,
            };
            assert_eq!(
                query.username_filter().as_deref(),
                Some("a@b.com"),
                "{filter}"
            );
        }
    }

    #[test]
    fn an_unsupported_filter_returns_the_full_list_rather_than_nothing() {
        // Returning nothing would look like "user does not exist" and make the provider
        // create a duplicate.
        let query = ScimQuery {
            filter: Some(r#"emails.value co "acme.com""#.to_string()),
            start_index: None,
            count: None,
        };
        assert_eq!(query.username_filter(), None);
    }

    #[test]
    fn an_absent_or_malformed_filter_yields_nothing() {
        for filter in [None, Some(String::new()), Some("userName eq".to_string())] {
            let query = ScimQuery {
                filter,
                start_index: None,
                count: None,
            };
            assert_eq!(query.username_filter(), None);
        }
    }

    #[tokio::test]
    async fn scim_requires_a_bearer_token() {
        let state = AppState::for_tests();
        let err = authenticate_scim(&state, &HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(err.error_type(), "unauthorized");
        assert!(format!("{err}").contains("bearer"));
    }

    #[tokio::test]
    async fn scim_does_not_accept_a_session_cookie() {
        // A SCIM token can deprovision every user in an org, so it lives in its own
        // namespace. Session auth must not be a way in here.
        let state = AppState::for_tests();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            axum::http::HeaderValue::from_static("aegis_session=sometoken"),
        );
        assert!(authenticate_scim(&state, &headers).await.is_err());
    }

    #[tokio::test]
    async fn the_service_provider_config_declares_only_what_we_support() {
        // Declaring a capability we lack produces confusing downstream failures.
        let response = scim_service_provider_config().await;
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(response.into_body(), 65_536)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["patch"]["supported"], true);
        assert_eq!(json["bulk"]["supported"], false);
        assert_eq!(json["sort"]["supported"], false);
        assert_eq!(json["changePassword"]["supported"], false);
    }

    #[tokio::test]
    async fn scim_responses_use_the_scim_content_type() {
        // Some providers reject application/json here.
        let response = scim_service_provider_config().await;
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert_eq!(content_type, SCIM_CONTENT_TYPE);
    }

    #[tokio::test]
    async fn sso_start_without_a_configured_domain_fails_clearly() {
        let state = AppState::for_tests();
        let response = sso_start(
            State(state),
            Query(SsoStartQuery {
                domain: "unknown.example".to_string(),
            }),
        )
        .await;
        // No database in tests, so this surfaces as an error rather than a redirect —
        // the point is that it never redirects somewhere unconfigured.
        assert_ne!(response.status(), StatusCode::FOUND);
    }
}

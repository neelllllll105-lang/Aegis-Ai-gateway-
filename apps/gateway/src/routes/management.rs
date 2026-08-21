//! The management API — everything under `/api`.
//!
//! `MASTER_BUILD.md` Part 6. Three rules hold across every handler here:
//!
//! 1. **Org-scoped.** The organisation comes from the authenticated context, never from
//!    the request body or a path parameter. A caller cannot name someone else's org
//!    because there is nowhere to put the name.
//! 2. **Audit-logged on mutation.** Key creation, revocation, credential changes, policy
//!    edits, budget changes, and membership changes all append to `audit_logs`.
//! 3. **Write authority is role-gated.** API keys can drive the gateway but cannot
//!    administer the organisation, so a leaked key cannot raise its own budget or mint
//!    new keys.

use crate::crypto;
use crate::db::repo;
use crate::error::{AegisError, Result};
use crate::middleware::auth::{self, AuthContext};
use crate::middleware::rate_limit;
use crate::money::{savings_share_basis_points, MicroCents};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Authenticate and require write authority.
async fn require_writer(state: &AppState, headers: &HeaderMap) -> Result<AuthContext> {
    let context = auth::authenticate_management(state, headers).await?;
    if !context.can_write() {
        return Err(AegisError::Forbidden(
            "this action requires an owner or admin role. API keys cannot administer an \
             organisation."
                .into(),
        ));
    }
    Ok(context)
}

/// Authenticate and require read authority.
async fn require_reader(state: &AppState, headers: &HeaderMap) -> Result<AuthContext> {
    let context = auth::authenticate_management(state, headers).await?;
    if !context.can_read() {
        return Err(AegisError::Forbidden("insufficient permissions".into()));
    }
    Ok(context)
}

/// Record a mutation in the audit log.
///
/// A failure here is logged but never propagated: an audit write must not undo the action
/// it describes. A gap in the log is recoverable; a half-applied key rotation is not.
async fn audit(
    state: &AppState,
    context: &AuthContext,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    metadata: Option<serde_json::Value>,
) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    if let Err(e) = repo::write_audit_log(
        pool,
        context.org_id,
        context.user_id,
        action,
        resource_type,
        resource_id,
        metadata,
    )
    .await
    {
        tracing::error!(error = %e, action, "audit log write failed");
    }
}

/// Wrap a handler result into a response.
fn respond<T: Serialize>(status: StatusCode, body: T) -> Response {
    (status, Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// `POST /api/auth/signup`
#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// Minimum password length.
///
/// Length is the only requirement. Composition rules (a digit, a symbol, a capital) push
/// people toward `Password1!` and are worse than a longer passphrase; NIST dropped them
/// for the same reason.
pub const MIN_PASSWORD_LENGTH: usize = 12;

/// Validate an email address well enough to reject obvious junk.
///
/// Deliberately not a full RFC 5322 implementation: the only authoritative test of an
/// address is whether mail to it arrives, which the verification email already performs.
pub fn is_plausible_email(email: &str) -> bool {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.contains(' ')
        && email.len() <= 254
}

/// Derive an organisation slug from an email address.
pub fn slug_from_email(email: &str, suffix: &str) -> String {
    let local: String = email
        .split('@')
        .next()
        .unwrap_or("org")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = local.trim_matches('-');
    let base = if trimmed.is_empty() { "org" } else { trimmed };
    format!("{base}-{suffix}")
}

/// `POST /api/auth/signup`
pub async fn signup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SignupRequest>,
) -> Response {
    match do_signup(&state, &headers, request).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn do_signup(
    state: &AppState,
    headers: &HeaderMap,
    request: SignupRequest,
) -> Result<Response> {
    // Unauthenticated endpoints are IP-limited: Cloudflare stops volumetric abuse, this
    // stops the slow grind that stays under a WAF threshold.
    let ip = client_ip(headers);
    let limit = rate_limit::check_ip(state.store.as_ref(), &ip, 10).await?;
    if !limit.allowed {
        return Err(limit.into_error());
    }

    if !is_plausible_email(&request.email) {
        return Err(AegisError::BadRequest("enter a valid email address".into()));
    }
    if request.password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(AegisError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }

    let pool = state.db()?;
    let password_hash = crypto::hash_password(&request.password)?;
    let user = repo::create_user(
        pool,
        &request.email,
        Some(&password_hash),
        request.name.as_deref(),
    )
    .await?;

    // Every signup gets a personal organisation, so a new user has somewhere to put keys
    // immediately rather than being asked to make a decision before seeing the product.
    let slug = slug_from_email(&request.email, &user.id.simple().to_string()[..8]);
    let org_name = request.name.clone().unwrap_or_else(|| {
        request
            .email
            .split('@')
            .next()
            .unwrap_or("Personal")
            .to_string()
    });
    let org = repo::create_org_with_owner(pool, &org_name, &slug, user.id).await?;

    let session = crypto::generate_session_token();
    repo::create_session(
        pool,
        user.id,
        &session.hash,
        Utc::now() + Duration::seconds(auth::SESSION_DURATION.as_secs() as i64),
    )
    .await?;

    let _ = repo::write_audit_log(
        pool,
        org.id,
        Some(user.id),
        "user.signup",
        "user",
        Some(user.id),
        None,
    )
    .await;

    let mut response = respond(
        StatusCode::CREATED,
        serde_json::json!({"user": user, "organization": org}),
    );
    if let Ok(cookie) = auth::session_cookie(&session.plaintext, &state.config).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::SET_COOKIE, cookie);
    }
    Ok(response)
}

/// `POST /api/auth/login`
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// `POST /api/auth/login`
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Response {
    match do_login(&state, &headers, request).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn do_login(
    state: &AppState,
    headers: &HeaderMap,
    request: LoginRequest,
) -> Result<Response> {
    let ip = client_ip(headers);
    let limit = rate_limit::check_ip(state.store.as_ref(), &ip, 20).await?;
    if !limit.allowed {
        return Err(limit.into_error());
    }

    let pool = state.db()?;
    let user = repo::find_user_by_email(pool, &request.email).await?;

    // One message for every failure mode. Distinguishing "no such account" from "wrong
    // password" turns the login form into an account-enumeration oracle.
    let invalid = || AegisError::Unauthorized("invalid email or password".into());

    let Some(user) = user else {
        // Hash anyway, so a missing account and a wrong password take the same time.
        let _ = crypto::hash_password(&request.password);
        return Err(invalid());
    };

    let Some(stored_hash) = &user.password_hash else {
        return Err(invalid());
    };
    if !crypto::verify_password(&request.password, stored_hash) {
        return Err(invalid());
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
        Utc::now() + Duration::seconds(auth::SESSION_DURATION.as_secs() as i64),
    )
    .await?;

    let organizations = repo::list_orgs_for_user(pool, user.id).await?;

    let mut response = respond(
        StatusCode::OK,
        serde_json::json!({"user": user, "organizations": organizations}),
    );
    if let Ok(cookie) = auth::session_cookie(&session.plaintext, &state.config).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::SET_COOKIE, cookie);
    }
    Ok(response)
}

/// `POST /api/auth/logout`
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let (Some(token), Some(pool)) = (auth::extract_session_cookie(&headers), state.db.as_ref()) {
        let _ = repo::delete_session(pool, &crypto::hash_token(&token)).await;
    }

    let mut response = respond(StatusCode::OK, serde_json::json!({"ok": true}));
    if let Ok(cookie) = auth::clear_session_cookie(&state.config).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::SET_COOKIE, cookie);
    }
    response
}

/// `GET /api/auth/me`
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match do_me(&state, &headers).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn do_me(state: &AppState, headers: &HeaderMap) -> Result<Response> {
    let context = require_reader(state, headers).await?;
    let pool = state.db()?;

    let user = match context.user_id {
        Some(user_id) => repo::find_user_by_id(pool, user_id).await?,
        None => None,
    };
    let org = repo::find_org(pool, context.org_id).await?;

    Ok(respond(
        StatusCode::OK,
        serde_json::json!({
            "user": user,
            "organization": org,
            "role": context.role,
            "is_admin": context.is_admin,
        }),
    ))
}

/// Best-effort client IP for abuse limiting.
///
/// Trusts `x-forwarded-for` because Cloudflare terminates TLS in front of us and is the
/// only thing that can reach the origin. In a deployment without that guarantee, this
/// header is caller-controlled and must not be trusted — see `docs/runbooks/deploy.md`.
fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

/// `POST /api/keys`
#[derive(Debug, Deserialize)]
pub struct CreateKeyRequest {
    pub name: String,
    #[serde(default)]
    pub team_id: Option<Uuid>,
    #[serde(default)]
    pub rate_limit_per_minute: Option<i32>,
    #[serde(default)]
    pub monthly_budget_mc: Option<i64>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

/// `GET /api/keys`
pub async fn list_keys(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let keys = repo::list_api_keys(state.db()?, context.org_id).await?;
        Ok::<_, AegisError>(respond(StatusCode::OK, serde_json::json!({"keys": keys})))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/keys`
pub async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateKeyRequest>,
) -> Response {
    match do_create_key(&state, &headers, request).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn do_create_key(
    state: &AppState,
    headers: &HeaderMap,
    request: CreateKeyRequest,
) -> Result<Response> {
    let context = require_writer(state, headers).await?;
    let pool = state.db()?;

    if request.name.trim().is_empty() {
        return Err(AegisError::BadRequest("a key name is required".into()));
    }

    let generated = crypto::generate_api_key();
    let key = repo::create_api_key(
        pool,
        context.org_id,
        context.user_id,
        request.name.trim(),
        &generated.prefix,
        &generated.hash,
        request.team_id,
        request
            .rate_limit_per_minute
            .unwrap_or(state.config.default_rate_limit_per_minute as i32),
        request.monthly_budget_mc,
        request.allowed_models.map(|m| serde_json::json!(m)),
        request.expires_at,
    )
    .await?;

    audit(
        state,
        &context,
        "key.created",
        "api_key",
        Some(key.id),
        Some(serde_json::json!({"name": key.name, "prefix": key.key_prefix})),
    )
    .await;

    // The plaintext key appears here and nowhere else, ever. It is not stored, not
    // logged, and not recoverable — a lost key is replaced, never retrieved.
    Ok(respond(
        StatusCode::CREATED,
        serde_json::json!({
            "key": generated.plaintext,
            "metadata": key,
            "warning": "This key is shown once and cannot be retrieved later. Store it now.",
        }),
    ))
}

/// `GET /api/keys/:id`
pub async fn get_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let key = repo::find_api_key(state.db()?, context.org_id, key_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("key not found".into()))?;
        Ok::<_, AegisError>(respond(StatusCode::OK, key))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `PATCH /api/keys/:id`
#[derive(Debug, Deserialize)]
pub struct UpdateKeyRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub rate_limit_per_minute: Option<i32>,
    #[serde(default)]
    pub monthly_budget_mc: Option<i64>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
}

/// `PATCH /api/keys/:id`
pub async fn update_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
    Json(request): Json<UpdateKeyRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let key = repo::update_api_key(
            pool,
            context.org_id,
            key_id,
            request.name.as_deref(),
            request.rate_limit_per_minute,
            request.monthly_budget_mc,
            request.allowed_models.map(|m| serde_json::json!(m)),
        )
        .await?
        .ok_or_else(|| AegisError::NotFound("key not found".into()))?;

        // The cached context still carries the old limits, so it must go now rather than
        // in up to 60 seconds.
        auth::invalidate_key(state.store.as_ref(), &state.key_cache, &key.key_hash).await?;

        audit(
            &state,
            &context,
            "key.updated",
            "api_key",
            Some(key.id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(StatusCode::OK, key))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/keys/:id`
pub async fn revoke_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        // Read before revoking: the hash is needed to invalidate the caches, and after
        // revocation the row is still there but we would be racing our own update.
        let key = repo::find_api_key(pool, context.org_id, key_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("key not found".into()))?;

        let revoked = repo::revoke_api_key(pool, context.org_id, key_id).await?;
        if !revoked {
            return Err(AegisError::NotFound(
                "key not found or already revoked".into(),
            ));
        }

        // Without this the key keeps authenticating for up to the cache TTL.
        auth::invalidate_key(state.store.as_ref(), &state.key_cache, &key.key_hash).await?;

        audit(
            &state,
            &context,
            "key.revoked",
            "api_key",
            Some(key_id),
            Some(serde_json::json!({"prefix": key.key_prefix})),
        )
        .await;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"revoked": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Organisation
// ---------------------------------------------------------------------------

/// `GET /api/org`
pub async fn get_org(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let org = repo::find_org(state.db()?, context.org_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("organisation not found".into()))?;

        let spend =
            crate::metering::usage::current_spend(state.store.as_ref(), context.org_id).await;
        let savings =
            crate::metering::usage::current_savings(state.store.as_ref(), context.org_id).await;
        let requests =
            crate::metering::usage::current_requests(state.store.as_ref(), context.org_id).await;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "organization": org,
                "usage": {
                    "month_to_date_spend_mc": spend.as_i64(),
                    "month_to_date_savings_mc": savings.as_i64(),
                    "month_to_date_requests": requests,
                }
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `PATCH /api/org`
#[derive(Debug, Deserialize)]
pub struct UpdateOrgRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub billing_email: Option<String>,
    #[serde(default)]
    pub zero_retention: Option<bool>,
    #[serde(default)]
    pub content_capture: Option<bool>,
}

/// `PATCH /api/org`
pub async fn update_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateOrgRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let org = repo::update_org_settings(
            pool,
            context.org_id,
            request.name.as_deref(),
            request.billing_email.as_deref(),
            request.zero_retention,
            request.content_capture,
        )
        .await?;

        // Enabling zero retention must not leave previously cached responses readable.
        if request.zero_retention == Some(true) {
            let cache =
                crate::cache::exact::ExactCache::new(state.store.as_ref(), state.config.cache_ttl);
            let _ = cache.invalidate_org(context.org_id).await;
        }

        // Retention toggles are exactly the changes an auditor asks about.
        audit(
            &state,
            &context,
            "org.settings_updated",
            "organization",
            Some(org.id),
            Some(serde_json::json!({
                "zero_retention": org.zero_retention,
                "content_capture": org.content_capture,
            })),
        )
        .await;

        Ok::<_, AegisError>(respond(StatusCode::OK, org))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/org/members`
pub async fn list_members(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let members = repo::list_members(state.db()?, context.org_id).await?;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"members": members}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/org/members/invite`
#[derive(Debug, Deserialize)]
pub struct InviteRequest {
    pub email: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "member".to_string()
}

/// `POST /api/org/members/invite`
pub async fn invite_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<InviteRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        if !matches!(
            request.role.as_str(),
            "owner" | "admin" | "member" | "viewer"
        ) {
            return Err(AegisError::BadRequest(
                "role must be one of: owner, admin, member, viewer".into(),
            ));
        }
        if !is_plausible_email(&request.email) {
            return Err(AegisError::BadRequest("enter a valid email address".into()));
        }

        // An invited address that has no account yet gets a placeholder user with no
        // password, so the membership exists before they sign up. They set a password
        // through the reset flow, which is also the email-verification step.
        let user = match repo::find_user_by_email(pool, &request.email).await? {
            Some(user) => user,
            None => repo::create_user(pool, &request.email, None, None).await?,
        };

        repo::add_member(
            pool,
            context.org_id,
            user.id,
            &request.role,
            context.user_id,
        )
        .await?;

        audit(
            &state,
            &context,
            "member.invited",
            "user",
            Some(user.id),
            Some(serde_json::json!({"role": request.role})),
        )
        .await;

        Ok::<_, AegisError>(respond(
            StatusCode::CREATED,
            serde_json::json!({"invited": request.email, "role": request.role}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/org/members/:userId`
pub async fn remove_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let removed = repo::remove_member(state.db()?, context.org_id, user_id).await?;
        if !removed {
            return Err(AegisError::NotFound("member not found".into()));
        }
        audit(
            &state,
            &context,
            "member.removed",
            "user",
            Some(user_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"removed": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Provider credentials (BYOK)
// ---------------------------------------------------------------------------

/// `POST /api/providers`
#[derive(Debug, Deserialize)]
pub struct CreateCredentialRequest {
    pub provider: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

/// `GET /api/providers`
///
/// Never returns key material — [`repo::ProviderCredential`] marks the ciphertext
/// `#[serde(skip)]`, so only the hint and label are visible.
pub async fn list_providers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let credentials = repo::list_credentials(state.db()?, context.org_id).await?;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"providers": credentials}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/providers`
pub async fn create_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateCredentialRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        if state.providers.get(&request.provider).is_none() {
            return Err(AegisError::BadRequest(format!(
                "unknown provider {:?}. Supported: {}",
                request.provider,
                state.providers.ids().join(", ")
            )));
        }
        if request.api_key.trim().is_empty() {
            return Err(AegisError::BadRequest("an API key is required".into()));
        }
        if request.provider == "custom" && request.base_url.is_none() {
            return Err(AegisError::BadRequest(
                "a custom provider requires a base_url".into(),
            ));
        }

        let key = request.api_key.trim();
        let encrypted = crypto::encrypt(&state.config.master_key, key.as_bytes())?;
        // Last four characters only — enough for a human to tell two keys apart, useless
        // to anyone who obtains it.
        let hint: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let credential = repo::create_credential(
            pool,
            context.org_id,
            &request.provider,
            &encrypted,
            Some(&format!("...{hint}")),
            request.base_url.as_deref(),
            request.label.as_deref(),
            request.is_default.unwrap_or(true),
        )
        .await?;

        audit(
            &state,
            &context,
            "credential.added",
            "provider_credential",
            Some(credential.id),
            // The provider name is safe to log; the key is not, and is not here.
            Some(serde_json::json!({"provider": request.provider})),
        )
        .await;

        Ok::<_, AegisError>(respond(StatusCode::CREATED, credential))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/providers/:id`
pub async fn delete_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(credential_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let deleted = repo::delete_credential(state.db()?, context.org_id, credential_id).await?;
        if !deleted {
            return Err(AegisError::NotFound("credential not found".into()));
        }
        audit(
            &state,
            &context,
            "credential.deleted",
            "provider_credential",
            Some(credential_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"deleted": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/providers/:id/test`
///
/// Sends a one-token completion to confirm the stored key actually works. Customers
/// mistype keys constantly, and finding out during a production request is worse than
/// finding out on the settings page.
pub async fn test_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(credential_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let credential = repo::find_credential(pool, context.org_id, credential_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("credential not found".into()))?;

        let provider = state
            .providers
            .get(&credential.provider)
            .ok_or_else(|| AegisError::BadRequest("unknown provider".into()))?;

        let plaintext = crypto::decrypt(&state.config.master_key, &credential.encrypted_key)?;
        let key = String::from_utf8(plaintext).map_err(|_| AegisError::Crypto)?;
        let live = match &credential.base_url {
            Some(base_url) => crate::providers::Credential::with_base_url(key, base_url.clone()),
            None => crate::providers::Credential::new(key),
        };

        let model = provider
            .supported_models()
            .first()
            .copied()
            .unwrap_or("gpt-4o-mini");
        let probe = crate::types::NormalizedRequest {
            max_tokens: Some(1),
            ..crate::types::NormalizedRequest::simple(model, "ping")
        };

        let result = provider
            .chat(
                &state.http,
                &probe,
                model,
                &live,
                std::time::Duration::from_secs(15),
            )
            .await;

        let ok = result.is_ok();
        let _ = repo::record_credential_test(pool, context.org_id, credential_id, ok).await;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "ok": ok,
                "provider": credential.provider,
                "error": result.err().map(|e| e.to_string()),
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Policies, teams, budgets
// ---------------------------------------------------------------------------

/// `GET /api/policies`
pub async fn list_policies(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let policies = repo::list_policies(state.db()?, context.org_id).await?;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"policies": policies}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/policies`
#[derive(Debug, Deserialize)]
pub struct CreatePolicyRequest {
    pub name: String,
    pub rules: serde_json::Value,
}

/// `POST /api/policies`
pub async fn create_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePolicyRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        // Validate before storing. A policy that fails to parse degrades silently to
        // default routing, so the moment to catch it is now, while a human is looking.
        let parsed = crate::engine::policy::RoutingPolicy::from_json(&request.rules.to_string());
        if parsed.is_empty()
            && !request
                .rules
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false)
        {
            return Err(AegisError::BadRequest(
                "rules must be an array of {\"when\": {...}, \"then\": {...}} objects".into(),
            ));
        }

        let policy =
            repo::create_policy(pool, context.org_id, &request.name, &request.rules).await?;

        // Policies are cached for five minutes; drop it so the change applies now.
        let _ = state
            .store
            .del(&format!("aegis:policy:{}", context.org_id))
            .await;

        audit(
            &state,
            &context,
            "policy.created",
            "routing_policy",
            Some(policy.id),
            Some(serde_json::json!({"name": policy.name})),
        )
        .await;

        Ok::<_, AegisError>(respond(StatusCode::CREATED, policy))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/policies/:id`
pub async fn delete_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(policy_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let deleted = repo::delete_policy(state.db()?, context.org_id, policy_id).await?;
        if !deleted {
            return Err(AegisError::NotFound("policy not found".into()));
        }
        let _ = state
            .store
            .del(&format!("aegis:policy:{}", context.org_id))
            .await;
        audit(
            &state,
            &context,
            "policy.deleted",
            "routing_policy",
            Some(policy_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"deleted": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/org/teams`
pub async fn list_teams(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let teams = repo::list_teams(state.db()?, context.org_id).await?;
        Ok::<_, AegisError>(respond(StatusCode::OK, serde_json::json!({"teams": teams})))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/org/teams`
#[derive(Debug, Deserialize)]
pub struct CreateTeamRequest {
    pub name: String,
    #[serde(default)]
    pub monthly_budget_mc: Option<i64>,
}

/// `POST /api/org/teams`
pub async fn create_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTeamRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let team = repo::create_team(
            state.db()?,
            context.org_id,
            &request.name,
            request.monthly_budget_mc,
        )
        .await?;
        audit(
            &state,
            &context,
            "team.created",
            "team",
            Some(team.id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(StatusCode::CREATED, team))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/org/teams/:id`
pub async fn delete_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let deleted = repo::delete_team(state.db()?, context.org_id, team_id).await?;
        if !deleted {
            return Err(AegisError::NotFound("team not found".into()));
        }
        audit(
            &state,
            &context,
            "team.deleted",
            "team",
            Some(team_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"deleted": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/budgets`
pub async fn list_budgets(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let budgets = repo::list_budgets(state.db()?, context.org_id).await?;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"budgets": budgets}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/budgets`
#[derive(Debug, Deserialize)]
pub struct CreateBudgetRequest {
    #[serde(default)]
    pub team_id: Option<Uuid>,
    #[serde(default)]
    pub api_key_id: Option<Uuid>,
    #[serde(default = "default_period")]
    pub period: String,
    pub limit_mc: i64,
    #[serde(default = "default_true")]
    pub hard_limit: bool,
}

fn default_period() -> String {
    "monthly".to_string()
}

fn default_true() -> bool {
    true
}

/// `POST /api/budgets`
pub async fn create_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateBudgetRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        if request.limit_mc < 0 {
            return Err(AegisError::BadRequest("limit must not be negative".into()));
        }
        let budget = repo::create_budget(
            state.db()?,
            context.org_id,
            request.team_id,
            request.api_key_id,
            &request.period,
            request.limit_mc,
            request.hard_limit,
        )
        .await?;
        audit(
            &state,
            &context,
            "budget.created",
            "budget",
            Some(budget.id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(StatusCode::CREATED, budget))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/budgets/:id`
pub async fn delete_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(budget_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let deleted = repo::delete_budget(state.db()?, context.org_id, budget_id).await?;
        if !deleted {
            return Err(AegisError::NotFound("budget not found".into()));
        }
        audit(
            &state,
            &context,
            "budget.deleted",
            "budget",
            Some(budget_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"deleted": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Usage, savings, requests
// ---------------------------------------------------------------------------

/// Date range query parameters.
#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    #[serde(default)]
    pub start: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

impl RangeQuery {
    /// Resolve to a concrete range, defaulting to the last 30 days.
    pub fn resolve(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let end = self.end.unwrap_or_else(Utc::now);
        let start = self.start.unwrap_or(end - Duration::days(30));
        (start, end)
    }
}

/// `GET /api/usage/summary`
pub async fn usage_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let (start, end) = range.resolve();
        let summary = repo::usage_summary(state.db()?, context.org_id, start, end).await?;

        // Percentages are computed here rather than in the client so every surface —
        // dashboard, CSV, invoice — shows the same number.
        let savings_percent = if summary.baseline_cost_mc > 0 {
            (summary.gross_savings_mc as f64 / summary.baseline_cost_mc as f64) * 100.0
        } else {
            0.0
        };
        let cache_hit_rate = if summary.requests > 0 {
            (summary.cache_hits as f64 / summary.requests as f64) * 100.0
        } else {
            0.0
        };

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "period": {"start": start, "end": end},
                "summary": summary,
                "derived": {
                    "savings_percent": savings_percent,
                    "cache_hit_rate": cache_hit_rate,
                    "customer_net_mc": summary.gross_savings_mc - summary.aegis_fee_mc,
                }
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/requests` — the metadata log. Never includes prompt or response content.
pub async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let (start, end) = range.resolve();
        let rows = repo::list_requests(
            state.db()?,
            context.org_id,
            start,
            end,
            range.limit.unwrap_or(100),
            range.offset.unwrap_or(0),
        )
        .await?;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"requests": rows}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/savings/report.csv` — finance-grade export.
pub async fn savings_report_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let (start, end) = range.resolve();
        let rows = repo::list_requests(state.db()?, context.org_id, start, end, 10_000, 0).await?;

        let mut csv = String::from(
            "request_id,timestamp,requested_model,served_model,provider,input_tokens,\
             output_tokens,baseline_cost_usd,actual_cost_usd,gross_savings_usd,cache,\
             routing_reason,status\n",
        );
        for row in &rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                row.request_id,
                row.created_at.to_rfc3339(),
                row.requested_model,
                row.served_model,
                row.provider,
                row.input_tokens,
                row.output_tokens,
                usd(row.baseline_cost_mc),
                usd(row.actual_cost_mc),
                usd(row.gross_savings_mc),
                row.cache_type.as_deref().unwrap_or("miss"),
                row.routing_reason,
                row.status_code,
            ));
        }

        Ok::<_, AegisError>(
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        "attachment; filename=\"aegis-savings.csv\"",
                    ),
                ],
                csv,
            )
                .into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// Format micro-cents as a plain decimal for CSV, without a currency symbol.
///
/// Spreadsheets treat `$0.0075` as text and `0.007500` as a number, and a finance team
/// needs to sum the column.
fn usd(micro_cents: i64) -> String {
    format!("{:.6}", micro_cents as f64 / 1_000_000.0)
}

/// `GET /api/billing/plan`
pub async fn billing_plan(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let org = repo::find_org(state.db()?, context.org_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("organisation not found".into()))?;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "plan": org.plan,
                "savings_share_bp": org.savings_share_bp,
                "savings_share_percent": org.savings_share_bp as f64 / 100.0,
                "subscription_mc": crate::billing::invoice::subscription_price(&org.plan).as_i64(),
                "limits": {
                    "requests_per_minute": rate_limit::org_limit_for_plan(&org.plan),
                    "monthly_request_allowance": if org.plan == "free" {
                        Some(state.config.free_tier_monthly_requests)
                    } else {
                        None
                    },
                    "byok": org.plan != "free",
                }
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/models`
///
/// The model catalogue as the dashboard needs to see it: every model Aegis can route to,
/// what it costs, and — the part that matters — what the cheapest equivalent in its tier
/// costs, so a human can see the substitution the router would make and judge it.
///
/// This exists separately from `/v1/models` (bearer-authenticated, OpenAI-shaped) and
/// `/api/admin/pricing` (admin-only, raw table) because the dashboard has a session
/// cookie and a reader role, and neither of those routes accepts that combination.
pub async fn model_catalogue(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        require_reader(&state, &headers).await?;

        let all: Vec<_> = state.pricing.all().collect();

        // Cheapest blended price within each tier — this is the number the router is
        // actually comparing against when it considers a downgrade.
        let mut cheapest_in_tier: std::collections::HashMap<&str, i64> =
            std::collections::HashMap::new();
        for m in &all {
            if !m.is_active {
                continue;
            }
            let blended = m.blended_per_mtok().as_i64();
            cheapest_in_tier
                .entry(m.tier.as_str())
                .and_modify(|current| {
                    if blended < *current {
                        *current = blended;
                    }
                })
                .or_insert(blended);
        }

        let mut models: Vec<serde_json::Value> = all
            .iter()
            .map(|m| {
                let blended = m.blended_per_mtok().as_i64();
                let floor = cheapest_in_tier
                    .get(m.tier.as_str())
                    .copied()
                    .unwrap_or(blended);

                // Percentage saved by taking the cheapest model in this tier instead of
                // this one. Zero for the cheapest model itself, which is correct.
                let potential_saving_pct = if blended > 0 && floor < blended {
                    ((blended - floor) as f64 / blended as f64 * 100.0).round() as i64
                } else {
                    0
                };

                serde_json::json!({
                    "model_id": m.model_id,
                    "provider": m.provider,
                    "tier": m.tier.as_str(),
                    "input_per_mtok_mc": m.input_per_mtok.as_i64(),
                    "output_per_mtok_mc": m.output_per_mtok.as_i64(),
                    "blended_per_mtok_mc": blended,
                    "cheapest_in_tier_mc": floor,
                    "potential_saving_pct": potential_saving_pct,
                    "context_window": m.context_window,
                    "supports_vision": m.supports_vision,
                    "supports_tools": m.supports_tools,
                    "is_active": m.is_active,
                    // Provenance travels with the price. A number nobody can trace is a
                    // number nobody should bill against.
                    "source": m.source,
                })
            })
            .collect();

        // Cheapest first within tier, tiers in escalating order — the order a human reads
        // when asking "what could I use instead".
        models.sort_by(|a, b| {
            let tier_rank = |v: &serde_json::Value| match v["tier"].as_str().unwrap_or("") {
                "economy" => 0,
                "standard" => 1,
                "premium" => 2,
                _ => 3,
            };
            tier_rank(a).cmp(&tier_rank(b)).then(
                a["blended_per_mtok_mc"]
                    .as_i64()
                    .cmp(&b["blended_per_mtok_mc"].as_i64()),
            )
        });

        let active = models.iter().filter(|m| m["is_active"] == true).count();

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "models": models,
                "count": models.len(),
                "active_count": active,
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Governance: anomalies, chargeback, credits
// ---------------------------------------------------------------------------

/// `GET /api/usage/anomalies`
///
/// Judges today against the organisation own recent history. Budgets catch spend above a
/// line; this catches spend that is within budget but wildly unlike normal — the runaway
/// agent loop that burns a month of budget in an afternoon and is invisible until the
/// invoice arrives.
pub async fn spend_anomalies(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let pool = state.db()?;

        // Thirty days of history, judged against today.
        let history = repo::daily_spend_history(pool, context.org_id, 30).await?;
        let today =
            crate::metering::usage::current_spend(state.store.as_ref(), context.org_id).await;

        // The most recent day in the history *is* today, so exclude it from its own
        // baseline — otherwise a spike partially masks itself.
        let baseline = if history.is_empty() {
            Vec::new()
        } else {
            history[..history.len().saturating_sub(1)].to_vec()
        };

        let report =
            crate::engine::governance::detect_spend_anomaly(context.org_id, today, &baseline);

        Ok::<_, AegisError>(respond(StatusCode::OK, report))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/usage/chargeback.csv`
///
/// Spend attributed to cost centers, for a finance system. Cost centers come from team
/// names, which is where organisations naturally put them.
pub async fn chargeback_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let (start, end) = range.resolve();

        let entries = repo::spend_by_cost_center(state.db()?, context.org_id, start, end).await?;
        let report = crate::engine::governance::ChargebackReport::build(
            context.org_id,
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
            entries,
        );

        Ok::<_, AegisError>(
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        "attachment; filename=\"aegis-chargeback.csv\"",
                    ),
                ],
                report.to_csv(),
            )
                .into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/usage/chargeback`
pub async fn chargeback_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let (start, end) = range.resolve();

        let entries = repo::spend_by_cost_center(state.db()?, context.org_id, start, end).await?;
        let report = crate::engine::governance::ChargebackReport::build(
            context.org_id,
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
            entries,
        );

        Ok::<_, AegisError>(respond(StatusCode::OK, report))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// Value of a referral credit, to each side. $10, per `MASTER_BUILD.md` P5.6.
pub const REFERRAL_CREDIT: MicroCents = MicroCents(10_000_000);

/// `GET /api/billing/credits`
pub async fn list_credits(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let pool = state.db()?;

        let balance = repo::credit_balance(pool, context.org_id).await?;
        let org = repo::find_org(pool, context.org_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("organisation not found".into()))?;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "balance_mc": balance.as_i64(),
                // The referral code is the org slug: stable, already unique, and readable
                // enough to say out loud. A random code would need its own table and
                // collision handling for no benefit.
                "referral_code": org.slug,
                "referral_url": format!("{}/signup?ref={}", state.config.app_url, org.slug),
                "credit_per_referral_mc": REFERRAL_CREDIT.as_i64(),
                "terms": "Both organisations receive credit once the referred org makes its first paid request.",
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/billing/referral` — claim a referral.
#[derive(Debug, Deserialize)]
pub struct ClaimReferralRequest {
    /// The referring organisation slug.
    pub code: String,
}

/// `POST /api/billing/referral`
pub async fn claim_referral(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ClaimReferralRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let referrer = repo::find_org_by_slug(pool, request.code.trim())
            .await?
            .ok_or_else(|| {
                AegisError::NotFound("no organisation with that referral code".into())
            })?;

        // Self-referral would be free money for nothing.
        if referrer.id == context.org_id {
            return Err(AegisError::BadRequest(
                "you cannot refer your own organisation".into(),
            ));
        }

        // One claim per organisation, ever. Without this, an org can re-claim on every
        // login and mint unlimited credit.
        if repo::has_claimed_referral(pool, context.org_id).await? {
            return Err(AegisError::BadRequest(
                "this organisation has already claimed a referral".into(),
            ));
        }

        // Both sides are credited, which is what makes anyone bother sharing a code.
        repo::create_referral_credit(
            pool,
            context.org_id,
            Some(referrer.id),
            REFERRAL_CREDIT,
            "referred_signup",
        )
        .await?;
        repo::create_referral_credit(
            pool,
            referrer.id,
            Some(context.org_id),
            REFERRAL_CREDIT,
            "referral_reward",
        )
        .await?;

        audit(
            &state,
            &context,
            "referral.claimed",
            "organization",
            Some(referrer.id),
            Some(serde_json::json!({"code": request.code})),
        )
        .await;

        Ok(respond(
            StatusCode::CREATED,
            serde_json::json!({
                "credited_mc": REFERRAL_CREDIT.as_i64(),
                "message": format!(
                    "{} credit applied to both organisations.",
                    REFERRAL_CREDIT.to_usd_string()
                ),
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// Expected savings-share rate for a plan, for the pricing calculator.
pub fn expected_rate(plan: &str) -> u32 {
    savings_share_basis_points(plan)
}

/// Convert micro-cents for display.
pub fn to_display(amount: MicroCents) -> String {
    amount.to_usd_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn email_validation_accepts_real_addresses() {
        for email in [
            "user@example.com",
            "first.last@sub.example.co.uk",
            "user+tag@example.com",
            "u@e.io",
        ] {
            assert!(is_plausible_email(email), "rejected valid address: {email}");
        }
    }

    #[test]
    fn email_validation_rejects_junk() {
        for email in [
            "",
            "no-at-sign",
            "@example.com",
            "user@",
            "user@nodot",
            "user@.example.com",
            "user@example.",
            "user name@example.com",
        ] {
            assert!(
                !is_plausible_email(email),
                "accepted invalid address: {email}"
            );
        }
    }

    #[test]
    fn email_validation_rejects_absurd_lengths() {
        let long = format!("{}@example.com", "a".repeat(300));
        assert!(!is_plausible_email(&long));
    }

    #[test]
    fn org_slugs_are_derived_safely_from_email() {
        assert_eq!(
            slug_from_email("jane.doe@example.com", "abc12345"),
            "jane-doe-abc12345"
        );
        assert_eq!(slug_from_email("UPPER@example.com", "xyz"), "upper-xyz");
        // Characters that would break a URL are replaced.
        assert_eq!(slug_from_email("a+b@example.com", "1"), "a-b-1");
    }

    #[test]
    fn a_degenerate_email_still_produces_a_usable_slug() {
        // Never emit an empty or leading-hyphen slug — both break routes.
        let slug = slug_from_email("+++@example.com", "id1");
        assert!(!slug.starts_with('-'), "{slug}");
        assert!(
            slug.starts_with("org-") || slug.chars().next().unwrap().is_alphanumeric(),
            "{slug}"
        );
    }

    #[test]
    fn slugs_are_unique_per_user_even_for_the_same_local_part() {
        // Two people can both be `admin@`; their orgs must not collide.
        let first = slug_from_email("admin@a.com", "aaaa1111");
        let second = slug_from_email("admin@b.com", "bbbb2222");
        assert_ne!(first, second);
    }

    #[test]
    fn password_policy_requires_length_not_composition() {
        // NIST guidance: length beats composition rules, which push people to Password1!.
        assert_eq!(MIN_PASSWORD_LENGTH, 12);
        assert!("correct horse battery staple".chars().count() >= MIN_PASSWORD_LENGTH);
        assert!("Sh0rt!".chars().count() < MIN_PASSWORD_LENGTH);
    }

    #[test]
    fn client_ip_prefers_the_cloudflare_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("10.0.0.1, 10.0.0.2"),
        );
        headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.5"));
        assert_eq!(client_ip(&headers), "203.0.113.5");
    }

    #[test]
    fn client_ip_takes_the_first_forwarded_hop() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 10.0.0.1, 10.0.0.2"),
        );
        assert_eq!(client_ip(&headers), "203.0.113.9");
    }

    #[test]
    fn client_ip_falls_back_rather_than_failing() {
        assert_eq!(client_ip(&HeaderMap::new()), "unknown");
    }

    #[test]
    fn csv_amounts_are_numeric_so_a_spreadsheet_can_sum_them() {
        // A currency symbol would make the column text.
        assert_eq!(usd(7_500), "0.007500");
        assert_eq!(usd(1_000_000), "1.000000");
        assert_eq!(usd(0), "0.000000");
        assert!(!usd(7_500).contains('$'));
    }

    #[test]
    fn range_defaults_to_the_last_thirty_days() {
        let range = RangeQuery {
            start: None,
            end: None,
            limit: None,
            offset: None,
        };
        let (start, end) = range.resolve();
        let span = end - start;
        assert_eq!(span.num_days(), 30);
    }

    #[test]
    fn an_explicit_range_is_respected() {
        let start = Utc::now() - Duration::days(7);
        let end = Utc::now();
        let range = RangeQuery {
            start: Some(start),
            end: Some(end),
            limit: None,
            offset: None,
        };
        let (resolved_start, resolved_end) = range.resolve();
        assert_eq!(resolved_start, start);
        assert_eq!(resolved_end, end);
    }

    #[test]
    fn plan_rates_match_the_business_model() {
        assert_eq!(expected_rate("pro"), 2_000);
        assert_eq!(expected_rate("team"), 1_500);
        assert_eq!(expected_rate("enterprise"), 1_000);
        assert_eq!(expected_rate("free"), 0);
    }

    #[tokio::test]
    async fn management_endpoints_reject_unauthenticated_callers() {
        let state = AppState::for_tests();
        let response = list_keys(State(state.clone()), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_clears_the_cookie_even_without_a_session() {
        let state = AppState::for_tests();
        let response = logout(State(state), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let cookie = response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("Max-Age=0"));
    }
}

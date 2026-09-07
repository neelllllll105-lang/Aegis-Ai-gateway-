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
use crate::middleware::ssrf_guard;
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

/// Authenticate and require key management authority (owner, admin, or member).
async fn require_key_writer(state: &AppState, headers: &HeaderMap) -> Result<AuthContext> {
    let context = auth::authenticate_management(state, headers).await?;
    let role = context.role.as_deref().unwrap_or("");
    if !matches!(role, "owner" | "admin" | "member") {
        return Err(AegisError::Forbidden(
            "this action requires an owner, admin, or member role.".into(),
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

/// Require that the calling organisation's plan includes `feature`, on top of whatever role
/// check already ran (`require_writer`, most often — a plan restriction and a role
/// restriction are independent: a Free-plan owner is still an owner, just not on a plan
/// that includes this).
///
/// Returns `AegisError::PlanRestricted` (403, carrying an `upgrade_url`) rather than
/// silently allowing a write the dashboard would never have shown a control for. Before
/// this existed, nothing on the backend checked plan at all for any of the endpoints that
/// call this — hiding the page in the UI was the only restriction, which any direct API
/// call bypassed entirely. See `billing::features` for the plan/feature mapping.
async fn require_plan_feature(
    state: &AppState,
    org_id: Uuid,
    feature: crate::billing::features::Feature,
) -> Result<()> {
    let org = repo::find_org(state.db()?, org_id)
        .await?
        .ok_or_else(|| AegisError::NotFound("organisation not found".into()))?;
    if crate::billing::features::plan_includes(&org.plan, feature) {
        Ok(())
    } else {
        Err(AegisError::PlanRestricted {
            feature: feature.as_str().to_string(),
            required_plan: feature.min_plan().as_str().to_string(),
        })
    }
}

/// The shared guard for every project(team)-scoped endpoint.
///
/// Org owner/admin may access any team in their organisation. A team **lead**
/// (`team_memberships.role = 'lead'`) may access only the team(s) they lead. Everyone
/// else — an ordinary org member, a lead of a *different* team, a caller from another
/// organisation entirely — gets **404, not 403**: a project-scoped resource must not even
/// confirm it exists to someone with no standing to ask.
///
/// This exists as one function, called at the top of every project-scoped handler, per
/// IG-1 §1.4: a handler that hand-rolled this check inline instead is exactly the shape of
/// bug that let scope checks silently drift out of sync across endpoints before.
async fn assert_project_access(
    state: &AppState,
    context: &AuthContext,
    team_id: Uuid,
) -> Result<()> {
    let pool = state.db()?;
    if !repo::team_belongs_to_org(pool, team_id, context.org_id).await? {
        return Err(AegisError::NotFound("project not found".into()));
    }
    if context.can_write() {
        // Org owner/admin. `can_write` is false for a bare API key, so a leaked key can
        // never claim project-lead authority it was never assigned.
        return Ok(());
    }
    let Some(user_id) = context.user_id else {
        return Err(AegisError::NotFound("project not found".into()));
    };
    match repo::role_in_team(pool, team_id, user_id).await? {
        Some(role) if role == "lead" => Ok(()),
        _ => Err(AegisError::NotFound("project not found".into())),
    }
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
    /// Required when the account has TOTP enabled. Absent for every account that does
    /// not, which is most of them — this field does not change the shape of an ordinary
    /// login.
    #[serde(default)]
    pub totp_code: Option<String>,
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

    // Two-factor. The algorithm (`enterprise::totp`), the schema columns, and this exact
    // check were all previously unconnected: `totp_enabled` was read onto `User` but
    // nothing in the login path looked at it, so a stolen password was fully sufficient
    // regardless of whether the account owner believed 2FA protected them. Found in the
    // enterprise readiness audit.
    if user.totp_enabled {
        let encrypted_secret = repo::get_totp_secret(pool, user.id).await?.ok_or_else(|| {
            // totp_enabled with no secret is a data inconsistency, not a normal
            // "no code supplied" case — enable_totp is only ever called right after a
            // secret is confirmed. Fail closed rather than silently skip the check.
            tracing::error!(user_id = %user.id, "totp_enabled but no secret is stored");
            AegisError::Internal("account's two-factor configuration is inconsistent".into())
        })?;
        let user_key = crypto::derive_user_key(&state.config.master_key, &user.id.to_string());
        let secret = crypto::decrypt(&user_key, &encrypted_secret)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| AegisError::Internal("could not decrypt TOTP secret".into()))?;

        let code = request.totp_code.as_deref().unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if !crate::enterprise::totp::verify_code(&secret, code, now) {
            // Same status as a wrong password (see AegisError::TotpRequired's own doc
            // comment) — the client distinguishes the two by error.type, not by whether a
            // guessed password alone gets a different response than a right one.
            return Err(AegisError::TotpRequired);
        }
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

// ---------------------------------------------------------------------------
// Two-factor authentication (TOTP)
//
// Self-service on the caller's *own* account, deliberately not gated by organisation
// role: a member and an owner both have exactly the same reason to protect their own
// login, and there is no tenant-scoping question here at all — nothing here reads or
// writes another account's row.
// ---------------------------------------------------------------------------

/// Require a session (not an API key) and return the authenticated context.
///
/// TOTP protects an *account's login*, which an API key was never involved in issuing —
/// accepting one here would let a leaked API key, a strictly lower-privilege credential by
/// design, manage the very control meant to protect against a leaked credential.
async fn require_session(state: &AppState, headers: &HeaderMap) -> Result<AuthContext> {
    let context = auth::authenticate_management(state, headers).await?;
    if context.user_id.is_none() {
        return Err(AegisError::Forbidden(
            "two-factor settings require a dashboard session, not an API key".into(),
        ));
    }
    Ok(context)
}

/// `POST /api/auth/totp/enroll`
///
/// Generates a new secret and returns the provisioning URI and the raw secret (for manual
/// entry, the same one-time-reveal pattern as an API key). Enrollment does **not** turn
/// enforcement on — [`totp_confirm`] does, once the caller proves they can actually
/// generate a matching code. Storing and enabling in the same step would let a bare call
/// to this endpoint — no proof the caller has the authenticator app running — lock the
/// account's own owner out immediately.
pub async fn totp_enroll(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_session(&state, &headers).await?;
        let user_id = context.user_id.expect("checked by require_session");
        let pool = state.db()?;
        let user = repo::find_user_by_id(pool, user_id)
            .await?
            .ok_or_else(|| AegisError::Unauthorized("session user no longer exists".into()))?;

        let secret = crate::enterprise::totp::generate_secret();
        let user_key = crypto::derive_user_key(&state.config.master_key, &user_id.to_string());
        let encrypted = crypto::encrypt(&user_key, secret.as_bytes())?;
        repo::set_totp_secret(pool, user_id, &encrypted).await?;

        let uri = crate::enterprise::totp::provisioning_uri(&secret, &user.email, "Aegis");
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "secret": secret,
                "provisioning_uri": uri,
                "note": "scan provisioning_uri with an authenticator app, then POST the \
                         code it shows to /api/auth/totp/confirm to finish enabling \
                         two-factor. Enrolling again before confirming replaces this secret.",
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct TotpConfirmRequest {
    pub code: String,
}

/// `POST /api/auth/totp/confirm`
///
/// Proves the caller actually has the enrolled secret before enforcement turns on.
pub async fn totp_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TotpConfirmRequest>,
) -> Response {
    match async {
        let context = require_session(&state, &headers).await?;
        let user_id = context.user_id.expect("checked by require_session");
        let pool = state.db()?;

        let encrypted = repo::get_totp_secret(pool, user_id).await?.ok_or_else(|| {
            AegisError::BadRequest(
                "no TOTP secret is pending; call /api/auth/totp/enroll first".into(),
            )
        })?;
        let user_key = crypto::derive_user_key(&state.config.master_key, &user_id.to_string());
        let secret = crypto::decrypt(&user_key, &encrypted)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| AegisError::Internal("could not decrypt TOTP secret".into()))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if !crate::enterprise::totp::verify_code(&secret, &request.code, now) {
            return Err(AegisError::Unauthorized(
                "that code does not match — check the time on your device and try again".into(),
            ));
        }

        repo::enable_totp(pool, user_id).await?;
        audit(
            &state,
            &context,
            "totp.enabled",
            "user",
            Some(user_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"totp_enabled": true}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct TotpDisableRequest {
    pub password: String,
}

/// `POST /api/auth/totp/disable`
///
/// Requires the account password again, not just an active session — a session already
/// past a TOTP-protected login is exactly the credential an attacker who stole it would
/// use to turn the protection back off, so disabling it demands proof independent of the
/// session itself.
pub async fn totp_disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TotpDisableRequest>,
) -> Response {
    match async {
        let context = require_session(&state, &headers).await?;
        let user_id = context.user_id.expect("checked by require_session");
        let pool = state.db()?;
        let user = repo::find_user_by_id(pool, user_id)
            .await?
            .ok_or_else(|| AegisError::Unauthorized("session user no longer exists".into()))?;

        let invalid = || AegisError::Unauthorized("incorrect password".into());
        let stored_hash = user.password_hash.as_ref().ok_or_else(invalid)?;
        if !crypto::verify_password(&request.password, stored_hash) {
            return Err(invalid());
        }

        repo::disable_totp(pool, user_id).await?;
        audit(
            &state,
            &context,
            "totp.disabled",
            "user",
            Some(user_id),
            None,
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"totp_enabled": false}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
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
    /// Issue this key to a named person, so their spend can be attributed to them.
    ///
    /// Omit for a shared project or service key. Only an owner or admin may set it — a
    /// member cannot mint a key in someone else's name.
    #[serde(default)]
    pub assigned_to_user_id: Option<Uuid>,
}

/// `GET /api/keys`
///
/// An owner or admin sees every key in the organisation. Anyone else sees the keys issued
/// to them plus the org's shared, unassigned keys.
///
/// The narrower view exists because keys now carry an assignee: once a key names a person
/// and carries their personal budget, "list every key" hands one colleague another
/// colleague's spending limit. That was defensible when a key belonged only to an
/// organisation and is not defensible now.
pub async fn list_keys(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let pool = state.db()?;

        // `can_write` is the owner/admin test the rest of the management API uses, and it
        // is false for API-key callers by construction — so a leaked key cannot enumerate
        // the organisation's people through this route either.
        let keys = if context.can_write() {
            repo::list_api_keys(pool, context.org_id).await?
        } else {
            match context.user_id {
                Some(user_id) => {
                    repo::list_api_keys_for_member(pool, context.org_id, user_id).await?
                }
                // A caller with no user identity and no admin rights — an API key acting
                // on its own behalf. It may see the shared keys and nothing personal.
                None => repo::list_api_keys_for_member(pool, context.org_id, Uuid::nil()).await?,
            }
        };

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
    let context = require_key_writer(state, headers).await?;
    let pool = state.db()?;

    if request.name.trim().is_empty() {
        return Err(AegisError::BadRequest("a key name is required".into()));
    }

    // Assigning a key to someone else is an administrative act: the assignee's spend will
    // be attributed to them and their personal budget enforced against them, so a member
    // must not be able to mint a key in a colleague's name. Assigning a key to *yourself*
    // is always allowed.
    let assignee = match request.assigned_to_user_id {
        None => None,
        Some(target) => {
            if !context.can_write() && context.user_id != Some(target) {
                return Err(AegisError::Forbidden(
                    "only an owner or admin can issue a key in another person's name".into(),
                ));
            }
            // The assignee must actually belong to this organisation. Without this an
            // administrator could attribute spend to a user id from another tenant, which
            // would put one org's identifier on another org's billing record.
            if repo::role_in_org(pool, context.org_id, target)
                .await?
                .is_none()
            {
                return Err(AegisError::BadRequest(
                    "the assignee is not a member of this organisation".into(),
                ));
            }
            Some(target)
        }
    };

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
        assignee,
    )
    .await?;

    audit(
        state,
        &context,
        "key.created",
        "api_key",
        Some(key.id),
        Some(serde_json::json!({
            "name": key.name,
            "prefix": key.key_prefix,
            // Who the key was issued to, so the audit trail explains whose spend this key
            // will produce — not just that a key appeared.
            "assigned_to_user_id": assignee,
        })),
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
    /// Mode this key falls back to when the caller sends no `X-Aegis-Routing-Hint` header.
    /// One of `passthrough`/`quality`/`balanced`/`economy`/`auto` — `auto` is also how an
    /// operator clears a previously-set default, since it resolves identically to unset.
    #[serde(default)]
    pub default_routing_mode: Option<String>,
}

/// The five routing-mode strings the ladder recognises.
const VALID_ROUTING_MODES: [&str; 5] = ["passthrough", "quality", "balanced", "economy", "auto"];

/// Validate a stored default-routing-mode value and return its canonical lowercase form.
///
/// `RoutingHint::parse` (what reads this value back at request time) is case-insensitive
/// on purpose — a customer's own request header must not fail on a case typo. But the
/// database's `CHECK` constraint (migration 0012) only admits the exact lowercase strings,
/// so an admin saving `"Balanced"` here would otherwise hit an opaque constraint-violation
/// 500 instead of the same forgiving behaviour the header gets. Normalising here closes
/// that gap with a clean 400 for anything genuinely unrecognised, and silent lowercasing
/// for anything that's merely differently cased.
fn validate_routing_mode(mode: &str) -> Result<String> {
    let normalized = mode.trim().to_ascii_lowercase();
    if VALID_ROUTING_MODES.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(AegisError::BadRequest(format!(
            "default_routing_mode must be one of {VALID_ROUTING_MODES:?}, got {mode:?}"
        )))
    }
}

/// `PATCH /api/keys/:id`
pub async fn update_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
    Json(request): Json<UpdateKeyRequest>,
) -> Response {
    match async {
        let context = require_key_writer(&state, &headers).await?;
        let pool = state.db()?;

        let default_routing_mode = request
            .default_routing_mode
            .as_deref()
            .map(validate_routing_mode)
            .transpose()?;

        let key = repo::update_api_key(
            pool,
            context.org_id,
            key_id,
            request.name.as_deref(),
            request.rate_limit_per_minute,
            request.monthly_budget_mc,
            request.allowed_models.map(|m| serde_json::json!(m)),
            default_routing_mode.as_deref(),
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
        let context = require_key_writer(&state, &headers).await?;
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
    /// Organisation-wide default routing mode — the last rung before `auto`. See
    /// [`UpdateKeyRequest::default_routing_mode`].
    #[serde(default)]
    pub default_routing_mode: Option<String>,
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

        let default_routing_mode = request
            .default_routing_mode
            .as_deref()
            .map(validate_routing_mode)
            .transpose()?;

        let org = repo::update_org_settings(
            pool,
            context.org_id,
            request.name.as_deref(),
            request.billing_email.as_deref(),
            request.zero_retention,
            request.content_capture,
            default_routing_mode.as_deref(),
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

        // Generate a single-use 7-day invite token and store it in auth_tokens
        let token = crypto::generate_session_token();
        let expires_at = Utc::now() + Duration::days(7);
        repo::create_auth_token(pool, user.id, &token.hash, "invite", expires_at).await?;

        let org = repo::find_org(pool, context.org_id).await?;
        let org_name = org.as_ref().map(|o| o.name.as_str()).unwrap_or("Aegis");

        let encoded_email = url::form_urlencoded::byte_serialize(request.email.as_bytes()).collect::<String>();
        let base_url = headers
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(state.config.app_url.as_str());
        let invite_url = format!("{}/join?token={}&email={}", base_url, token.plaintext, encoded_email);

        let subject = format!("You've been invited to join {} on Aegis", org_name);
        let text = format!(
            "You have been invited to join {org_name} on Aegis with the role of {role}.\n\n\
             Click the link below to set your password and access your account:\n\
             {invite_url}\n\n\
             This link expires in 7 days.",
            role = request.role
        );
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<body style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #f8fafc; padding: 40px 20px;">
  <div style="max-width: 520px; margin: 0 auto; background: #1e293b; border-radius: 16px; border: 1px solid #334155; padding: 32px; box-shadow: 0 4px 6px -1px rgba(0,0,0,0.1);">
    <div style="font-size: 20px; font-weight: 700; color: #38bdf8; margin-bottom: 8px;">Aegis</div>
    <h1 style="font-size: 22px; font-weight: 700; color: #f8fafc; margin: 0 0 16px;">Join {org_name} on Aegis</h1>
    <p style="font-size: 15px; color: #94a3b8; line-height: 1.6; margin: 0 0 24px;">
      You have been invited to collaborate with the role of <strong style="color: #f8fafc;">{role}</strong>. Set your password to activate your account and access the dashboard.
    </p>
    <div style="margin: 28px 0;">
      <a href="{invite_url}" style="display: inline-block; background: #0284c7; color: #ffffff; padding: 12px 24px; border-radius: 10px; font-size: 14px; font-weight: 600; text-decoration: none;">
        Accept Invitation &amp; Set Password &rarr;
      </a>
    </div>
    <p style="font-size: 13px; color: #64748b; margin-top: 28px; border-top: 1px solid #334155; padding-top: 20px;">
      Or copy this URL into your browser:<br>
      <a href="{invite_url}" style="color: #38bdf8; word-break: break-all;">{invite_url}</a>
    </p>
    <p style="font-size: 12px; color: #475569; margin-top: 12px;">This invitation link will expire in 7 days.</p>
  </div>
</body>
</html>"#,
            org_name = org_name,
            role = request.role,
            invite_url = invite_url
        );

        let email_sent = crate::workers::budget_alerts::send_email_full(
            &state.http,
            &state.config,
            &request.email,
            &subject,
            &text,
            Some(&html),
        )
        .await
        .unwrap_or(false);

        Ok::<_, AegisError>(respond(
            StatusCode::CREATED,
            serde_json::json!({
                "invited": request.email,
                "role": request.role,
                "invite_url": invite_url,
                "email_sent": email_sent,
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/auth/accept-invite`
#[derive(Debug, Deserialize)]
pub struct AcceptInviteRequest {
    pub token: String,
    pub password: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn accept_invite(
    State(state): State<AppState>,
    Json(request): Json<AcceptInviteRequest>,
) -> Response {
    match do_accept_invite(&state, request).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn do_accept_invite(state: &AppState, request: AcceptInviteRequest) -> Result<Response> {
    if request.password.len() < MIN_PASSWORD_LENGTH {
        return Err(AegisError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }

    let pool = state.db()?;
    let token_hash = crypto::hash_token(&request.token);
    let user_id = repo::consume_auth_token(pool, &token_hash, "invite")
        .await?
        .ok_or_else(|| {
            AegisError::BadRequest(
                "invalid or expired invitation link. Please request a new invite.".into(),
            )
        })?;

    let hash = crypto::hash_password(&request.password)?;
    repo::update_password(pool, user_id, &hash).await?;

    // Mark email as verified and update name if supplied
    sqlx::query(
        "UPDATE users SET email_verified_at = NOW(), name = COALESCE($2, name), updated_at = NOW() WHERE id = $1",
    )
    .bind(user_id)
    .bind(&request.name)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;

    let user = repo::find_user_by_id(pool, user_id)
        .await?
        .ok_or_else(|| AegisError::NotFound("user not found".into()))?;

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
        serde_json::json!({
            "user": user,
            "organizations": organizations,
            "message": "Account activated successfully"
        }),
    );

    if let Ok(cookie) = auth::session_cookie(&session.plaintext, &state.config).parse() {
        response
            .headers_mut()
            .insert(axum::http::header::SET_COOKIE, cookie);
    }

    Ok(response)
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

// ---------------------------------------------------------------------------
// SCIM token self-service
//
// `repo::create_scim_token` existed and was tested; nothing in the management API ever
// called it, so an organisation wanting SCIM provisioning had no way to get a token
// without us running a direct database write on their behalf. Found in the enterprise
// readiness audit.
// ---------------------------------------------------------------------------

/// `POST /api/scim-tokens`
///
/// Requires owner or admin, the same bar as every other action that changes how the
/// organisation can be administered — a SCIM token can deprovision every member, which is
/// a strictly more powerful action than most things `require_writer` already gates.
pub async fn create_scim_token(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let generated = crypto::generate_scim_token();
        let token_id = repo::create_scim_token(pool, context.org_id, &generated.hash).await?;
        audit(
            &state,
            &context,
            "scim_token.created",
            "scim_token",
            Some(token_id),
            None,
        )
        .await;

        Ok::<_, AegisError>(respond(
            StatusCode::CREATED,
            serde_json::json!({
                "id": token_id,
                "token": generated.plaintext,
                "note": "shown once — store it now. Configure it as the bearer token in \
                         your identity provider's SCIM connector.",
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/scim-tokens`
pub async fn list_scim_tokens(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let pool = state.db()?;
        let tokens = repo::list_scim_tokens(pool, context.org_id).await?;
        Ok::<_, AegisError>(respond(StatusCode::OK, tokens))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/scim-tokens/:id`
pub async fn revoke_scim_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let revoked = repo::revoke_scim_token(pool, context.org_id, token_id).await?;
        if !revoked {
            return Err(AegisError::NotFound(
                "SCIM token not found or already revoked".into(),
            ));
        }
        audit(
            &state,
            &context,
            "scim_token.revoked",
            "scim_token",
            Some(token_id),
            None,
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

/// `POST /api/providers`
pub async fn create_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateCredentialRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        require_plan_feature(
            &state,
            context.org_id,
            crate::billing::features::Feature::Byok,
        )
        .await?;
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
        // Every adapter honours base_url as a literal override, which makes it a
        // server-side-request-forgery vector against the gateway's own infrastructure —
        // cloud metadata services, internal admin panels, anything reachable from where
        // this process runs — for any org that can write a credential, which today means
        // any organisation at all, free tier included. See middleware/ssrf_guard.rs.
        if let Some(base_url) = request.base_url.as_deref() {
            ssrf_guard::validate_base_url(base_url).await?;
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

        let mut candidates: Vec<String> = Vec::new();

        // 1. Try querying the provider's live /models endpoint to discover the account's accessible active models
        let base_url = credential
            .base_url
            .as_deref()
            .unwrap_or_else(|| provider.default_base_url());
        let models_url = format!("{}/models", base_url.trim_end_matches('/'));
        let mut req = state
            .http
            .get(&models_url)
            .timeout(std::time::Duration::from_secs(5));
        for (k, v) in provider.auth_headers(&live) {
            req = req.header(k, v);
        }

        if let Ok(res) = req.send().await {
            if res.status().is_success() {
                if let Ok(json) = res.json::<serde_json::Value>().await {
                    // Standard OpenAI format: { "data": [ { "id": "..." } ] }
                    if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
                        for item in arr {
                            if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                                if !id.contains("embed")
                                    && !id.contains("whisper")
                                    && !id.contains("tts")
                                    && !id.contains("guard")
                                    && !id.contains("moderation")
                                    && !id.contains("dall-e")
                                {
                                    candidates.push(id.to_string());
                                }
                            }
                        }
                    }
                    // Google format: { "models": [ { "name": "models/..." } ] }
                    if let Some(arr) = json.get("models").and_then(|d| d.as_array()) {
                        for item in arr {
                            if let Some(name) = item.get("name").and_then(|i| i.as_str()) {
                                let bare = name.strip_prefix("models/").unwrap_or(name);
                                if !bare.contains("embed")
                                    && !bare.contains("aqa")
                                    && !bare.contains("imagen")
                                {
                                    candidates.push(bare.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Fall back to static supported models if /models was not supported
        if candidates.is_empty() {
            for m in provider.supported_models() {
                if !m.contains("embed") {
                    candidates.push(m.to_string());
                }
            }
        }

        let mut ok = false;
        let mut last_err = None;

        for model in &candidates {
            let probe = crate::types::NormalizedRequest {
                max_tokens: Some(10),
                ..crate::types::NormalizedRequest::simple(model, "ping")
            };

            let result = provider
                .chat(
                    &state.http,
                    &probe,
                    model,
                    &live,
                    std::time::Duration::from_secs(10),
                    None,
                )
                .await;

            tracing::info!(
                provider = %credential.provider,
                model = %model,
                ok = result.is_ok(),
                "credential test probe attempt"
            );

            match result {
                Ok(_) => {
                    ok = true;
                    last_err = None;
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        provider = %credential.provider,
                        model = %model,
                        error = %e,
                        "credential test probe model failed"
                    );
                    last_err = Some(e);
                }
            }
        }

        let _ = repo::record_credential_test(pool, context.org_id, credential_id, ok).await;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "ok": ok,
                "provider": credential.provider,
                "error": last_err.map(|e| e.to_string()),
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
        require_plan_feature(
            &state,
            context.org_id,
            crate::billing::features::Feature::Policies,
        )
        .await?;
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
    /// This project's default routing mode, for keys that set none of their own. See
    /// [`UpdateKeyRequest::default_routing_mode`].
    #[serde(default)]
    pub default_routing_mode: Option<String>,
}

/// `POST /api/org/teams`
pub async fn create_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTeamRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        require_plan_feature(
            &state,
            context.org_id,
            crate::billing::features::Feature::TeamManagement,
        )
        .await?;
        let default_routing_mode = request
            .default_routing_mode
            .as_deref()
            .map(validate_routing_mode)
            .transpose()?;
        let team = repo::create_team(
            state.db()?,
            context.org_id,
            &request.name,
            request.monthly_budget_mc,
            default_routing_mode.as_deref(),
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

#[derive(Debug, Deserialize)]
pub struct UpdateTeamRequest {
    pub name: String,
}

/// `PATCH /api/org/teams/:id` — rename a project. This is the only field a project's
/// identity has that changes independently of its usage: `usage_summary_for_team` and
/// `project_usage` are both keyed by `team_id`, never by name, so a rename touches zero
/// rows in `usage_records` — the dashboard's analytics for this project keep working under
/// the new label the instant it re-fetches the team list.
pub async fn update_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<Uuid>,
    Json(request): Json<UpdateTeamRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let name = request.name.trim();
        if name.is_empty() {
            return Err(AegisError::BadRequest(
                "a project name must not be empty".into(),
            ));
        }
        let team = repo::update_team(state.db()?, context.org_id, team_id, name)
            .await?
            .ok_or_else(|| AegisError::NotFound("team not found".into()))?;
        audit(
            &state,
            &context,
            "team.renamed",
            "team",
            Some(team.id),
            Some(serde_json::json!({"name": team.name})),
        )
        .await;
        Ok::<_, AegisError>(respond(StatusCode::OK, team))
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

/// `GET /api/org/teams/{id}/members` — a project's roster. Reachable by anyone with
/// project access (org admin, or the team's own lead) — see [`assert_project_access`].
pub async fn list_team_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<Uuid>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        assert_project_access(&state, &context, team_id).await?;
        let members = repo::list_team_members(state.db()?, team_id).await?;
        Ok::<_, AegisError>(respond(StatusCode::OK, members))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/org/teams/{id}/members` — add a person to a project, or change their role in
/// it. Org owner/admin only: per the RBAC matrix (IG-1 §1.6), *who* leads a project is an
/// organisation-level decision, not something a lead can grant to someone else or to
/// themselves.
pub async fn add_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<Uuid>,
    Json(request): Json<AddTeamMemberRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;
        if !repo::team_belongs_to_org(pool, team_id, context.org_id).await? {
            return Err(AegisError::NotFound("team not found".into()));
        }
        if !matches!(request.role.as_str(), "lead" | "member") {
            return Err(AegisError::BadRequest(
                "role must be 'lead' or 'member'".into(),
            ));
        }
        // The person being added must actually belong to this organisation — otherwise
        // this would be a way to grant a stranger visibility into the project's usage.
        if repo::role_in_org(pool, context.org_id, request.user_id)
            .await?
            .is_none()
        {
            return Err(AegisError::BadRequest(
                "user_id is not a member of this organisation".into(),
            ));
        }
        repo::add_team_member(pool, team_id, request.user_id, &request.role).await?;
        audit(
            &state,
            &context,
            "team.member_added",
            "team",
            Some(team_id),
            Some(serde_json::json!({"user_id": request.user_id, "role": request.role})),
        )
        .await;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"team_id": team_id, "user_id": request.user_id, "role": request.role}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `DELETE /api/org/teams/{id}/members/{user_id}` — remove a person from a project.
/// Org owner/admin only, same reasoning as [`add_team_member`].
pub async fn remove_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((team_id, user_id)): Path<(Uuid, Uuid)>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;
        if !repo::team_belongs_to_org(pool, team_id, context.org_id).await? {
            return Err(AegisError::NotFound("team not found".into()));
        }
        let removed = repo::remove_team_member(pool, team_id, user_id).await?;
        if !removed {
            return Err(AegisError::NotFound("membership not found".into()));
        }
        audit(
            &state,
            &context,
            "team.member_removed",
            "team",
            Some(team_id),
            Some(serde_json::json!({"user_id": user_id})),
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

/// Body for [`add_team_member`].
#[derive(Debug, Deserialize)]
pub struct AddTeamMemberRequest {
    pub user_id: Uuid,
    /// `"lead"` or `"member"`.
    pub role: String,
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
    /// Cap one person's spend, summed across every key issued to them — "how much may this
    /// employee spend." Mutually exclusive with the other scopes below, same as they are
    /// with each other.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// Cap one region's share of the organisation's spend, e.g. `eu-central`.
    ///
    /// Mutually exclusive with `team_id`, `api_key_id`, and `user_id` — a budget caps one
    /// counter, and a row naming several scopes would be ambiguous about which.
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default = "default_period")]
    pub period: String,
    pub limit_mc: i64,
    /// An additional ceiling in tokens on the same scope, enforced independently of
    /// `limit_mc` — a request that is cheap in dollars can still be unbounded in tokens
    /// without one. Optional: omitting it caps money only, as every budget did before this
    /// field existed.
    #[serde(default)]
    pub limit_tokens: Option<i64>,
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
        require_plan_feature(
            &state,
            context.org_id,
            crate::billing::features::Feature::Budgets,
        )
        .await?;
        if request.limit_mc < 0 {
            return Err(AegisError::BadRequest("limit must not be negative".into()));
        }
        // One scope per budget. The database enforces this too, but a 400 naming the
        // problem is a better answer than a constraint violation surfacing as a 500.
        let scopes = [
            request.team_id.is_some(),
            request.api_key_id.is_some(),
            request.user_id.is_some(),
            request.region.is_some(),
        ]
        .iter()
        .filter(|set| **set)
        .count();
        if scopes > 1 {
            return Err(AegisError::BadRequest(
                "a budget caps one scope: set at most one of team_id, api_key_id, user_id, \
                 or region"
                    .into(),
            ));
        }
        if let Some(limit_tokens) = request.limit_tokens {
            if limit_tokens < 0 {
                return Err(AegisError::BadRequest(
                    "limit_tokens must not be negative".into(),
                ));
            }
        }
        // A budget scoped to a person must actually name someone in this organisation —
        // otherwise it silently caps nobody, which reads as "it isn't working" to whoever
        // configured it.
        if let Some(user_id) = request.user_id {
            if crate::db::repo::role_in_org(state.db()?, context.org_id, user_id)
                .await?
                .is_none()
            {
                return Err(AegisError::BadRequest(
                    "user_id is not a member of this organisation".into(),
                ));
            }
        }
        if !["monthly", "daily"].contains(&request.period.as_str()) {
            return Err(AegisError::BadRequest(
                "period must be 'monthly' or 'daily'".into(),
            ));
        }
        // Enforcement reads month-to-date counters, so a daily row would refuse traffic for
        // the rest of the month after one heavy day. Say so rather than accepting a budget
        // that would behave nothing like its name.
        if request.period == "daily" {
            return Err(AegisError::BadRequest(
                "daily budgets are not enforced yet — the spend counters they would be \
                 checked against are monthly. Use 'monthly'."
                    .into(),
            ));
        }
        let budget = repo::create_budget(
            state.db()?,
            repo::NewBudget {
                org_id: context.org_id,
                team_id: request.team_id,
                api_key_id: request.api_key_id,
                user_id: request.user_id,
                region: request.region.as_deref(),
                period: &request.period,
                limit_mc: request.limit_mc,
                limit_tokens: request.limit_tokens,
                hard_limit: request.hard_limit,
            },
        )
        .await?;
        // A budget nobody can see take effect is worse than no budget. Drop the cached
        // limit set now rather than letting an operator watch a blocked customer stay
        // blocked for up to a minute after they raised the ceiling.
        crate::middleware::budget::invalidate_limits(state.store.as_ref(), context.org_id).await;
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
        crate::middleware::budget::invalidate_limits(state.store.as_ref(), context.org_id).await;
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
    /// Narrow to one project. Only `list_requests`/`savings_report_csv` read this today;
    /// every other `RangeQuery` consumer simply ignores it, same as they already ignore
    /// `limit`/`offset` when it doesn't apply to them.
    #[serde(default)]
    pub team_id: Option<Uuid>,
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
        let summary =
            repo::usage_summary(state.analytics_db()?, context.org_id, start, end).await?;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "period": {"start": start, "end": end},
                "summary": summary,
                "derived": derived_usage_metrics(&summary),
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// The percentages and rollups every usage view derives the same way from a
/// [`repo::UsageSummary`], so the dashboard's org, project, and "my usage" views never
/// silently disagree about how a rate is computed.
fn derived_usage_metrics(summary: &repo::UsageSummary) -> serde_json::Value {
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
    serde_json::json!({
        "savings_percent": savings_percent,
        "cache_hit_rate": cache_hit_rate,
        "customer_net_mc": summary.gross_savings_mc - summary.aegis_fee_mc,
        "savings_breakdown_mc": {
            "routing": summary.routing_savings_mc,
            "compression": summary.compression_savings_mc,
            "cache": summary.cache_savings_mc,
        },
    })
}

/// `GET /api/projects/{id}/usage` — a project (team) lead's or org admin's view of one
/// project's spend. See [`assert_project_access`] for who may reach this and why a caller
/// with no standing gets 404 rather than 403.
pub async fn project_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<Uuid>,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        assert_project_access(&state, &context, team_id).await?;
        let (start, end) = range.resolve();
        let summary = repo::usage_summary_for_team(
            state.analytics_db()?,
            context.org_id,
            team_id,
            start,
            end,
        )
        .await?;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "team_id": team_id,
                "period": {"start": start, "end": end},
                "summary": summary,
                "derived": derived_usage_metrics(&summary),
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/me/usage` — a member's own attributed spend: every request made with a key
/// issued to them, in this organisation, summed. Available to anyone authenticated —
/// a member views only ever their own figures, never anyone else's, so there is no
/// escalation to guard against.
pub async fn my_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(range): Query<RangeQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let Some(user_id) = context.user_id else {
            // A bare API key with no assignee has no "me" to report on — an org-wide
            // service key was never issued to a person.
            return Err(AegisError::BadRequest(
                "this credential is not assigned to a person; there is no per-person usage \
                 to report"
                    .into(),
            ));
        };
        let (start, end) = range.resolve();
        let summary = repo::usage_summary_for_user(
            state.analytics_db()?,
            context.org_id,
            user_id,
            start,
            end,
        )
        .await?;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({
                "user_id": user_id,
                "period": {"start": start, "end": end},
                "summary": summary,
                "derived": derived_usage_metrics(&summary),
            }),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/me/onboarding-complete` — marks the calling person's dashboard onboarding
/// tour as seen, so `GET /api/auth/me`'s `onboarding_completed_at` stops being `null` and
/// the tour does not auto-show again. Self-scoped only: any authenticated person may mark
/// their own record, and there is no path to mark anyone else's — same reasoning as
/// `my_usage` just above.
pub async fn complete_onboarding(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let Some(user_id) = context.user_id else {
            return Err(AegisError::BadRequest(
                "this credential is not assigned to a person".into(),
            ));
        };
        repo::mark_onboarding_complete(state.db()?, user_id).await?;
        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"onboarding_completed": true}),
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
            state.analytics_db()?,
            context.org_id,
            start,
            end,
            range.limit.unwrap_or(100),
            range.offset.unwrap_or(0),
            range.team_id,
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
        let rows = repo::list_requests(
            state.analytics_db()?,
            context.org_id,
            start,
            end,
            10_000,
            0,
            range.team_id,
        )
        .await?;

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

/// `GET /api/audit-log.jsonl`
///
/// A customer's own audit trail, one JSON object per line — the shape a SIEM ingests
/// directly, no client-side parsing of a wrapping array required.
///
/// This did not exist before this session. `/api/admin/audit` existed, but it is gated by
/// `is_admin` — a platform-staff flag, not an organisation role — so no customer could
/// ever reach it, and the compliance whitepaper's "audit log export (JSONL, SIEM-friendly)"
/// described a capability nothing in the router actually provided. Found in the enterprise
/// readiness audit, alongside the related finding that the *admin* endpoint was also
/// scoped to the wrong organisation for its own stated purpose.
pub async fn audit_log_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuditLogQuery>,
) -> Response {
    match async {
        let context = require_reader(&state, &headers).await?;
        let entries = repo::list_audit_logs(
            state.analytics_db()?,
            context.org_id,
            query.limit.unwrap_or(1_000),
            query.offset.unwrap_or(0),
        )
        .await?;

        let mut jsonl = String::new();
        for entry in &entries {
            if let Ok(line) = serde_json::to_string(entry) {
                jsonl.push_str(&line);
                jsonl.push('\n');
            }
        }

        Ok::<_, AegisError>(
            (
                StatusCode::OK,
                [
                    (
                        axum::http::header::CONTENT_TYPE,
                        "application/x-ndjson; charset=utf-8",
                    ),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        "attachment; filename=\"aegis-audit-log.jsonl\"",
                    ),
                ],
                jsonl,
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

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
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

        // Every dashboard nav item and gated page reads this, not a hardcoded plan string
        // of its own — see `billing::features` for the mapping and the write-endpoint
        // guards (`require_plan_feature`) that enforce the same thing server-side.
        let features: std::collections::HashMap<&'static str, bool> =
            crate::billing::features::Feature::ALL
                .into_iter()
                .map(|f| {
                    (
                        f.as_str(),
                        crate::billing::features::plan_includes(&org.plan, f),
                    )
                })
                .collect();

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
                    "byok": features[crate::billing::features::Feature::Byok.as_str()],
                },
                "features": features,
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

        let pricing = state.pricing();
        let all: Vec<_> = pricing.all().collect();

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
        let pool = state.analytics_db()?;

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

        let entries =
            repo::spend_by_cost_center(state.analytics_db()?, context.org_id, start, end).await?;
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

        let entries =
            repo::spend_by_cost_center(state.analytics_db()?, context.org_id, start, end).await?;
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

/// `POST /api/billing/checkout`
#[derive(Debug, Deserialize)]
pub struct CreateCheckoutRequest {
    /// One of the self-serve plans. `enterprise` is sales-assisted, not sold here.
    pub plan: String,
}

/// The two plans a checkout session can actually be started for. `enterprise` is
/// deliberately excluded — see `Config::stripe_price_pro`/`stripe_price_team`'s own doc
/// comment for why.
fn stripe_price_for_plan<'a>(config: &'a crate::config::Config, plan: &str) -> Option<&'a str> {
    match plan {
        "pro" => config.stripe_price_pro.as_deref(),
        "team" => config.stripe_price_team.as_deref(),
        _ => None,
    }
}

/// `POST /api/billing/checkout` — start a Stripe Checkout session and hand back its hosted
/// URL for the dashboard to redirect the browser to.
///
/// The organisation's Stripe customer is created here, exactly once: the first checkout for
/// an org that has never had one mints a customer and stores its id, and every later
/// checkout (a plan change, a lapsed subscription resumed) reuses it, so an organisation
/// never accumulates more than one Stripe customer no matter how many times it checks out.
pub async fn create_checkout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateCheckoutRequest>,
) -> Response {
    match async {
        let context = require_writer(&state, &headers).await?;
        let pool = state.db()?;

        let Some(secret_key) = state.config.stripe_secret_key.as_deref() else {
            return Err(AegisError::ServiceUnavailable(
                "billing is not configured on this deployment yet (STRIPE_SECRET_KEY unset)".into(),
            ));
        };
        let Some(price_id) = stripe_price_for_plan(&state.config, &request.plan) else {
            return Err(AegisError::ServiceUnavailable(format!(
                "no Stripe price is configured for the \"{}\" plan yet",
                request.plan
            )));
        };

        let org = repo::find_org(pool, context.org_id)
            .await?
            .ok_or_else(|| AegisError::NotFound("organisation not found".into()))?;

        // Reuse the existing Stripe customer if this organisation already checked out once;
        // otherwise mint one and remember it, so it is never created twice.
        let customer_id = match &org.stripe_customer_id {
            Some(id) => id.clone(),
            None => {
                let created = crate::billing::stripe::create_customer(
                    &state.http,
                    secret_key,
                    org.id,
                    &org.name,
                    org.billing_email.as_deref(),
                )
                .await?;
                repo::set_stripe_customer_id(pool, org.id, &created).await?;
                created
            }
        };

        let success_url = format!("{}/billing?checkout=success", state.config.app_url);
        let cancel_url = format!("{}/billing?checkout=cancelled", state.config.app_url);
        let mut body = crate::billing::stripe::checkout_session_body(
            org.id,
            &request.plan,
            price_id,
            &success_url,
            &cancel_url,
        );
        body.push(("customer".to_string(), customer_id));

        let checkout_url =
            crate::billing::stripe::create_checkout_session(&state.http, secret_key, body).await?;

        audit(
            &state,
            &context,
            "billing.checkout_started",
            "organization",
            Some(org.id),
            Some(serde_json::json!({"plan": request.plan})),
        )
        .await;

        Ok::<_, AegisError>(respond(
            StatusCode::OK,
            serde_json::json!({"checkout_url": checkout_url}),
        ))
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/billing/webhook` — receives events Stripe sends, never a call the dashboard
/// or an SDK makes. Authenticated by `Stripe-Signature` alone, never a session or API key:
/// Stripe cannot present either, and the signature is exactly as strong an authenticator as
/// this deliberately public endpoint needs. See `billing::stripe`'s own module doc for why
/// verification runs against the **raw** body — `axum::body::Bytes` here, never `Json<T>`,
/// which would parse (and silently reformat) the body before the signature could be checked
/// against the exact bytes Stripe signed.
pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match async {
        let Some(webhook_secret) = state.config.stripe_webhook_secret.as_deref() else {
            // Fails closed, same as `verify_signature` itself: an unconfigured secret must
            // never be read as "accept everything Stripe-shaped".
            return Err(AegisError::ServiceUnavailable(
                "Stripe webhooks are not configured on this deployment (STRIPE_WEBHOOK_SECRET unset)"
                    .into(),
            ));
        };
        let signature = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AegisError::Unauthorized("missing Stripe-Signature header".into()))?;

        let event = crate::billing::stripe::parse_webhook(
            &body,
            signature,
            Some(webhook_secret),
            Utc::now().timestamp(),
        )?;

        if !event.is_handled() {
            // A real, verified event we simply don't act on (Stripe sends dozens of event
            // types). 200 tells Stripe not to retry; there is nothing to retry into.
            return Ok::<_, AegisError>(respond(
                StatusCode::OK,
                serde_json::json!({"handled": false}),
            ));
        }

        let pool = state.db()?;

        // `org_reference()` is set at checkout and propagates onto the subscription, so it
        // covers every handled event type; the customer-id lookup is a fallback for the
        // rarer shape that carries one but not the other, so a webhook is never silently
        // dropped just because one of the two paths back to an organisation was absent.
        let org = match event.org_reference() {
            Some(org_id) => repo::find_org(pool, org_id).await?,
            None => match event.customer_id() {
                Some(customer_id) => {
                    repo::find_org_by_stripe_customer_id(pool, &customer_id).await?
                }
                None => None,
            },
        };
        let Some(org) = org else {
            // A verified event this deployment has no organisation for — most likely a
            // customer created directly in the Stripe dashboard rather than through
            // checkout. Acknowledge it so Stripe stops retrying; there is nothing to apply
            // it to.
            return Ok(respond(
                StatusCode::OK,
                serde_json::json!({"handled": false, "reason": "no matching organisation"}),
            ));
        };

        let change = crate::billing::stripe::plan_change_for(&event);
        match &change {
            crate::billing::stripe::PlanChange::Upgrade { plan } => {
                repo::update_org_plan(pool, org.id, plan, savings_share_basis_points(plan) as i32)
                    .await?;
            }
            crate::billing::stripe::PlanChange::Downgrade => {
                repo::update_org_plan(
                    pool,
                    org.id,
                    "free",
                    savings_share_basis_points("free") as i32,
                )
                .await?;
            }
            // Stripe retries a failed payment for days; cutting the organisation off on the
            // first failure (often just an expired card) is worse than carrying it through
            // the retry window. Recorded in the audit log below so it is visible, not acted
            // on automatically.
            crate::billing::stripe::PlanChange::PaymentFailed
            | crate::billing::stripe::PlanChange::None => {}
        }

        repo::write_audit_log(
            pool,
            org.id,
            None,
            "billing.webhook_processed",
            "organization",
            Some(org.id),
            Some(serde_json::json!({
                "event_type": event.event_type,
                "event_id": event.id,
                "change": format!("{change:?}"),
            })),
        )
        .await
        .ok();

        Ok(respond(
            StatusCode::OK,
            serde_json::json!({"handled": true, "event_type": event.event_type}),
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
    fn routing_mode_validation_accepts_the_five_ladder_values_and_nothing_else() {
        for mode in ["passthrough", "quality", "balanced", "economy", "auto"] {
            assert_eq!(validate_routing_mode(mode).unwrap(), mode);
        }
        // `cheap` was the legacy header spelling of `economy`, but a stored default is a
        // deliberate admin choice, not a customer's request header — the forgiving
        // synonym does not extend here, unlike case, which is normalised below.
        for bogus in ["cheap", "", "not-a-mode"] {
            assert!(
                validate_routing_mode(bogus).is_err(),
                "{bogus:?} should be rejected, not silently coerced"
            );
        }
    }

    #[test]
    fn routing_mode_validation_normalises_case_and_whitespace_rather_than_rejecting() {
        // The database CHECK constraint only admits exact lowercase strings, but
        // `RoutingHint::parse` reading this value back at request time is case-insensitive
        // — an admin typing "Balanced" must get the same forgiving treatment a customer's
        // header gets, not an opaque constraint-violation 500.
        assert_eq!(validate_routing_mode("Balanced").unwrap(), "balanced");
        assert_eq!(validate_routing_mode("  ECONOMY  ").unwrap(), "economy");
    }

    #[tokio::test]
    async fn signup_without_a_database_returns_503_not_a_bare_500() {
        // Found live: testing the actual dashboard against a gateway with no
        // DATABASE_URL configured, signup returned a generic 500 "internal_error" with no
        // actionable signal — the one gap in an otherwise-consistent 401-or-503 pattern
        // the rest of the management API already followed for a missing dependency.
        // `AppState::for_tests()` has `db: None` by construction, so this needs no real
        // database absent to reproduce — it's the default test state.
        let state = AppState::for_tests();
        let response = signup(
            State(state),
            HeaderMap::new(),
            Json(SignupRequest {
                email: "new-user@example.com".into(),
                password: "a-perfectly-fine-password".into(),
                name: None,
            }),
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a missing database must read as 'temporarily unavailable, retry', not a bare \
             500 that sends an on-call engineer looking for a code bug"
        );
    }

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
            team_id: None,
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
            team_id: None,
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

#[cfg(test)]
mod analytics_pool_tests {
    /// Handler source, with the test modules cut off.
    ///
    /// Without this, the last handler in the file "extends" into these tests and picks
    /// up every identifier they mention — which made the first version of this test fail
    /// on a handler that was perfectly correct.
    fn handlers() -> &'static str {
        let source = include_str!("management.rs");
        match source.find(
            "
#[cfg(test)]",
        ) {
            Some(offset) => &source[..offset],
            None => source,
        }
    }

    /// The body of one handler, from its signature to the start of the next.
    fn body_of(handler: &str) -> &'static str {
        let source = handlers();
        let needle = format!("pub async fn {handler}(");
        let start = source
            .find(&needle)
            .unwrap_or_else(|| panic!("handler {handler} no longer exists — update this list"));

        let end = source[start + 1..]
            .find(
                "
pub async fn ",
            )
            .map(|offset| start + 1 + offset)
            .unwrap_or(source.len());

        &source[start..end]
    }

    /// Reporting handlers must read from the analytics pool, not the primary.
    ///
    /// # Why this reads its own source
    ///
    /// The difference between `state.db()?` and `state.analytics_db()?` is invisible in
    /// every environment without a replica configured — which is all of them, until
    /// production. There is no runtime behaviour to assert against. What can be asserted
    /// is the source text, and that is exactly where the mistake gets made: someone
    /// copies an existing reporting handler, keeps `state.db()?`, and a query that scans
    /// a year of usage lands on the pool serving request authentication.
    ///
    /// The same technique guards tenant isolation in `db::repo`.
    #[test]
    fn reporting_handlers_read_from_the_analytics_pool() {
        for handler in [
            "usage_summary",
            "list_requests",
            "savings_report_csv",
            "spend_anomalies",
            "chargeback_csv",
            "chargeback_report",
        ] {
            let body = body_of(handler);

            assert!(
                !body.contains("state.db()?"),
                "{handler} is a reporting handler but reads from the primary pool via                  state.db()?. Use state.analytics_db()? so the query lands on the read                  replica when one is configured."
            );
            assert!(
                body.contains("state.analytics_db()?"),
                "{handler} should read through state.analytics_db()?"
            );
        }
    }

    /// The converse: handlers whose result feeds a write must stay on the primary.
    #[test]
    fn mutating_handlers_do_not_read_from_a_replica() {
        for handler in [
            "create_key",
            "revoke_key",
            "create_provider",
            "create_policy",
            "create_budget",
            "claim_referral",
            "invite_member",
        ] {
            assert!(
                !body_of(handler).contains("analytics_db"),
                "{handler} performs a write. Reading from a replica first can return                  pre-replication state, so a uniqueness or existence check made against                  it is not a check at all."
            );
        }
    }

    /// The scan must actually find handlers, or both tests above pass vacuously.
    #[test]
    fn the_source_scan_finds_real_handler_bodies() {
        let body = body_of("usage_summary");
        assert!(body.len() > 100, "handler body looks truncated: {body:?}");
        assert!(!handlers().contains("mod analytics_pool_tests"));
    }
}

/// `POST /api/compression/preview`
///
/// Run the compressor over a prompt and report exactly what it would save — without
/// calling a provider, spending anything, or recording usage.
///
/// This exists for demonstration and for tuning. "We compress your context" is an
/// assertion; a before/after token count with the money attached is evidence, and the
/// only honest way to show it is to run the real compressor, not a mock of it. The same
/// `compressor::compress` the live pipeline calls at stage [6c] runs here, on the caller's
/// own prompt, so the number shown is the number they would actually get.
///
/// Costs are priced at the named model's real input rate from the live pricing table, so
/// the saving is denominated in the currency the customer is billed in rather than in
/// tokens they then have to convert themselves.
pub async fn compression_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CompressionPreviewRequest>,
) -> Response {
    match async {
        require_reader(&state, &headers).await?;

        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "openai/gpt-4o".to_string());

        let mut normalized = crate::types::NormalizedRequest {
            messages: request.messages.clone(),
            ..crate::types::NormalizedRequest::simple(&model, "")
        };

        let before_text = normalized.all_text();
        // Scanned before compression, on the prompt as the caller actually sent it —
        // technique #7 (detect-only): see `crate::engine::cache_bust`.
        let cache_bust_report = crate::engine::cache_bust::scan(&normalized.system_text());
        let result = crate::engine::compressor::compress(
            &mut normalized,
            &crate::engine::compressor::CompressorConfig::default(),
        );
        let after_text = normalized.all_text();

        // Priced at the input rate: compression only ever removes prompt tokens, never
        // output ones, so charging the saving at the output rate would overstate it.
        let pricing = state.pricing();
        let (input_rate_mc, cost_saved_mc) = match pricing.get(&model) {
            Some(m) => (
                m.input_per_mtok.as_i64(),
                (m.input_per_mtok.as_i64().saturating_mul(result.tokens_saved() as i64)) / 1_000_000,
            ),
            None => (0, 0),
        };

        Ok::<Response, AegisError>(
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "model": model,
                    "tokens_before": result.tokens_before,
                    "tokens_after": result.tokens_after,
                    "tokens_saved": result.tokens_saved(),
                    "savings_percent": (result.savings_percent() * 100.0).round() / 100.0,
                    // Micro-cents, and the dollar figure alongside it so a demo does not
                    // have to do arithmetic on a projector.
                    "input_rate_per_mtok_mc": input_rate_mc,
                    "cost_saved_mc": cost_saved_mc,
                    "cost_saved_usd": format!("{:.6}", cost_saved_mc as f64 / 1_000_000.0),
                    "techniques": {
                        "duplicate_system_messages_removed": result.duplicate_system_messages_removed,
                        "json_blocks_minified": result.json_blocks_minified,
                        "duplicate_blocks_referenced": result.duplicate_blocks_referenced,
                        "stale_tool_results_trimmed": result.stale_tool_results_trimmed,
                        "whitespace_chars_removed": result.whitespace_chars_removed,
                        "messages_truncated": result.messages_truncated,
                    },
                    // The prompt as the provider would have received it, before and after.
                    // Returned so a demo can show the actual diff rather than asking the
                    // audience to trust a number.
                    "prompt_before": before_text,
                    "prompt_after": after_text,
                    "messages_after": normalized.messages,
                    // Detect-only (technique #7): does this system prompt contain a
                    // timestamp, UUID, request id, or nonce that would bust the provider's
                    // own prefix cache on every single turn? See `engine::cache_bust`.
                    "cache_bust": {
                        "detected": cache_bust_report.detected(),
                        "occurrences": cache_bust_report.count(),
                        "kinds": cache_bust_report.hits.iter().map(|h| h.kind).collect::<Vec<_>>(),
                    },
                })),
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

/// Body for [`compression_preview`].
#[derive(Debug, Deserialize)]
pub struct CompressionPreviewRequest {
    pub messages: Vec<crate::types::Message>,
    /// Model whose input rate prices the saving. Defaults to `openai/gpt-4o`.
    #[serde(default)]
    pub model: Option<String>,
}

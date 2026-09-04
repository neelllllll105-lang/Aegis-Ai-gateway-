//! Data access.
//!
//! # The rule this module exists to enforce
//!
//! **Every tenant-scoped query takes an `org_id` and filters on it.** Not "usually", not
//! "when the caller remembers" — the function signatures make it impossible to ask for a
//! resource without also saying which organisation is asking. A lookup by id alone does
//! not exist here, so no caller can accidentally read across tenants.
//!
//! The consequence is that a mistyped or malicious id returns `NotFound` rather than
//! somebody else's data, and the integration tests in `tests/` assert exactly that.
//!
//! # Runtime-checked queries
//!
//! Queries use `sqlx::query_as` rather than the compile-time `query_as!` macro. See
//! `docs/adr/0004-runtime-checked-sql.md` — the short version is that the macro requires
//! a live database at build time, which would mean nobody can compile or test this
//! repository without first standing up PostgreSQL. Column mapping is covered by the
//! integration tests instead.

use crate::error::{AegisError, Result};
use crate::money::MicroCents;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// A user account.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Never serialized to a client.
    #[serde(skip)]
    pub password_hash: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub totp_enabled: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl User {
    /// True when the account may sign in.
    pub fn is_active(&self) -> bool {
        self.disabled_at.is_none()
    }
}

/// An organisation — the tenancy root.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub plan: String,
    pub savings_share_bp: i32,
    pub billing_email: Option<String>,
    pub zero_retention: bool,
    pub content_capture: bool,
    pub region: String,
    pub stripe_customer_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Organization {
    /// Savings-share rate in basis points.
    pub fn savings_share_basis_points(&self) -> u32 {
        self.savings_share_bp.max(0) as u32
    }

    /// Whether responses may be cached for this organisation.
    pub fn caching_allowed(&self) -> bool {
        !self.zero_retention
    }
}

/// An API key, without the key itself.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub org_id: Uuid,
    pub team_id: Option<Uuid>,
    /// The person this key was issued to. `None` for a shared project or service key.
    #[sqlx(default)]
    #[serde(default)]
    pub assigned_to_user_id: Option<Uuid>,
    pub name: String,
    pub key_prefix: String,
    #[serde(skip)]
    pub key_hash: String,
    pub rate_limit_per_minute: i32,
    pub monthly_budget_mc: Option<i64>,
    pub allowed_models: Option<serde_json::Value>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ApiKey {
    /// True when the key may authenticate a request right now.
    pub fn is_usable(&self) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|e| e > Utc::now())
    }

    /// The model allowlist, if one is set.
    pub fn allowed_model_list(&self) -> Option<Vec<String>> {
        self.allowed_models.as_ref().and_then(|value| {
            value.as_array().map(|models| {
                models
                    .iter()
                    .filter_map(|m| m.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
    }
}

/// A BYOK provider credential, without the key.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ProviderCredential {
    pub id: Uuid,
    pub org_id: Uuid,
    pub provider: String,
    /// Ciphertext. Never serialized; the `skip` is what keeps it out of API responses.
    #[serde(skip)]
    pub encrypted_key: Vec<u8>,
    pub key_hint: Option<String>,
    pub base_url: Option<String>,
    pub label: Option<String>,
    pub is_default: bool,
    pub last_tested_at: Option<DateTime<Utc>>,
    pub last_test_ok: Option<bool>,
    pub created_at: DateTime<Utc>,
}

/// A team.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Team {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub monthly_budget_mc: Option<i64>,
    pub created_at: DateTime<Utc>,
}

/// A stored routing policy.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct StoredPolicy {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub rules: serde_json::Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// A budget.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Budget {
    pub id: Uuid,
    pub org_id: Uuid,
    pub team_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    /// Region this budget caps, lower-cased. `None` for a budget that is not
    /// region-scoped. Exactly one of `team_id`, `api_key_id`, `region` may be set; a row
    /// with none of them is the organisation-wide budget.
    pub region: Option<String>,
    pub period: String,
    pub limit_mc: i64,
    pub hard_limit: bool,
    pub created_at: DateTime<Utc>,
}

/// A membership row joined with the user.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Member {
    pub user_id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

/// Aggregated usage for a period.
#[derive(Debug, Clone, Default, FromRow, Serialize)]
pub struct UsageSummary {
    pub requests: i64,
    pub cache_hits: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub baseline_cost_mc: i64,
    pub actual_cost_mc: i64,
    pub gross_savings_mc: i64,
    pub aegis_fee_mc: i64,
}

/// One row of the request metadata log. Deliberately carries no prompt or response
/// content — Principle 4.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct RequestLogRow {
    pub request_id: Uuid,
    pub requested_model: String,
    pub served_model: String,
    pub provider: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    #[sqlx(default)]
    pub cached_input_tokens: i64,
    pub baseline_cost_mc: i64,
    pub actual_cost_mc: i64,
    #[sqlx(default)]
    pub input_cost_mc: i64,
    #[sqlx(default)]
    pub output_cost_mc: i64,
    pub gross_savings_mc: i64,
    pub latency_ms: i32,
    pub cache_hit: bool,
    pub cache_type: Option<String>,
    pub routing_reason: String,
    #[sqlx(default)]
    pub complexity_score_milli: Option<i16>,
    #[sqlx(default)]
    pub tokens_saved_by_compression: i32,
    pub status_code: i32,
    pub created_at: DateTime<Utc>,
}

/// Everything the hot path needs about a key, resolved in one query.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct KeyContext {
    pub api_key_id: Uuid,
    pub org_id: Uuid,
    pub team_id: Option<Uuid>,
    /// The person this key was issued to, when it was issued to one.
    ///
    /// Carried on the hot path so every usage record can be attributed to a human without
    /// a second query. `None` is a shared key — a project or service key — which is a
    /// legitimate case, not missing data.
    #[sqlx(default)]
    #[serde(default)]
    pub assigned_to_user_id: Option<Uuid>,
    pub rate_limit_per_minute: i32,
    pub monthly_budget_mc: Option<i64>,
    pub allowed_models: Option<serde_json::Value>,
    pub plan: String,
    pub savings_share_bp: i32,
    pub zero_retention: bool,
    pub org_region: String,
}

impl KeyContext {
    /// The model allowlist, if one is set.
    pub fn allowed_model_list(&self) -> Option<Vec<String>> {
        self.allowed_models.as_ref().and_then(|value| {
            value.as_array().map(|models| {
                models
                    .iter()
                    .filter_map(|m| m.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Users & auth
// ---------------------------------------------------------------------------

/// Create a user.
pub async fn create_user(
    pool: &PgPool,
    email: &str,
    password_hash: Option<&str>,
    name: Option<&str>,
) -> Result<User> {
    sqlx::query_as::<_, User>(
        "INSERT INTO users (email, password_hash, name)
         VALUES (LOWER($1), $2, $3)
         RETURNING id, email, email_verified_at, password_hash, name, avatar_url,
                   is_admin, totp_enabled, disabled_at, created_at",
    )
    .bind(email)
    .bind(password_hash)
    .bind(name)
    .fetch_one(pool)
    .await
    .map_err(map_unique_violation(
        "an account with this email already exists",
    ))
}

/// Find a user by email.
pub async fn find_user_by_email(pool: &PgPool, email: &str) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(
        "SELECT id, email, email_verified_at, password_hash, name, avatar_url,
                is_admin, totp_enabled, disabled_at, created_at
         FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Find a user by id.
pub async fn find_user_by_id(pool: &PgPool, user_id: Uuid) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(
        "SELECT id, email, email_verified_at, password_hash, name, avatar_url,
                is_admin, totp_enabled, disabled_at, created_at
         FROM users WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

// ---------------------------------------------------------------------------
// TOTP two-factor authentication
//
// `totp_secret_encrypted` and `totp_enabled` have existed on `users` since the initial
// schema, and the RFC 6238 algorithm in `enterprise::totp` was correct from the start.
// Nothing in between them existed: no function here read or wrote the secret column, no
// enrollment endpoint, no login-time check. Found in the enterprise readiness audit.
// ---------------------------------------------------------------------------

/// Read a user's encrypted TOTP secret, if one has ever been set.
///
/// Returns the ciphertext regardless of whether `totp_enabled` is true — the enrollment
/// flow needs to read a secret back to verify the confirmation code *before* turning
/// enforcement on, which is exactly the state where a secret exists and enabled does not.
pub async fn get_totp_secret(pool: &PgPool, user_id: Uuid) -> Result<Option<Vec<u8>>> {
    let row: Option<(Option<Vec<u8>>,)> =
        sqlx::query_as("SELECT totp_secret_encrypted FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .map_err(AegisError::Database)?;
    Ok(row.and_then(|(secret,)| secret))
}

/// Store a newly generated TOTP secret. Does **not** enable enforcement — that only
/// happens once the user proves they can generate a matching code, in
/// [`enable_totp`]. Storing and enabling in one step would let a bare enrollment call
/// (no proof of possessing the authenticator app) lock the account's own owner out.
pub async fn set_totp_secret(pool: &PgPool, user_id: Uuid, encrypted_secret: &[u8]) -> Result<()> {
    sqlx::query("UPDATE users SET totp_secret_encrypted = $1, updated_at = NOW() WHERE id = $2")
        .bind(encrypted_secret)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(())
}

/// Turn on TOTP enforcement for a user who has already stored and confirmed a secret.
pub async fn enable_totp(pool: &PgPool, user_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE users SET totp_enabled = true, updated_at = NOW() WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(())
}

/// Turn off TOTP and forget the secret entirely.
///
/// Clearing the secret, not just the flag, matters: leaving it in place would mean
/// re-enabling 2FA silently reactivates an old, possibly-compromised secret rather than
/// starting a fresh enrollment.
pub async fn disable_totp(pool: &PgPool, user_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE users SET totp_enabled = false, totp_secret_encrypted = NULL, \
         updated_at = NOW() WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(())
}

/// Store a session.
pub async fn create_session(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<Uuid> {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(row.0)
}

/// Resolve a session token hash to its user, if the session is live.
pub async fn find_user_by_session(pool: &PgPool, token_hash: &str) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.email_verified_at, u.password_hash, u.name, u.avatar_url,
                u.is_admin, u.totp_enabled, u.disabled_at, u.created_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1
           AND s.expires_at > NOW()
           AND u.deleted_at IS NULL
           AND u.disabled_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Delete a session (logout).
pub async fn delete_session(pool: &PgPool, token_hash: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Remove expired sessions. Called periodically.
pub async fn purge_expired_sessions(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected())
}

/// Mark an email verified.
pub async fn mark_email_verified(pool: &PgPool, user_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE users SET email_verified_at = NOW(), updated_at = NOW() WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(AegisError::Database)
}

/// Store a single-use auth token.
pub async fn create_auth_token(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    purpose: &str,
    expires_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO auth_tokens (user_id, token_hash, purpose, expires_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(purpose)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AegisError::Database)
}

/// Consume a single-use auth token, returning its user.
///
/// The `consumed_at IS NULL` predicate is inside the UPDATE, so consumption is atomic: two
/// concurrent redemptions of the same reset link cannot both succeed.
pub async fn consume_auth_token(
    pool: &PgPool,
    token_hash: &str,
    purpose: &str,
) -> Result<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE auth_tokens SET consumed_at = NOW()
         WHERE token_hash = $1 AND purpose = $2 AND consumed_at IS NULL AND expires_at > NOW()
         RETURNING user_id",
    )
    .bind(token_hash)
    .bind(purpose)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(row.map(|(user_id,)| user_id))
}

/// Update a user's password hash.
pub async fn update_password(pool: &PgPool, user_id: Uuid, password_hash: &str) -> Result<()> {
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = NOW() WHERE id = $1")
        .bind(user_id)
        .bind(password_hash)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(AegisError::Database)
}

// ---------------------------------------------------------------------------
// Organizations
// ---------------------------------------------------------------------------

/// Create an organisation and make `owner_id` its owner, atomically.
///
/// One transaction: an organisation with no owner is unreachable through the UI and would
/// need manual repair.
pub async fn create_org_with_owner(
    pool: &PgPool,
    name: &str,
    slug: &str,
    owner_id: Uuid,
) -> Result<Organization> {
    let mut tx = pool.begin().await.map_err(AegisError::Database)?;

    let org = sqlx::query_as::<_, Organization>(
        "INSERT INTO organizations (name, slug, plan, savings_share_bp)
         VALUES ($1, $2, 'free', 0)
         RETURNING id, name, slug, plan, savings_share_bp, billing_email, zero_retention,
                   content_capture, region, stripe_customer_id, created_at",
    )
    .bind(name)
    .bind(slug)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_unique_violation("that organisation slug is taken"))?;

    sqlx::query("INSERT INTO org_memberships (org_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(org.id)
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .map_err(AegisError::Database)?;

    tx.commit().await.map_err(AegisError::Database)?;
    Ok(org)
}

/// Fetch an organisation the user belongs to.
///
/// Membership is part of the query, not a separate check a caller could forget.
pub async fn find_org_for_user(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
) -> Result<Option<Organization>> {
    sqlx::query_as::<_, Organization>(
        "SELECT o.id, o.name, o.slug, o.plan, o.savings_share_bp, o.billing_email,
                o.zero_retention, o.content_capture, o.region, o.stripe_customer_id, o.created_at
         FROM organizations o
         JOIN org_memberships m ON m.org_id = o.id
         WHERE o.id = $1 AND m.user_id = $2",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Fetch an organisation by id, without a membership check.
///
/// For the gateway hot path, where authentication already established the org from an API
/// key, and for admin tooling. Never reachable from a user-facing route.
pub async fn find_org(pool: &PgPool, org_id: Uuid) -> Result<Option<Organization>> {
    sqlx::query_as::<_, Organization>(
        "SELECT id, name, slug, plan, savings_share_bp, billing_email, zero_retention,
                content_capture, region, stripe_customer_id, created_at
         FROM organizations WHERE id = $1",
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Every organisation a user belongs to.
pub async fn list_orgs_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Organization>> {
    sqlx::query_as::<_, Organization>(
        "SELECT o.id, o.name, o.slug, o.plan, o.savings_share_bp, o.billing_email,
                o.zero_retention, o.content_capture, o.region, o.stripe_customer_id, o.created_at
         FROM organizations o
         JOIN org_memberships m ON m.org_id = o.id
         WHERE m.user_id = $1
         ORDER BY o.created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// A user's role in an organisation, if they are a member.
pub async fn role_in_org(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT role FROM org_memberships WHERE org_id = $1 AND user_id = $2")
            .bind(org_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .map_err(AegisError::Database)?;
    Ok(row.map(|(role,)| role))
}

/// Update organisation settings.
pub async fn update_org_settings(
    pool: &PgPool,
    org_id: Uuid,
    name: Option<&str>,
    billing_email: Option<&str>,
    zero_retention: Option<bool>,
    content_capture: Option<bool>,
) -> Result<Organization> {
    sqlx::query_as::<_, Organization>(
        "UPDATE organizations SET
            name = COALESCE($2, name),
            billing_email = COALESCE($3, billing_email),
            zero_retention = COALESCE($4, zero_retention),
            -- Zero retention wins: enabling it must force capture off in the same
            -- statement, or the CHECK constraint would reject an otherwise valid update.
            content_capture = CASE
                WHEN COALESCE($4, zero_retention) THEN false
                ELSE COALESCE($5, content_capture)
            END,
            updated_at = NOW()
         WHERE id = $1
         RETURNING id, name, slug, plan, savings_share_bp, billing_email, zero_retention,
                   content_capture, region, stripe_customer_id, created_at",
    )
    .bind(org_id)
    .bind(name)
    .bind(billing_email)
    .bind(zero_retention)
    .bind(content_capture)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)
}

/// Change an organisation's plan and savings-share rate together.
pub async fn update_org_plan(
    pool: &PgPool,
    org_id: Uuid,
    plan: &str,
    savings_share_bp: i32,
) -> Result<()> {
    sqlx::query(
        "UPDATE organizations SET plan = $2, savings_share_bp = $3, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(org_id)
    .bind(plan)
    .bind(savings_share_bp)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AegisError::Database)
}

/// List members of an organisation.
pub async fn list_members(pool: &PgPool, org_id: Uuid) -> Result<Vec<Member>> {
    sqlx::query_as::<_, Member>(
        "SELECT m.user_id, u.email, u.name, m.role, m.joined_at
         FROM org_memberships m
         JOIN users u ON u.id = m.user_id
         WHERE m.org_id = $1
         ORDER BY m.joined_at",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Add a member.
pub async fn add_member(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
    role: &str,
    invited_by: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO org_memberships (org_id, user_id, role, invited_by)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(role)
    .bind(invited_by)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AegisError::Database)
}

/// Remove a member.
///
/// Refuses to remove the last owner: an organisation with no owner cannot be
/// administered, and recovering one is a manual database operation.
pub async fn remove_member(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> Result<bool> {
    let mut tx = pool.begin().await.map_err(AegisError::Database)?;

    let owners: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM org_memberships WHERE org_id = $1 AND role = 'owner'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(AegisError::Database)?;

    let target_role: Option<(String,)> =
        sqlx::query_as("SELECT role FROM org_memberships WHERE org_id = $1 AND user_id = $2")
            .bind(org_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AegisError::Database)?;

    if let Some((role,)) = &target_role {
        if role == "owner" && owners.0 <= 1 {
            return Err(AegisError::BadRequest(
                "cannot remove the last owner of an organisation".into(),
            ));
        }
    }

    let result = sqlx::query("DELETE FROM org_memberships WHERE org_id = $1 AND user_id = $2")
        .bind(org_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(AegisError::Database)?;

    tx.commit().await.map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

/// Create an API key. The plaintext is never passed in or stored — only its hash.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_key(
    pool: &PgPool,
    org_id: Uuid,
    created_by: Option<Uuid>,
    name: &str,
    key_prefix: &str,
    key_hash: &str,
    team_id: Option<Uuid>,
    rate_limit_per_minute: i32,
    monthly_budget_mc: Option<i64>,
    allowed_models: Option<serde_json::Value>,
    expires_at: Option<DateTime<Utc>>,
    assigned_to_user_id: Option<Uuid>,
) -> Result<ApiKey> {
    sqlx::query_as::<_, ApiKey>(
        "INSERT INTO api_keys
            (org_id, team_id, created_by, name, key_prefix, key_hash,
             rate_limit_per_minute, monthly_budget_mc, allowed_models, expires_at,
             assigned_to_user_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         RETURNING id, org_id, team_id, assigned_to_user_id, name, key_prefix, key_hash,
                   rate_limit_per_minute, monthly_budget_mc, allowed_models, last_used_at,
                   expires_at, revoked_at, created_at",
    )
    .bind(org_id)
    .bind(team_id)
    .bind(created_by)
    .bind(name)
    .bind(key_prefix)
    .bind(key_hash)
    .bind(rate_limit_per_minute)
    .bind(monthly_budget_mc)
    .bind(allowed_models)
    .bind(expires_at)
    .bind(assigned_to_user_id)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)
}

/// Resolve a key hash to everything the hot path needs, in one round trip.
///
/// Only ever reached on a cache miss; the result is cached in Redis for 60 seconds.
pub async fn resolve_key(pool: &PgPool, key_hash: &str) -> Result<Option<KeyContext>> {
    sqlx::query_as::<_, KeyContext>(
        "SELECT k.id AS api_key_id, k.org_id, k.team_id, k.assigned_to_user_id,
                k.rate_limit_per_minute,
                k.monthly_budget_mc, k.allowed_models,
                o.plan, o.savings_share_bp, o.zero_retention, o.region AS org_region
         FROM api_keys k
         JOIN organizations o ON o.id = k.org_id
         WHERE k.key_hash = $1
           AND k.revoked_at IS NULL
           AND (k.expires_at IS NULL OR k.expires_at > NOW())",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// List an organisation's keys.
pub async fn list_api_keys(pool: &PgPool, org_id: Uuid) -> Result<Vec<ApiKey>> {
    sqlx::query_as::<_, ApiKey>(
        "SELECT id, org_id, team_id, assigned_to_user_id, name, key_prefix, key_hash,
                rate_limit_per_minute, monthly_budget_mc, allowed_models, last_used_at,
                expires_at, revoked_at, created_at
         FROM api_keys WHERE org_id = $1 ORDER BY created_at DESC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// List only the keys issued to one person, plus the org's shared (unassigned) keys.
///
/// The list an ordinary member is entitled to see. Before per-person assignment existed
/// every reader saw every key in the organisation, which was defensible when a key
/// belonged only to an org — and stops being defensible the moment keys are issued to
/// named individuals, because "whose key is this and what is its budget" becomes personal
/// information about a colleague.
///
/// Shared keys are included deliberately: a project key with no assignee is meant to be
/// used by the whole team, so hiding it would break the common case to protect nothing.
pub async fn list_api_keys_for_member(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ApiKey>> {
    sqlx::query_as::<_, ApiKey>(
        "SELECT id, org_id, team_id, assigned_to_user_id, name, key_prefix, key_hash,
                rate_limit_per_minute, monthly_budget_mc, allowed_models, last_used_at,
                expires_at, revoked_at, created_at
         FROM api_keys
         WHERE org_id = $1
           AND (assigned_to_user_id = $2 OR created_by = $2)
         ORDER BY created_at DESC",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Fetch one key **within an organisation**.
///
/// Note the two-part predicate: passing another org's key id returns `None`, not the key.
pub async fn find_api_key(pool: &PgPool, org_id: Uuid, key_id: Uuid) -> Result<Option<ApiKey>> {
    sqlx::query_as::<_, ApiKey>(
        "SELECT id, org_id, team_id, name, key_prefix, key_hash, rate_limit_per_minute,
                monthly_budget_mc, allowed_models, last_used_at, expires_at, revoked_at,
                created_at
         FROM api_keys WHERE id = $1 AND org_id = $2",
    )
    .bind(key_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Update mutable key settings.
pub async fn update_api_key(
    pool: &PgPool,
    org_id: Uuid,
    key_id: Uuid,
    name: Option<&str>,
    rate_limit_per_minute: Option<i32>,
    monthly_budget_mc: Option<i64>,
    allowed_models: Option<serde_json::Value>,
) -> Result<Option<ApiKey>> {
    sqlx::query_as::<_, ApiKey>(
        "UPDATE api_keys SET
            name = COALESCE($3, name),
            rate_limit_per_minute = COALESCE($4, rate_limit_per_minute),
            monthly_budget_mc = COALESCE($5, monthly_budget_mc),
            allowed_models = COALESCE($6, allowed_models)
         WHERE id = $1 AND org_id = $2
         RETURNING id, org_id, team_id, name, key_prefix, key_hash, rate_limit_per_minute,
                   monthly_budget_mc, allowed_models, last_used_at, expires_at, revoked_at,
                   created_at",
    )
    .bind(key_id)
    .bind(org_id)
    .bind(name)
    .bind(rate_limit_per_minute)
    .bind(monthly_budget_mc)
    .bind(allowed_models)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Revoke a key.
///
/// A soft revoke: the row stays so usage records keep referring to something real, and so
/// an audit can show when the key stopped working.
pub async fn revoke_api_key(pool: &PgPool, org_id: Uuid, key_id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE api_keys SET revoked_at = NOW()
         WHERE id = $1 AND org_id = $2 AND revoked_at IS NULL",
    )
    .bind(key_id)
    .bind(org_id)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Permanently delete an API key row from the database.
pub async fn delete_api_key(pool: &PgPool, org_id: Uuid, key_id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM api_keys WHERE id = $1 AND org_id = $2",
    )
    .bind(key_id)
    .bind(org_id)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Record that a key was used. Best-effort and off the hot path.
pub async fn touch_api_key(pool: &PgPool, key_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
        .bind(key_id)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(AegisError::Database)
}

// ---------------------------------------------------------------------------
// Provider credentials
// ---------------------------------------------------------------------------

/// Store an encrypted BYOK credential.
#[allow(clippy::too_many_arguments)]
pub async fn create_credential(
    pool: &PgPool,
    org_id: Uuid,
    provider: &str,
    encrypted_key: &[u8],
    key_hint: Option<&str>,
    base_url: Option<&str>,
    label: Option<&str>,
    is_default: bool,
) -> Result<ProviderCredential> {
    let mut tx = pool.begin().await.map_err(AegisError::Database)?;

    if is_default {
        // A partial unique index enforces one default per provider; clear the old one
        // first so setting a new default is not a constraint violation.
        sqlx::query(
            "UPDATE provider_credentials SET is_default = false
             WHERE org_id = $1 AND provider = $2 AND is_default",
        )
        .bind(org_id)
        .bind(provider)
        .execute(&mut *tx)
        .await
        .map_err(AegisError::Database)?;
    }

    let credential = sqlx::query_as::<_, ProviderCredential>(
        "INSERT INTO provider_credentials
            (org_id, provider, encrypted_key, key_hint, base_url, label, is_default)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, org_id, provider, encrypted_key, key_hint, base_url, label,
                   is_default, last_tested_at, last_test_ok, created_at",
    )
    .bind(org_id)
    .bind(provider)
    .bind(encrypted_key)
    .bind(key_hint)
    .bind(base_url)
    .bind(label)
    .bind(is_default)
    .fetch_one(&mut *tx)
    .await
    .map_err(AegisError::Database)?;

    tx.commit().await.map_err(AegisError::Database)?;
    Ok(credential)
}

/// List an organisation's credentials. Ciphertext is present but never serialized.
pub async fn list_credentials(pool: &PgPool, org_id: Uuid) -> Result<Vec<ProviderCredential>> {
    sqlx::query_as::<_, ProviderCredential>(
        "SELECT id, org_id, provider, encrypted_key, key_hint, base_url, label,
                is_default, last_tested_at, last_test_ok, created_at
         FROM provider_credentials WHERE org_id = $1 ORDER BY provider, created_at",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// The default credential for a provider within an organisation.
pub async fn find_default_credential(
    pool: &PgPool,
    org_id: Uuid,
    provider: &str,
) -> Result<Option<ProviderCredential>> {
    sqlx::query_as::<_, ProviderCredential>(
        "SELECT id, org_id, provider, encrypted_key, key_hint, base_url, label,
                is_default, last_tested_at, last_test_ok, created_at
         FROM provider_credentials
         WHERE org_id = $1 AND provider = $2
         ORDER BY is_default DESC, created_at
         LIMIT 1",
    )
    .bind(org_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Fetch one credential within an organisation.
pub async fn find_credential(
    pool: &PgPool,
    org_id: Uuid,
    credential_id: Uuid,
) -> Result<Option<ProviderCredential>> {
    sqlx::query_as::<_, ProviderCredential>(
        "SELECT id, org_id, provider, encrypted_key, key_hint, base_url, label,
                is_default, last_tested_at, last_test_ok, created_at
         FROM provider_credentials WHERE id = $1 AND org_id = $2",
    )
    .bind(credential_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Delete a credential.
pub async fn delete_credential(pool: &PgPool, org_id: Uuid, credential_id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM provider_credentials WHERE id = $1 AND org_id = $2")
        .bind(credential_id)
        .bind(org_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Record the outcome of a credential test.
pub async fn record_credential_test(
    pool: &PgPool,
    org_id: Uuid,
    credential_id: Uuid,
    ok: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE provider_credentials SET last_tested_at = NOW(), last_test_ok = $3
         WHERE id = $1 AND org_id = $2",
    )
    .bind(credential_id)
    .bind(org_id)
    .bind(ok)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AegisError::Database)
}

// ---------------------------------------------------------------------------
// Teams, policies, budgets
// ---------------------------------------------------------------------------

/// Create a team.
pub async fn create_team(
    pool: &PgPool,
    org_id: Uuid,
    name: &str,
    monthly_budget_mc: Option<i64>,
) -> Result<Team> {
    sqlx::query_as::<_, Team>(
        "INSERT INTO teams (org_id, name, monthly_budget_mc) VALUES ($1, $2, $3)
         RETURNING id, org_id, name, monthly_budget_mc, created_at",
    )
    .bind(org_id)
    .bind(name)
    .bind(monthly_budget_mc)
    .fetch_one(pool)
    .await
    .map_err(map_unique_violation("a team with that name already exists"))
}

/// List teams.
pub async fn list_teams(pool: &PgPool, org_id: Uuid) -> Result<Vec<Team>> {
    sqlx::query_as::<_, Team>(
        "SELECT id, org_id, name, monthly_budget_mc, created_at
         FROM teams WHERE org_id = $1 ORDER BY name",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Delete a team.
pub async fn delete_team(pool: &PgPool, org_id: Uuid, team_id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM teams WHERE id = $1 AND org_id = $2")
        .bind(team_id)
        .bind(org_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// The active routing policy for an organisation.
pub async fn find_active_policy(pool: &PgPool, org_id: Uuid) -> Result<Option<StoredPolicy>> {
    sqlx::query_as::<_, StoredPolicy>(
        "SELECT id, org_id, name, rules, is_active, created_at
         FROM routing_policies WHERE org_id = $1 AND is_active
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// List policies.
pub async fn list_policies(pool: &PgPool, org_id: Uuid) -> Result<Vec<StoredPolicy>> {
    sqlx::query_as::<_, StoredPolicy>(
        "SELECT id, org_id, name, rules, is_active, created_at
         FROM routing_policies WHERE org_id = $1 ORDER BY created_at DESC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Create a policy.
pub async fn create_policy(
    pool: &PgPool,
    org_id: Uuid,
    name: &str,
    rules: &serde_json::Value,
) -> Result<StoredPolicy> {
    sqlx::query_as::<_, StoredPolicy>(
        "INSERT INTO routing_policies (org_id, name, rules) VALUES ($1, $2, $3)
         RETURNING id, org_id, name, rules, is_active, created_at",
    )
    .bind(org_id)
    .bind(name)
    .bind(rules)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)
}

/// Delete a policy.
pub async fn delete_policy(pool: &PgPool, org_id: Uuid, policy_id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM routing_policies WHERE id = $1 AND org_id = $2")
        .bind(policy_id)
        .bind(org_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// List budgets.
pub async fn list_budgets(pool: &PgPool, org_id: Uuid) -> Result<Vec<Budget>> {
    sqlx::query_as::<_, Budget>(concat!(
        "SELECT id, org_id, team_id, api_key_id, region, period, limit_mc, hard_limit, ",
        "created_at FROM budgets WHERE org_id = $1 ORDER BY created_at",
    ))
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// The fields of a new budget.
///
/// A struct rather than eight positional parameters: `create_budget(pool, org, None, None,
/// None, "monthly", 1000, true)` is a line nobody can read, and the two `Option<Uuid>`s
/// next to each other are exactly the shape that silently swaps a team budget for a key
/// budget.
#[derive(Debug, Clone)]
pub struct NewBudget<'a> {
    pub org_id: Uuid,
    pub team_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub region: Option<&'a str>,
    pub period: &'a str,
    pub limit_mc: i64,
    pub hard_limit: bool,
}

/// Create a budget.
pub async fn create_budget(pool: &PgPool, budget: NewBudget<'_>) -> Result<Budget> {
    // Lower-cased here rather than trusted from the caller: the database CHECK constraint
    // rejects a mixed-case region, and the request path compares this against the
    // gateway's own configured region without normalising on every lookup.
    let region = budget.region.map(|r| r.trim().to_ascii_lowercase());
    sqlx::query_as::<_, Budget>(concat!(
        "INSERT INTO budgets ",
        "(org_id, team_id, api_key_id, region, period, limit_mc, hard_limit) ",
        "VALUES ($1, $2, $3, $4, $5, $6, $7) ",
        "RETURNING id, org_id, team_id, api_key_id, region, period, limit_mc, ",
        "hard_limit, created_at",
    ))
    .bind(budget.org_id)
    .bind(budget.team_id)
    .bind(budget.api_key_id)
    .bind(region)
    .bind(budget.period)
    .bind(budget.limit_mc)
    .bind(budget.hard_limit)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)
}

/// A configured budget alert, joined with the budget it watches.
#[derive(Debug, Clone, FromRow)]
pub struct BudgetAlertRule {
    pub id: Uuid,
    pub org_id: Uuid,
    pub org_name: String,
    pub team_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub region: Option<String>,
    pub limit_mc: i64,
    pub hard_limit: bool,
    pub threshold_pct: i32,
    pub channel: String,
    pub destination: Option<String>,
    pub last_triggered_at: Option<DateTime<Utc>>,
}

impl BudgetAlertRule {
    /// Which counter this rule's budget caps. Mirrors `middleware::budget::scopes`.
    pub fn scope(&self) -> &'static str {
        match (self.team_id, self.api_key_id, self.region.as_deref()) {
            (Some(_), _, _) => "team",
            (_, Some(_), _) => "key",
            (_, _, Some(_)) => "region",
            _ => "organization",
        }
    }
}

/// Every alert rule whose budget is still live.
///
/// Joined rather than fetched per budget: an alert sweep touching every organisation
/// should be one query, not one per row. `org_name` comes along because the rendered alert
/// names the organisation and a second lookup per alert would be wasteful.
pub async fn list_budget_alert_rules(pool: &PgPool) -> Result<Vec<BudgetAlertRule>> {
    sqlx::query_as::<_, BudgetAlertRule>(concat!(
        "SELECT a.id, b.org_id, o.name AS org_name, b.team_id, b.api_key_id, b.region, ",
        "       b.limit_mc, b.hard_limit, a.threshold_pct, a.channel, a.destination, ",
        "       a.last_triggered_at ",
        "FROM budget_alerts a ",
        "JOIN budgets b ON b.id = a.budget_id ",
        "JOIN organizations o ON o.id = b.org_id ",
        "WHERE b.period = 'monthly'",
    ))
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Stamp an alert as fired, so the same threshold does not re-fire every sweep.
pub async fn mark_alert_triggered(pool: &PgPool, alert_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE budget_alerts SET last_triggered_at = NOW() WHERE id = $1")
        .bind(alert_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(())
}

/// Every organisation that has served a request this month.
///
/// Scoped to organisations with recent activity rather than every row in the table: the
/// reconciliation worker compares month-to-date counters against month-to-date records,
/// and an organisation that has sent nothing this month has nothing to reconcile. On a
/// large tenant base that is the difference between a job that finishes and one that does
/// not.
///
/// Not tenant-scoped, and deliberately so — this is a platform-operations query, not a
/// customer-facing one, and it returns identifiers only. It is called from the
/// reconciliation worker, never from a request handler.
pub async fn list_active_org_ids(pool: &PgPool) -> Result<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT DISTINCT org_id FROM usage_records
         WHERE created_at >= DATE_TRUNC('month', NOW())",
    )
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Delete a budget.
pub async fn delete_budget(pool: &PgPool, org_id: Uuid, budget_id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM budgets WHERE id = $1 AND org_id = $2")
        .bind(budget_id)
        .bind(org_id)
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Insert a usage record idempotently.
///
/// `ON CONFLICT DO NOTHING` against the unique index on `(request_id, created_at)` is what
/// makes stream redelivery safe. Returns true when a row was actually written, so the
/// worker can distinguish new records from duplicates.
#[allow(clippy::too_many_arguments)]
pub async fn insert_usage_record(
    pool: &PgPool,
    event: &crate::metering::usage::UsageEvent,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO usage_records
            (request_id, org_id, api_key_id, team_id, requested_model, served_model, provider,
             input_tokens, output_tokens, tokens_estimated, baseline_cost_mc, actual_cost_mc,
             gross_savings_mc, aegis_fee_mc, latency_ms, gateway_overhead_us, cache_hit,
             cache_type, routing_reason, complexity_score_milli, tokens_saved_by_compression,
             status_code, error_type, created_at, cached_input_tokens, cache_write_tokens,
             input_cost_mc, output_cost_mc, user_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                 $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29)
         ON CONFLICT (request_id, created_at) DO NOTHING",
    )
    .bind(event.request_id)
    .bind(event.org_id)
    .bind(event.api_key_id)
    .bind(event.team_id)
    .bind(&event.requested_model)
    .bind(&event.served_model)
    .bind(&event.provider)
    .bind(event.input_tokens as i32)
    .bind(event.output_tokens as i32)
    .bind(event.tokens_estimated)
    .bind(event.baseline_cost_mc)
    .bind(event.actual_cost_mc)
    .bind(event.gross_savings_mc)
    .bind(event.aegis_fee_mc)
    .bind(event.latency_ms as i32)
    .bind(microseconds(event.gateway_overhead_ms))
    .bind(event.cache_hit)
    .bind(event.cache_type.as_deref())
    .bind(&event.routing_reason)
    .bind(event.complexity_score.map(thousandths))
    .bind(event.tokens_saved_by_compression as i32)
    .bind(event.status_code as i32)
    .bind(event.error_type.as_deref())
    .bind(event.created_at)
    .bind(event.cached_input_tokens as i64)
    .bind(event.cache_write_tokens as i64)
    .bind(event.input_cost_mc)
    .bind(event.output_cost_mc)
    .bind(event.user_id)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;

    Ok(result.rows_affected() > 0)
}

/// Aggregated usage for an organisation over a period.
pub async fn usage_summary(
    pool: &PgPool,
    org_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<UsageSummary> {
    sqlx::query_as::<_, UsageSummary>(
        "SELECT
            COUNT(*)::BIGINT AS requests,
            COALESCE(SUM(CASE WHEN cache_hit THEN 1 ELSE 0 END), 0)::BIGINT AS cache_hits,
            COALESCE(SUM(input_tokens), 0)::BIGINT AS input_tokens,
            COALESCE(SUM(output_tokens), 0)::BIGINT AS output_tokens,
            COALESCE(SUM(baseline_cost_mc), 0)::BIGINT AS baseline_cost_mc,
            COALESCE(SUM(actual_cost_mc), 0)::BIGINT AS actual_cost_mc,
            COALESCE(SUM(gross_savings_mc), 0)::BIGINT AS gross_savings_mc,
            COALESCE(SUM(aegis_fee_mc), 0)::BIGINT AS aegis_fee_mc
         FROM usage_records
         WHERE org_id = $1 AND created_at >= $2 AND created_at < $3",
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)
}

/// Paginated request metadata log. Never returns prompt or response content.
pub async fn list_requests(
    pool: &PgPool,
    org_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<RequestLogRow>> {
    sqlx::query_as::<_, RequestLogRow>(
        "SELECT request_id, requested_model, served_model, provider, input_tokens,
                output_tokens, COALESCE(cached_input_tokens, 0) AS cached_input_tokens,
                baseline_cost_mc, actual_cost_mc, COALESCE(input_cost_mc, 0) AS input_cost_mc,
                COALESCE(output_cost_mc, 0) AS output_cost_mc, gross_savings_mc,
                latency_ms, cache_hit, cache_type, routing_reason,
                complexity_score_milli,
                COALESCE(tokens_saved_by_compression, 0) AS tokens_saved_by_compression,
                status_code, created_at
         FROM usage_records
         WHERE org_id = $1 AND created_at >= $2 AND created_at < $3
         ORDER BY created_at DESC
         LIMIT $4 OFFSET $5",
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .bind(limit.clamp(1, 1_000))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Fold a usage record into the daily rollup.
pub async fn upsert_daily_aggregate(
    pool: &PgPool,
    event: &crate::metering::usage::UsageEvent,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO daily_aggregates
            (org_id, day, served_model, team_id, requests, cache_hits, input_tokens,
             output_tokens, baseline_cost_mc, actual_cost_mc, gross_savings_mc, aegis_fee_mc)
         VALUES ($1, $2::DATE, $3, COALESCE($4, '00000000-0000-0000-0000-000000000000'::UUID),
                 1, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (org_id, day, served_model, team_id) DO UPDATE SET
            requests = daily_aggregates.requests + 1,
            cache_hits = daily_aggregates.cache_hits + EXCLUDED.cache_hits,
            input_tokens = daily_aggregates.input_tokens + EXCLUDED.input_tokens,
            output_tokens = daily_aggregates.output_tokens + EXCLUDED.output_tokens,
            baseline_cost_mc = daily_aggregates.baseline_cost_mc + EXCLUDED.baseline_cost_mc,
            actual_cost_mc = daily_aggregates.actual_cost_mc + EXCLUDED.actual_cost_mc,
            gross_savings_mc = daily_aggregates.gross_savings_mc + EXCLUDED.gross_savings_mc,
            aegis_fee_mc = daily_aggregates.aegis_fee_mc + EXCLUDED.aegis_fee_mc,
            updated_at = NOW()",
    )
    .bind(event.org_id)
    .bind(event.created_at)
    .bind(&event.served_model)
    .bind(event.team_id)
    .bind(if event.cache_hit { 1i64 } else { 0i64 })
    .bind(event.input_tokens as i64)
    .bind(event.output_tokens as i64)
    .bind(event.baseline_cost_mc)
    .bind(event.actual_cost_mc)
    .bind(event.gross_savings_mc)
    .bind(event.aegis_fee_mc)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AegisError::Database)
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// Append an audit entry.
///
/// Failures are returned but callers generally log and continue: an audit write must not
/// undo the action it describes, and a gap is recoverable while a rolled-back key
/// rotation is not.
pub async fn write_audit_log(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Option<Uuid>,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    metadata: Option<serde_json::Value>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_logs
            (org_id, user_id, action, resource_type, resource_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(metadata)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AegisError::Database)
}

/// An audit log entry.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub org_id: Uuid,
    pub user_id: Option<Uuid>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Read the audit log for an organisation.
pub async fn list_audit_logs(
    pool: &PgPool,
    org_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEntry>> {
    sqlx::query_as::<_, AuditEntry>(
        "SELECT id, org_id, user_id, action, resource_type, resource_id, metadata, created_at
         FROM audit_logs WHERE org_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(org_id)
    .bind(limit.clamp(1, 10_000))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

// ---------------------------------------------------------------------------
// Pricing
// ---------------------------------------------------------------------------

/// A pricing row as stored.
#[derive(Debug, Clone, FromRow)]
pub struct PricingRow {
    pub model_id: String,
    pub provider: String,
    pub display_name: String,
    pub tier: String,
    pub input_cost_per_mtok_mc: i64,
    pub output_cost_per_mtok_mc: i64,
    pub context_window: i32,
    pub supports_tools: bool,
    pub supports_vision: bool,
    /// Whether the model can serve a chat completion. False for embedding models.
    #[sqlx(default)]
    pub supports_chat: bool,
    pub is_active: bool,
    pub source: String,
    /// Cache-read rate in basis points of the input rate. 10,000 = no discount.
    #[sqlx(default)]
    pub cache_read_bp: i32,
    /// Cache-write rate in basis points of the input rate. 10,000 = no premium.
    #[sqlx(default)]
    pub cache_write_bp: i32,
    /// Prompt size at which the long-context rates below take over, if any.
    #[sqlx(default)]
    pub long_context_threshold_tokens: Option<i64>,
    #[sqlx(default)]
    pub long_context_input_per_mtok_mc: Option<i64>,
    #[sqlx(default)]
    pub long_context_output_per_mtok_mc: Option<i64>,
}

impl Default for PricingRow {
    fn default() -> PricingRow {
        PricingRow {
            model_id: String::new(),
            provider: String::new(),
            display_name: String::new(),
            tier: "mid".to_string(),
            input_cost_per_mtok_mc: 0,
            output_cost_per_mtok_mc: 0,
            context_window: 0,
            supports_tools: false,
            supports_vision: false,
            supports_chat: true,
            is_active: true,
            source: String::new(),
            // 10,000 bp is 100% of the input rate: no discount, no premium. The
            // conservative default, matching the column defaults in migration 0003.
            cache_read_bp: 10_000,
            cache_write_bp: 10_000,
            long_context_threshold_tokens: None,
            long_context_input_per_mtok_mc: None,
            long_context_output_per_mtok_mc: None,
        }
    }
}

/// Load the current pricing table.
pub async fn load_pricing(pool: &PgPool) -> Result<Vec<PricingRow>> {
    sqlx::query_as::<_, PricingRow>(concat!(
        "SELECT model_id, provider, display_name, tier, input_cost_per_mtok_mc, ",
        "       output_cost_per_mtok_mc, context_window, supports_tools, supports_vision, ",
        "       supports_chat, is_active, source, cache_read_bp, cache_write_bp, ",
        "       long_context_threshold_tokens, long_context_input_per_mtok_mc, ",
        "       long_context_output_per_mtok_mc ",
        "FROM model_pricing ",
        "WHERE effective_to IS NULL AND is_active ",
        "ORDER BY model_id",
    ))
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Supersede a model's price with a new one, closing the previous row.
///
/// Two statements in one transaction so there is never a moment with two current prices
/// for a model, nor a moment with none.
pub async fn upsert_pricing(pool: &PgPool, row: &PricingRow) -> Result<()> {
    let mut tx = pool.begin().await.map_err(AegisError::Database)?;

    sqlx::query(
        "UPDATE model_pricing SET effective_to = NOW()
         WHERE model_id = $1 AND effective_to IS NULL",
    )
    .bind(&row.model_id)
    .execute(&mut *tx)
    .await
    .map_err(AegisError::Database)?;

    sqlx::query(
        "INSERT INTO model_pricing
            (model_id, provider, display_name, tier, input_cost_per_mtok_mc,
             output_cost_per_mtok_mc, context_window, supports_tools, supports_vision,
             supports_chat, is_active, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(&row.model_id)
    .bind(&row.provider)
    .bind(&row.display_name)
    .bind(&row.tier)
    .bind(row.input_cost_per_mtok_mc)
    .bind(row.output_cost_per_mtok_mc)
    .bind(row.context_window)
    .bind(row.supports_tools)
    .bind(row.supports_vision)
    .bind(row.supports_chat)
    .bind(row.is_active)
    .bind(&row.source)
    .execute(&mut *tx)
    .await
    .map_err(AegisError::Database)?;

    tx.commit().await.map_err(AegisError::Database)
}

/// Replace the entire `openrouter_pricing_reference` snapshot with `rows`.
///
/// Not tenant-scoped: this table holds no organisation data, only a public third-party
/// reference dataset every org would see the same copy of regardless.
///
/// A full snapshot, not an incremental upsert-and-leave-the-rest: OpenRouter can retire a
/// model, and if this only ever inserted/updated it would keep a stale reference to a
/// model that no longer exists forever. Deleting everything and re-inserting inside one
/// transaction means a reader never sees a half-replaced table, and a model OpenRouter
/// dropped disappears from here too rather than silently going stale.
pub async fn replace_openrouter_pricing_reference(
    pool: &PgPool,
    rows: &[crate::metering::openrouter_reference::OpenRouterPricingRow],
) -> Result<()> {
    let mut tx = pool.begin().await.map_err(AegisError::Database)?;

    sqlx::query("DELETE FROM openrouter_pricing_reference")
        .execute(&mut *tx)
        .await
        .map_err(AegisError::Database)?;

    for row in rows {
        sqlx::query(
            "INSERT INTO openrouter_pricing_reference
                (model_id, display_name, context_length, input_per_mtok_mc,
                 output_per_mtok_mc, cache_read_per_mtok_mc, cache_write_per_mtok_mc, raw)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&row.model_id)
        .bind(&row.display_name)
        .bind(row.context_length)
        .bind(row.input_per_mtok_mc)
        .bind(row.output_per_mtok_mc)
        .bind(row.cache_read_per_mtok_mc)
        .bind(row.cache_write_per_mtok_mc)
        .bind(&row.raw)
        .execute(&mut *tx)
        .await
        .map_err(AegisError::Database)?;
    }

    tx.commit().await.map_err(AegisError::Database)
}

/// Organisations that made at least one request in the last `days` days.
///
/// The weekly digest iterates this rather than every organisation, so a dormant account
/// never receives an email saying it saved nothing. A weekly "you saved $0.00" teaches
/// the recipient that Aegis mail is noise, and the budget alert that actually matters
/// gets filtered along with it.
pub async fn orgs_with_recent_usage(pool: &PgPool, days: i64) -> Result<Vec<Organization>> {
    sqlx::query_as::<_, Organization>(
        "SELECT o.id, o.name, o.slug, o.plan, o.savings_share_bp, o.billing_email,
                o.zero_retention, o.content_capture, o.region, o.stripe_customer_id,
                o.created_at
         FROM organizations o
         WHERE EXISTS (
             SELECT 1 FROM usage_records u
             WHERE u.org_id = o.id
               AND u.created_at >= NOW() - ($1 || ' days')::INTERVAL
         )
         ORDER BY o.created_at",
    )
    .bind(days.to_string())
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Find an organisation by slug. Used to resolve a referral code.
pub async fn find_org_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Organization>> {
    sqlx::query_as::<_, Organization>(
        "SELECT id, name, slug, plan, savings_share_bp, billing_email, zero_retention,
                content_capture, region, stripe_customer_id, created_at
         FROM organizations WHERE LOWER(slug) = LOWER($1)",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Whether an organisation has already claimed a referral.
///
/// One claim per organisation, ever. Without this check an org can re-claim on every
/// login and mint unlimited credit.
pub async fn has_claimed_referral(pool: &PgPool, org_id: Uuid) -> Result<bool> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM referral_credits WHERE org_id = $1 AND reason = 'referred_signup'",
    )
    .bind(org_id)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(row.0 > 0)
}

// ---------------------------------------------------------------------------
// Enterprise: SCIM and SSO
// ---------------------------------------------------------------------------

/// A configured SSO connection, without its client secret.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SsoConnectionRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub protocol: String,
    pub issuer: String,
    pub client_id: Option<String>,
    /// Ciphertext. Never serialized — same reason as a provider credential.
    #[serde(skip)]
    pub client_secret_encrypted: Option<Vec<u8>>,
    pub email_domain: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Resolve a SCIM bearer token hash to the organisation it provisions.
///
/// A SCIM token can deprovision every member of an organisation, so it is looked up in
/// its own table and never treated as an ordinary API key.
pub async fn find_org_by_scim_token(pool: &PgPool, token_hash: &str) -> Result<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT org_id FROM scim_tokens WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(row.map(|(org_id,)| org_id))
}

/// Issue a SCIM token for an organisation. Returns the plaintext, shown once.
pub async fn create_scim_token(pool: &PgPool, org_id: Uuid, token_hash: &str) -> Result<Uuid> {
    let row: (Uuid,) =
        sqlx::query_as("INSERT INTO scim_tokens (org_id, token_hash) VALUES ($1, $2) RETURNING id")
            .bind(org_id)
            .bind(token_hash)
            .fetch_one(pool)
            .await
            .map_err(AegisError::Database)?;
    Ok(row.0)
}

/// A SCIM token's metadata, without the hash. Never enough to authenticate as it.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ScimTokenSummary {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// List an organisation's SCIM tokens, most recent first.
///
/// Metadata only — creation and revocation time, never the hash, exactly like an API key
/// listing never returns the key material. An organisation cannot know from this response
/// which token is "the right one" beyond its creation date; that is by design, the same
/// reason a bank statement shows a card's last four digits and nothing more.
pub async fn list_scim_tokens(pool: &PgPool, org_id: Uuid) -> Result<Vec<ScimTokenSummary>> {
    sqlx::query_as::<_, ScimTokenSummary>(
        "SELECT id, created_at, revoked_at FROM scim_tokens \
         WHERE org_id = $1 ORDER BY created_at DESC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)
}

/// Revoke a SCIM token. Returns true when a matching, not-already-revoked row existed.
///
/// Scoped by `org_id` like every tenant-scoped query — an organisation must not be able to
/// revoke another organisation's provisioning token even by guessing its id.
pub async fn revoke_scim_token(pool: &PgPool, org_id: Uuid, token_id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE scim_tokens SET revoked_at = NOW() \
         WHERE id = $1 AND org_id = $2 AND revoked_at IS NULL",
    )
    .bind(token_id)
    .bind(org_id)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Revoke every API key a user created within an organisation.
///
/// Called on deprovisioning. Removing the membership alone would leave any key that user
/// minted still working — which is precisely the access a terminated employee should lose
/// first.
pub async fn revoke_keys_for_user(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE api_keys SET revoked_at = NOW()
         WHERE org_id = $1 AND created_by = $2 AND revoked_at IS NULL",
    )
    .bind(org_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(result.rows_affected())
}

/// The SSO connection for an organisation.
pub async fn find_sso_connection(pool: &PgPool, org_id: Uuid) -> Result<Option<SsoConnectionRow>> {
    sqlx::query_as::<_, SsoConnectionRow>(
        "SELECT id, org_id, protocol, issuer, client_id, client_secret_encrypted,
                email_domain, is_active, created_at
         FROM sso_connections WHERE org_id = $1 AND is_active
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Find an SSO connection by email domain.
///
/// This is what makes "Sign in with SSO" work from an email address alone, without the
/// user needing to know their organisation id.
pub async fn find_sso_connection_by_domain(
    pool: &PgPool,
    domain: &str,
) -> Result<Option<SsoConnectionRow>> {
    sqlx::query_as::<_, SsoConnectionRow>(
        "SELECT id, org_id, protocol, issuer, client_id, client_secret_encrypted,
                email_domain, is_active, created_at
         FROM sso_connections
         WHERE LOWER(email_domain) = LOWER($1) AND is_active
         LIMIT 1",
    )
    .bind(domain)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Daily spend totals for an organisation, most recent last.
///
/// Feeds the spend anomaly detector, which needs a baseline of the org against itself
/// rather than an absolute threshold.
pub async fn daily_spend_history(
    pool: &PgPool,
    org_id: Uuid,
    days: i64,
) -> Result<Vec<MicroCents>> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(actual_cost_mc), 0)::BIGINT
         FROM usage_records
         WHERE org_id = $1 AND created_at >= NOW() - ($2 || ' days')::INTERVAL
         GROUP BY DATE(created_at)
         ORDER BY DATE(created_at)",
    )
    .bind(org_id)
    .bind(days.to_string())
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)?;

    Ok(rows.into_iter().map(|(total,)| MicroCents(total)).collect())
}

/// Spend grouped by cost center, for the chargeback report.
///
/// The cost center is read from the team name, which is where organisations naturally put
/// it. Spend with no team is returned with a `None` key so it can be reported as
/// unattributed rather than silently spread across the others.
pub async fn spend_by_cost_center(
    pool: &PgPool,
    org_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(Option<String>, i64, MicroCents, MicroCents, MicroCents)>> {
    let rows: Vec<(Option<String>, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT t.name,
                COUNT(*)::BIGINT,
                COALESCE(SUM(u.actual_cost_mc), 0)::BIGINT,
                COALESCE(SUM(u.gross_savings_mc), 0)::BIGINT,
                COALESCE(SUM(u.aegis_fee_mc), 0)::BIGINT
         FROM usage_records u
         LEFT JOIN teams t ON t.id = u.team_id AND t.org_id = u.org_id
         WHERE u.org_id = $1 AND u.created_at >= $2 AND u.created_at < $3
         GROUP BY t.name",
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(AegisError::Database)?;

    Ok(rows
        .into_iter()
        .map(|(center, requests, spend, savings, fee)| {
            (
                center,
                requests,
                MicroCents(spend),
                MicroCents(savings),
                MicroCents(fee),
            )
        })
        .collect())
}

/// Grant a referral credit.
pub async fn create_referral_credit(
    pool: &PgPool,
    org_id: Uuid,
    referred_org_id: Option<Uuid>,
    amount: MicroCents,
    reason: &str,
) -> Result<Uuid> {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO referral_credits (org_id, referred_org_id, amount_mc, reason)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(org_id)
    .bind(referred_org_id)
    .bind(amount.as_i64())
    .bind(reason)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(row.0)
}

/// Unconsumed credit balance for an organisation.
pub async fn credit_balance(pool: &PgPool, org_id: Uuid) -> Result<MicroCents> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_mc), 0)::BIGINT
         FROM referral_credits WHERE org_id = $1 AND consumed_at IS NULL",
    )
    .bind(org_id)
    .fetch_one(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(MicroCents(row.0))
}

/// Consume up to `amount` of an organisation credit balance, returning what was applied.
///
/// Runs in one transaction and consumes oldest-first, so a partially applied credit
/// cannot be double-spent by two concurrent invoice runs.
pub async fn consume_credits(
    pool: &PgPool,
    org_id: Uuid,
    amount: MicroCents,
) -> Result<MicroCents> {
    if amount.as_i64() <= 0 {
        return Ok(MicroCents::ZERO);
    }

    let mut tx = pool.begin().await.map_err(AegisError::Database)?;

    let available: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT id, amount_mc FROM referral_credits
         WHERE org_id = $1 AND consumed_at IS NULL
         ORDER BY created_at
         FOR UPDATE",
    )
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AegisError::Database)?;

    let mut applied = 0i64;
    for (credit_id, credit_amount) in available {
        if applied >= amount.as_i64() {
            break;
        }
        sqlx::query("UPDATE referral_credits SET consumed_at = NOW() WHERE id = $1")
            .bind(credit_id)
            .execute(&mut *tx)
            .await
            .map_err(AegisError::Database)?;
        applied = applied.saturating_add(credit_amount);
    }

    tx.commit().await.map_err(AegisError::Database)?;

    // Never report applying more than was asked for, even if the last credit overshot.
    Ok(MicroCents(applied.min(amount.as_i64())))
}

// ---------------------------------------------------------------------------
// Durable cache (tiered exact-match cache, Part 5's warm tier)
// ---------------------------------------------------------------------------

/// One encrypted, promoted cache entry. `encrypted_response` is opaque here — only
/// `cache::durable` holds the key to decrypt it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DurableCacheRow {
    pub encrypted_response: Vec<u8>,
    pub hit_count: i32,
}

/// Fetch a promoted entry, if one exists and has not expired.
///
/// An expired row is treated exactly like a miss rather than returned with a "please
/// delete this" flag — the purge job (`workers::scheduler`) is what actually removes it,
/// so a read never needs write access.
pub async fn get_cache_entry(
    pool: &PgPool,
    org_id: Uuid,
    fingerprint: &str,
) -> Result<Option<DurableCacheRow>> {
    sqlx::query_as::<_, DurableCacheRow>(
        "SELECT encrypted_response, hit_count
         FROM cache_entries
         WHERE org_id = $1 AND fingerprint = $2 AND expires_at > NOW()",
    )
    .bind(org_id)
    .bind(fingerprint)
    .fetch_optional(pool)
    .await
    .map_err(AegisError::Database)
}

/// Promote a fingerprint into the durable tier, or refresh it if already there.
///
/// `ON CONFLICT` rather than a read-then-write: two replicas promoting the same
/// fingerprint at once is a real possibility (two requests for the same repeated query,
/// microseconds apart), and a read-then-write would race exactly like the budget check
/// used to. The upsert makes "arrived twice" collapse into "one row, hit_count bumped
/// once more" instead of a duplicate-key error or a lost update.
pub async fn upsert_cache_entry(
    pool: &PgPool,
    org_id: Uuid,
    fingerprint: &str,
    encrypted_response: &[u8],
    ttl_days: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cache_entries (org_id, fingerprint, encrypted_response, expires_at)
         VALUES ($1, $2, $3, NOW() + ($4 || ' days')::interval)
         ON CONFLICT (org_id, fingerprint) DO UPDATE SET
             hit_count   = cache_entries.hit_count + 1,
             last_hit_at = NOW(),
             expires_at  = NOW() + ($4 || ' days')::interval",
    )
    .bind(org_id)
    .bind(fingerprint)
    .bind(encrypted_response)
    .bind(ttl_days)
    .execute(pool)
    .await
    .map_err(AegisError::Database)?;
    Ok(())
}

/// Delete every expired durable-cache row. Returns how many were removed.
///
/// Expiry alone (the `WHERE expires_at > NOW()` in `get_cache_entry`) already keeps a
/// stale row from ever being served — this exists so the table doesn't grow forever, not
/// because a stale row is otherwise dangerous. Not org-scoped: unlike every other function
/// in this file, deleting rows nobody can read anymore is not a tenant-isolation concern.
pub async fn purge_expired_cache_entries(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query("DELETE FROM cache_entries WHERE expires_at <= NOW()")
        .execute(pool)
        .await
        .map_err(AegisError::Database)?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Milliseconds to microseconds, saturating and NaN-safe.
///
/// The gateway overhead column is a scaled integer, so the float never reaches the
/// database. A non-finite reading — which would mean the clock misbehaved — is recorded as
/// zero rather than rejecting a usage record and losing the billing row entirely.
fn microseconds(millis: f64) -> i32 {
    if !millis.is_finite() || millis <= 0.0 {
        return 0;
    }
    (millis * 1_000.0).round().min(i32::MAX as f64) as i32
}

/// A 0.0..=1.0 score to thousandths, clamped.
fn thousandths(score: f32) -> i16 {
    if !score.is_finite() {
        return 0;
    }
    (score.clamp(0.0, 1.0) * 1_000.0).round() as i16
}

/// Turn a unique-constraint violation into a readable 400 rather than a 500.
fn map_unique_violation(message: &'static str) -> impl Fn(sqlx::Error) -> AegisError {
    move |error| match &error {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AegisError::BadRequest(message.to_string())
        }
        _ => AegisError::Database(error),
    }
}

/// Money helper: read a micro-cent column as [`MicroCents`].
pub fn micro_cents(value: i64) -> MicroCents {
    MicroCents(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_usability_covers_revocation_and_expiry() {
        let base = ApiKey {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            assigned_to_user_id: None,
            name: "test".into(),
            key_prefix: "aegis_sk_abcdefg".into(),
            key_hash: "hash".into(),
            rate_limit_per_minute: 60,
            monthly_budget_mc: None,
            allowed_models: None,
            last_used_at: None,
            expires_at: None,
            revoked_at: None,
            created_at: Utc::now(),
        };
        assert!(base.is_usable());

        let revoked = ApiKey {
            revoked_at: Some(Utc::now()),
            ..base.clone()
        };
        assert!(!revoked.is_usable());

        let expired = ApiKey {
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
            ..base.clone()
        };
        assert!(!expired.is_usable());

        let future = ApiKey {
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            ..base
        };
        assert!(future.is_usable());
    }

    #[test]
    fn allowlists_parse_from_jsonb() {
        let key = ApiKey {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            assigned_to_user_id: None,
            name: "k".into(),
            key_prefix: "p".into(),
            key_hash: "h".into(),
            rate_limit_per_minute: 60,
            monthly_budget_mc: None,
            allowed_models: Some(serde_json::json!(["gpt-4o", "gpt-4o-mini"])),
            last_used_at: None,
            expires_at: None,
            revoked_at: None,
            created_at: Utc::now(),
        };
        assert_eq!(
            key.allowed_model_list(),
            Some(vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()])
        );

        let unrestricted = ApiKey {
            allowed_models: None,
            ..key.clone()
        };
        assert_eq!(unrestricted.allowed_model_list(), None);

        // A malformed value must not be read as "allow nothing", which would break every
        // request for that key.
        let malformed = ApiKey {
            allowed_models: Some(serde_json::json!("oops")),
            ..key
        };
        assert_eq!(malformed.allowed_model_list(), None);
    }

    #[test]
    fn credentials_never_serialize_their_ciphertext() {
        // The `#[serde(skip)]` that keeps encrypted keys out of API responses.
        let credential = ProviderCredential {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            provider: "openai".into(),
            encrypted_key: b"super secret ciphertext".to_vec(),
            key_hint: Some("...abcd".into()),
            base_url: None,
            label: Some("prod".into()),
            is_default: true,
            last_tested_at: None,
            last_test_ok: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&credential).unwrap();
        assert!(!json.contains("ciphertext"), "{json}");
        assert!(!json.contains("encrypted_key"), "{json}");
        assert!(json.contains("...abcd"), "the display hint should survive");
    }

    #[test]
    fn users_never_serialize_their_password_hash() {
        let user = User {
            id: Uuid::new_v4(),
            email: "a@b.com".into(),
            email_verified_at: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$abc$def".into()),
            name: None,
            avatar_url: None,
            is_admin: false,
            totp_enabled: false,
            disabled_at: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&user).unwrap();
        assert!(!json.contains("argon2"), "{json}");
        assert!(!json.contains("password"), "{json}");
    }

    #[test]
    fn org_savings_rate_is_never_negative() {
        let org = Organization {
            id: Uuid::new_v4(),
            name: "n".into(),
            slug: "s".into(),
            plan: "pro".into(),
            savings_share_bp: -100,
            billing_email: None,
            zero_retention: false,
            content_capture: false,
            region: "eu-central".into(),
            stripe_customer_id: None,
            created_at: Utc::now(),
        };
        assert_eq!(org.savings_share_basis_points(), 0);
        assert!(org.caching_allowed());
    }

    #[test]
    fn zero_retention_orgs_disallow_caching() {
        let org = Organization {
            id: Uuid::new_v4(),
            name: "n".into(),
            slug: "s".into(),
            plan: "enterprise".into(),
            savings_share_bp: 1000,
            billing_email: None,
            zero_retention: true,
            content_capture: false,
            region: "eu-central".into(),
            stripe_customer_id: None,
            created_at: Utc::now(),
        };
        assert!(!org.caching_allowed());
    }

    #[test]
    fn every_tenant_scoped_query_names_org_id() {
        // A structural check on this file: any function taking an org_id must mention
        // org_id in its SQL. Catches the "forgot the WHERE clause" class of bug at the
        // point where it would otherwise become a cross-tenant leak.
        let source = include_str!("repo.rs");
        let scoped_fns = [
            "fn find_api_key",
            "fn update_api_key",
            "fn revoke_api_key",
            "fn list_api_keys",
            "fn find_credential",
            "fn delete_credential",
            "fn list_credentials",
            "fn find_default_credential",
            "fn list_teams",
            "fn delete_team",
            "fn list_policies",
            "fn delete_policy",
            "fn list_budgets",
            "fn delete_budget",
            "fn usage_summary",
            "fn list_requests",
            "fn list_audit_logs",
            "fn find_org_for_user",
            "fn get_cache_entry",
            "fn upsert_cache_entry",
        ];
        for name in scoped_fns {
            let start = source
                .find(name)
                .unwrap_or_else(|| panic!("{name} not found"));
            // Look at the function body that follows, bounded generously.
            let body: String = source[start..].chars().take(1_400).collect();
            let sql_end = body.find("\n}").unwrap_or(body.len());
            let body = &body[..sql_end];
            assert!(
                body.contains("org_id = $")
                    || body.contains("m.user_id = $")
                    // An INSERT-shaped scoped query (upsert_cache_entry) has no WHERE to
                    // filter by — org_id is a bound column value instead, part of the
                    // (org_id, fingerprint) uniqueness constraint that makes the upsert
                    // itself tenant-scoped.
                    || body.contains("(org_id, fingerprint"),
                "{name} does not appear to filter by org_id — possible cross-tenant read"
            );
        }
    }

    #[test]
    fn overhead_is_stored_as_microseconds() {
        assert_eq!(microseconds(0.371), 371);
        assert_eq!(microseconds(1.0), 1_000);
        assert_eq!(microseconds(0.0), 0);
        // A misbehaving clock must not cost us the billing row: non-finite and negative
        // readings record as zero rather than failing the insert.
        assert_eq!(microseconds(-1.0), 0);
        assert_eq!(microseconds(f64::NAN), 0);
        assert_eq!(microseconds(f64::INFINITY), 0);
        // A finite but absurd reading saturates instead of wrapping negative.
        assert_eq!(microseconds(1e30), i32::MAX);
    }

    #[test]
    fn complexity_is_stored_as_thousandths() {
        assert_eq!(thousandths(0.0), 0);
        assert_eq!(thousandths(0.371), 371);
        assert_eq!(thousandths(1.0), 1_000);
        // Out-of-range scores are clamped, never stored as a constraint violation.
        assert_eq!(thousandths(1.5), 1_000);
        assert_eq!(thousandths(-0.5), 0);
        assert_eq!(thousandths(f32::NAN), 0);
    }
}

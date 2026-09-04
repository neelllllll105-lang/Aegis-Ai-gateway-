//! Authentication — pipeline stage [1].
//!
//! Budget: **0.05ms**. That rules out a database round trip on the common path, so
//! lookups go through three tiers, each an order of magnitude slower than the last:
//!
//! ```text
//!   in-process LRU  (~100ns)  -> Redis  (~0.3ms)  -> PostgreSQL  (~2ms)
//! ```
//!
//! A hot key is answered from the LRU without leaving the process. A cold key costs one
//! Redis round trip. Only a genuinely unknown key reaches PostgreSQL, and the result is
//! written back into both caches.
//!
//! # The cost of caching authentication
//!
//! Caching an authorisation decision means a revoked key keeps working until the entry
//! expires. That window is bounded at [`KEY_CACHE_TTL`] (60 seconds) and, more
//! importantly, revocation actively invalidates both tiers ([`invalidate_key`]) — so the
//! stale window only applies if the invalidation itself fails. The alternative, a
//! database read per request, costs 2ms on every call to save at most a second of
//! exposure on a key the customer has already stopped using.

use crate::config::Config;
use crate::crypto;
use crate::db::repo::{self, KeyContext};
use crate::error::{AegisError, Result};
use crate::store::KvStore;
use crate::AppState;
use axum::http::HeaderMap;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a resolved key stays cached.
pub const KEY_CACHE_TTL: Duration = Duration::from_secs(60);

/// Entries held in the in-process LRU.
pub const KEY_CACHE_CAPACITY: usize = 100_000;

/// Session lifetime.
pub const SESSION_DURATION: Duration = Duration::from_secs(30 * 24 * 3_600);

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "aegis_session";

/// Who is making this request, and what they may do.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub org_id: Uuid,
    /// Present for API-key auth, absent for session auth.
    pub api_key_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    /// Present for session auth.
    pub user_id: Option<Uuid>,
    pub plan: String,
    pub savings_share_bp: u32,
    pub zero_retention: bool,
    pub rate_limit_per_minute: u32,
    pub monthly_budget_mc: Option<i64>,
    pub allowed_models: Option<Vec<String>>,
    pub region: String,
    /// Organisation role, for session auth.
    pub role: Option<String>,
    pub is_admin: bool,
}

impl AuthContext {
    /// Build from a resolved API key.
    pub fn from_key(context: KeyContext) -> AuthContext {
        AuthContext {
            org_id: context.org_id,
            api_key_id: Some(context.api_key_id),
            team_id: context.team_id,
            // The person the key was issued to, when there is one. This was hardcoded
            // `None`, which meant no gateway request — and gateway requests are all the
            // billable traffic there is — ever carried a human identity, so "what did this
            // employee spend" had no answer at any layer. A shared project key still
            // resolves to `None`, which is correct: nobody in particular sent it.
            user_id: context.assigned_to_user_id,
            plan: context.plan.clone(),
            savings_share_bp: context.savings_share_bp.max(0) as u32,
            zero_retention: context.zero_retention,
            rate_limit_per_minute: context.rate_limit_per_minute.max(1) as u32,
            monthly_budget_mc: context.monthly_budget_mc,
            allowed_models: context.allowed_model_list(),
            region: context.org_region.clone(),
            role: None,
            is_admin: false,
        }
    }

    /// True when the caller may perform a write in this organisation.
    ///
    /// API keys can write through the gateway but cannot administer the organisation:
    /// a leaked key must not be able to escalate by, say, raising its own budget.
    pub fn can_write(&self) -> bool {
        match (&self.role, self.api_key_id) {
            (Some(role), _) => matches!(role.as_str(), "owner" | "admin"),
            (None, Some(_)) => false,
            (None, None) => false,
        }
    }

    /// True when the caller may read organisation data.
    pub fn can_read(&self) -> bool {
        self.role.is_some() || self.api_key_id.is_some()
    }
}

/// An LRU entry with its own expiry.
#[derive(Clone)]
struct CachedKey {
    context: KeyContext,
    cached_at: Instant,
}

/// In-process key cache.
pub struct KeyCache {
    inner: Mutex<LruCache<String, CachedKey>>,
}

impl Default for KeyCache {
    fn default() -> Self {
        KeyCache::new(KEY_CACHE_CAPACITY)
    }
}

impl KeyCache {
    /// A cache holding at most `capacity` entries.
    pub fn new(capacity: usize) -> KeyCache {
        let capacity = NonZeroUsize::new(capacity.max(1)).unwrap_or(NonZeroUsize::MIN);
        KeyCache {
            inner: Mutex::new(LruCache::new(capacity)),
        }
    }

    /// Fetch a live entry.
    pub fn get(&self, key_hash: &str) -> Option<KeyContext> {
        let mut cache = self.inner.lock().ok()?;
        let entry = cache.get(key_hash)?;
        if entry.cached_at.elapsed() >= KEY_CACHE_TTL {
            cache.pop(key_hash);
            return None;
        }
        Some(entry.context.clone())
    }

    /// Store an entry.
    pub fn put(&self, key_hash: &str, context: KeyContext) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.put(
                key_hash.to_string(),
                CachedKey {
                    context,
                    cached_at: Instant::now(),
                },
            );
        }
    }

    /// Remove an entry.
    pub fn invalidate(&self, key_hash: &str) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.pop(key_hash);
        }
    }

    /// Entries currently held.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop every entry.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.clear();
        }
    }
}

/// Redis key for a cached API key context.
fn redis_key(key_hash: &str) -> String {
    format!("aegis:auth:key:{key_hash}")
}

/// Extract a bearer token from the `Authorization` header.
///
/// Also accepts `x-api-key`, which several client libraries send instead.
pub fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            let trimmed = text.trim();
            if let Some(token) = trimmed.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
            if let Some(token) = trimmed.strip_prefix("bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Extract the session token from the cookie header.
pub fn extract_session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == SESSION_COOKIE).then(|| value.trim().to_string())
    })
}

/// Authenticate an API key.
pub async fn authenticate_api_key(state: &AppState, token: &str) -> Result<AuthContext> {
    // A syntactic check first: malformed input never reaches the store, so a flood of
    // junk tokens cannot turn into a flood of Redis lookups.
    if !crypto::looks_like_api_key(token) {
        return Err(AegisError::Unauthorized(
            "invalid API key format. Keys look like aegis_sk_...".into(),
        ));
    }

    let key_hash = crypto::hash_token(token);

    if let Some(context) = state.key_cache.get(&key_hash) {
        return Ok(AuthContext::from_key(context));
    }

    if let Some(raw) = state.store.get(&redis_key(&key_hash)).await.ok().flatten() {
        if let Ok(context) = serde_json::from_str::<KeyContext>(&raw) {
            state.key_cache.put(&key_hash, context.clone());
            return Ok(AuthContext::from_key(context));
        }
    }

    // Running without persistence is a supported development mode, but it means no key
    // can be resolved. Say that plainly rather than surfacing a generic internal error,
    // which sends the reader looking for a bug that is not there.
    let Some(pool) = state.db.as_ref() else {
        return Err(AegisError::Unauthorized(
            concat!(
                "this gateway is running without a database, so API keys cannot be ",
                "verified. Set DATABASE_URL and restart, or see docs/HANDOFF.md."
            )
            .into(),
        ));
    };

    let Some(context) = repo::resolve_key(pool, &key_hash).await? else {
        // Deliberately identical for unknown, revoked, and expired keys: distinguishing
        // them tells an attacker which of their guesses was once real.
        return Err(AegisError::Unauthorized(
            "invalid or revoked API key".into(),
        ));
    };

    if let Ok(encoded) = serde_json::to_string(&context) {
        let _ = state
            .store
            .set_ex(&redis_key(&key_hash), &encoded, KEY_CACHE_TTL)
            .await;
    }
    state.key_cache.put(&key_hash, context.clone());

    Ok(AuthContext::from_key(context))
}

/// Drop a key from both cache tiers.
///
/// Called on revocation and on any change to a key's limits. Without this a revoked key
/// would keep working for up to [`KEY_CACHE_TTL`].
pub async fn invalidate_key(store: &dyn KvStore, cache: &KeyCache, key_hash: &str) -> Result<()> {
    cache.invalidate(key_hash);
    let _ = store.del(&redis_key(key_hash)).await;
    Ok(())
}

/// Authenticate a dashboard session.
pub async fn authenticate_session(
    state: &AppState,
    token: &str,
    org_id: Option<Uuid>,
) -> Result<AuthContext> {
    let pool = state.db()?;
    let token_hash = crypto::hash_token(token);

    let Some(user) = repo::find_user_by_session(pool, &token_hash).await? else {
        return Err(AegisError::Unauthorized(
            "session expired or invalid".into(),
        ));
    };

    // Resolve which organisation this request is acting in. An explicit org must be one
    // the user actually belongs to; otherwise fall back to their first.
    let org = match org_id {
        Some(id) => repo::find_org_for_user(pool, id, user.id)
            .await?
            .ok_or_else(|| AegisError::Forbidden("not a member of that organisation".into()))?,
        None => repo::list_orgs_for_user(pool, user.id)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| AegisError::Forbidden("no organisation".into()))?,
    };

    let role = repo::role_in_org(pool, org.id, user.id).await?;

    Ok(AuthContext {
        org_id: org.id,
        api_key_id: None,
        team_id: None,
        user_id: Some(user.id),
        plan: org.plan.clone(),
        savings_share_bp: org.savings_share_basis_points(),
        zero_retention: org.zero_retention,
        rate_limit_per_minute: 600,
        monthly_budget_mc: None,
        allowed_models: None,
        region: org.region.clone(),
        role,
        is_admin: user.is_admin,
    })
}

/// Authenticate a management request by session cookie or API key.
pub async fn authenticate_management(state: &AppState, headers: &HeaderMap) -> Result<AuthContext> {
    if let Some(session) = extract_session_cookie(headers) {
        let org_id = headers
            .get("x-aegis-org")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Uuid::parse_str(v).ok());
        return authenticate_session(state, &session, org_id).await;
    }
    if let Some(token) = extract_bearer(headers) {
        return authenticate_api_key(state, &token).await;
    }
    Err(AegisError::Unauthorized(
        "authentication required: send a session cookie or an Authorization: Bearer header".into(),
    ))
}

/// Build the `Set-Cookie` value for a session.
///
/// `HttpOnly` keeps the token away from JavaScript, `SameSite=Lax` blocks cross-site
/// submission, and `Secure` is set outside development — where it would break plain-HTTP
/// localhost.
pub fn session_cookie(token: &str, config: &Config) -> String {
    let secure = if config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax{secure}; Max-Age={}",
        SESSION_DURATION.as_secs()
    )
}

/// Build the `Set-Cookie` value that clears a session.
pub fn clear_session_cookie(config: &Config) -> String {
    let secure = if config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax{secure}; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn key_context() -> KeyContext {
        KeyContext {
            api_key_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            team_id: None,
            assigned_to_user_id: None,
            rate_limit_per_minute: 60,
            monthly_budget_mc: Some(1_000_000),
            allowed_models: None,
            plan: "pro".into(),
            savings_share_bp: 2_000,
            zero_retention: false,
            org_region: "eu-central".into(),
        }
    }

    #[test]
    fn bearer_tokens_are_extracted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer aegis_sk_test123"),
        );
        assert_eq!(
            extract_bearer(&headers).as_deref(),
            Some("aegis_sk_test123")
        );
    }

    #[test]
    fn bearer_extraction_tolerates_case_and_whitespace() {
        for value in ["bearer  token123", "Bearer token123", "  Bearer token123  "] {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                HeaderValue::from_str(value).unwrap(),
            );
            assert_eq!(
                extract_bearer(&headers).as_deref(),
                Some("token123"),
                "{value:?}"
            );
        }
    }

    #[test]
    fn x_api_key_header_is_accepted() {
        // Several client libraries send this instead of Authorization.
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("aegis_sk_abc"));
        assert_eq!(extract_bearer(&headers).as_deref(), Some("aegis_sk_abc"));
    }

    #[test]
    fn missing_or_empty_credentials_yield_nothing() {
        assert_eq!(extract_bearer(&HeaderMap::new()), None);

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static(""));
        assert_eq!(extract_bearer(&headers), None);

        let mut basic = HeaderMap::new();
        basic.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        );
        assert_eq!(extract_bearer(&basic), None);
    }

    #[test]
    fn session_cookies_are_parsed_from_a_crowded_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("theme=dark; aegis_session=tok123; other=x"),
        );
        assert_eq!(extract_session_cookie(&headers).as_deref(), Some("tok123"));
    }

    #[test]
    fn absent_session_cookie_yields_nothing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("theme=dark; other=x"),
        );
        assert_eq!(extract_session_cookie(&headers), None);
        assert_eq!(extract_session_cookie(&HeaderMap::new()), None);
    }

    #[test]
    fn cache_returns_what_was_stored() {
        let cache = KeyCache::new(10);
        let context = key_context();
        cache.put("hash1", context.clone());

        let found = cache.get("hash1").unwrap();
        assert_eq!(found.org_id, context.org_id);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("nothing").is_none());
    }

    #[test]
    fn invalidation_removes_an_entry_immediately() {
        // This is what makes revocation take effect without waiting out the TTL.
        let cache = KeyCache::new(10);
        cache.put("hash1", key_context());
        assert!(cache.get("hash1").is_some());
        cache.invalidate("hash1");
        assert!(cache.get("hash1").is_none());
    }

    #[test]
    fn cache_evicts_least_recently_used_entries() {
        let cache = KeyCache::new(2);
        cache.put("a", key_context());
        cache.put("b", key_context());
        // Touch "a" so "b" becomes the least recently used.
        let _ = cache.get("a");
        cache.put("c", key_context());

        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none(), "LRU should have evicted b");
        assert!(cache.get("c").is_some());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_is_bounded_under_pressure() {
        // A key-enumeration attack must not be able to grow the cache without limit.
        let cache = KeyCache::new(100);
        for i in 0..10_000 {
            cache.put(&format!("hash{i}"), key_context());
        }
        assert_eq!(cache.len(), 100);
    }

    #[test]
    fn clearing_empties_the_cache() {
        let cache = KeyCache::new(10);
        cache.put("a", key_context());
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn api_keys_cannot_administer_an_organisation() {
        // A leaked gateway key must not be able to raise its own budget or mint new keys.
        let context = AuthContext::from_key(key_context());
        assert!(context.can_read());
        assert!(
            !context.can_write(),
            "API keys must not have write authority"
        );
    }

    #[test]
    fn only_owners_and_admins_may_write() {
        let base = AuthContext::from_key(key_context());
        for (role, expected) in [
            ("owner", true),
            ("admin", true),
            ("member", false),
            ("viewer", false),
        ] {
            let context = AuthContext {
                role: Some(role.to_string()),
                api_key_id: None,
                user_id: Some(Uuid::new_v4()),
                ..base.clone()
            };
            assert_eq!(context.can_write(), expected, "role {role}");
            assert!(context.can_read());
        }
    }

    #[test]
    fn auth_context_carries_the_orgs_billing_and_privacy_settings() {
        let mut key = key_context();
        key.zero_retention = true;
        key.savings_share_bp = 1_500;
        key.plan = "team".into();

        let context = AuthContext::from_key(key);
        assert!(context.zero_retention);
        assert_eq!(context.savings_share_bp, 1_500);
        assert_eq!(context.plan, "team");
    }

    #[test]
    fn a_negative_rate_limit_never_becomes_zero() {
        // A corrupt row must not silently block every request for that key.
        let mut key = key_context();
        key.rate_limit_per_minute = -5;
        assert_eq!(AuthContext::from_key(key).rate_limit_per_minute, 1);
    }

    #[test]
    fn session_cookies_carry_the_right_security_attributes() {
        let mut config = Config::for_tests();
        let dev = session_cookie("tok", &config);
        assert!(dev.contains("HttpOnly"));
        assert!(dev.contains("SameSite=Lax"));
        // Secure would break plain-HTTP localhost, so it is off in development only.
        assert!(!dev.contains("Secure"));

        config.environment = crate::config::Environment::Prod;
        let prod = session_cookie("tok", &config);
        assert!(prod.contains("Secure"), "{prod}");
        assert!(prod.contains("HttpOnly"));
    }

    #[test]
    fn clearing_cookie_expires_immediately() {
        let cookie = clear_session_cookie(&Config::for_tests());
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn malformed_keys_are_rejected_before_any_lookup() {
        // Junk must never reach the store: otherwise a flood of bad tokens becomes a
        // flood of Redis traffic.
        let state = AppState::for_tests();
        for bad in ["", "not-a-key", "sk-openai-style", "aegis_sk_tooshort"] {
            let err = authenticate_api_key(&state, bad).await.unwrap_err();
            assert_eq!(err.error_type(), "unauthorized", "{bad:?}");
            assert!(format!("{err}").contains("format"), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn a_well_formed_but_unknown_key_is_rejected() {
        // No database is configured in tests, so this exercises the path where the key
        // passes validation and misses both caches.
        let state = AppState::for_tests();
        let key = crypto::generate_api_key();
        let err = authenticate_api_key(&state, &key.plaintext)
            .await
            .unwrap_err();

        // 401, not 500. Without a database the gateway genuinely cannot verify the key,
        // but that is an authentication outcome from the caller point of view, and the
        // message has to tell a developer what is actually wrong.
        assert_eq!(err.error_type(), "unauthorized");
        assert!(format!("{err}").contains("without a database"), "{err}");
    }

    #[tokio::test]
    async fn a_cached_key_authenticates_without_a_database() {
        // Demonstrates the hot path: a warm LRU answers with no Redis and no PostgreSQL.
        let state = AppState::for_tests();
        let key = crypto::generate_api_key();
        let context = key_context();
        state.key_cache.put(&key.hash, context.clone());

        let auth = authenticate_api_key(&state, &key.plaintext).await.unwrap();
        assert_eq!(auth.org_id, context.org_id);
        assert_eq!(auth.api_key_id, Some(context.api_key_id));
    }

    #[tokio::test]
    async fn invalidation_forces_the_next_lookup_to_miss() {
        let state = AppState::for_tests();
        let key = crypto::generate_api_key();
        state.key_cache.put(&key.hash, key_context());
        assert!(authenticate_api_key(&state, &key.plaintext).await.is_ok());

        invalidate_key(state.store.as_ref(), &state.key_cache, &key.hash)
            .await
            .unwrap();
        assert!(
            authenticate_api_key(&state, &key.plaintext).await.is_err(),
            "a revoked key must stop working immediately"
        );
    }

    #[tokio::test]
    async fn management_auth_requires_some_credential() {
        let state = AppState::for_tests();
        let err = authenticate_management(&state, &HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(err.error_type(), "unauthorized");
    }

    // -----------------------------------------------------------------------
    // Per-person attribution.
    //
    // `from_key` used to hardcode `user_id: None`, so no gateway request — which
    // is all the billable traffic there is — ever carried a human identity.
    // -----------------------------------------------------------------------

    #[test]
    fn a_key_issued_to_a_person_carries_that_person_into_the_auth_context() {
        let assignee = Uuid::new_v4();
        let context = KeyContext {
            assigned_to_user_id: Some(assignee),
            ..key_context()
        };

        let auth = AuthContext::from_key(context);

        assert_eq!(
            auth.user_id,
            Some(assignee),
            "the assignee must reach AuthContext, or nothing downstream can attribute spend"
        );
    }

    #[test]
    fn a_shared_key_attributes_to_nobody_rather_than_to_someone_wrong() {
        // A project or service key genuinely has no person behind it. `None` here is a
        // real answer and must not be papered over with the key's creator, who may not
        // be the one sending traffic.
        let auth = AuthContext::from_key(key_context());
        assert_eq!(auth.user_id, None);
    }

    #[test]
    fn assignment_does_not_grant_administrative_rights() {
        // Attribution is not authorisation. A key that names a person still cannot
        // administer the organisation — otherwise assigning a key would quietly be a
        // privilege escalation.
        let auth = AuthContext::from_key(KeyContext {
            assigned_to_user_id: Some(Uuid::new_v4()),
            ..key_context()
        });

        assert!(
            !auth.can_write(),
            "an assigned key must still not be a writer"
        );
        assert!(!auth.is_admin);
        assert_eq!(auth.role, None);
    }
}

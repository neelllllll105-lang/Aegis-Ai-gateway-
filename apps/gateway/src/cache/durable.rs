//! Durable cache — the warm tier between the hot Redis cache and "gone after 24 hours."
//!
//! # Why this exists
//!
//! The hot tier (`cache::exact`) is short-lived by design: a one-off prompt shouldn't
//! linger. But a genuinely popular question — the same onboarding question, the same
//! boilerplate code request — asked again after the hot tier's TTL has expired is a full
//! provider call for something Aegis has already answered. That's a real, avoidable cost.
//!
//! The fix is not "make the hot tier's TTL longer": that would keep every one-off prompt
//! around too, for no benefit and a real compliance cost (a bigger window of plaintext
//! content sitting in Redis). Instead, only fingerprints that have **already proven they
//! repeat** — a genuine hot-tier hit — get promoted here, encrypted, for longer.
//!
//! # Why this is safe to keep longer than the hot tier
//!
//! Two things the hot tier doesn't have:
//!
//! 1. **Encrypted at rest**, with the same per-tenant HKDF-derived key
//!    (`crypto::derive_tenant_key`) already used for BYOK provider credentials — no new
//!    key-management story, no new crypto surface.
//! 2. **`zero_retention` is honoured exactly as it is everywhere else** — an org that
//!    opted out of storage never has an entry promoted here, full stop, regardless of how
//!    many times the same question repeats.
//!
//! Expiry is sliding (`db::repo::upsert_cache_entry` refreshes it on every hit) and
//! purged by `workers::scheduler` on the same schedule as expired-session cleanup — see
//! `docs/adr/0008-tiered-durable-cache.md` for the full design and the promotion timeline.

use crate::cache::exact::CachedResponse;
use crate::crypto;
use crate::db::repo;
use crate::error::{AegisError, Result};
use crate::types::NormalizedResponse;
use sqlx::PgPool;
use uuid::Uuid;

/// The durable cache tier.
pub struct DurableCache<'a> {
    pool: &'a PgPool,
    master_key: &'a [u8; 32],
    ttl_days: i64,
}

impl<'a> DurableCache<'a> {
    /// Construct with a database pool, the platform master key, and the sliding TTL.
    pub fn new(pool: &'a PgPool, master_key: &'a [u8; 32], ttl_days: i64) -> DurableCache<'a> {
        DurableCache {
            pool,
            master_key,
            ttl_days,
        }
    }

    /// Look up a fingerprint that may have been promoted here.
    ///
    /// A decryption failure (the master key changed since this row was written, or the
    /// row is corrupt) is treated as a miss and logged, never a failed request — a stale
    /// cache row must cost at most one upstream call, the same guarantee the hot tier
    /// already makes for a malformed entry.
    pub async fn get(
        &self,
        fingerprint: &str,
        org_id: Uuid,
        zero_retention: bool,
    ) -> Result<Option<CachedResponse>> {
        if zero_retention {
            return Ok(None);
        }
        let Some(row) = repo::get_cache_entry(self.pool, org_id, fingerprint).await? else {
            return Ok(None);
        };

        let key = crypto::derive_tenant_key(self.master_key, &org_id.to_string());
        let plaintext = match crypto::decrypt(&key, &row.encrypted_response) {
            Ok(plaintext) => plaintext,
            Err(_) => {
                tracing::warn!(%org_id, "durable cache entry could not be decrypted, treating as a miss");
                return Ok(None);
            }
        };

        match serde_json::from_slice::<CachedResponse>(&plaintext) {
            Ok(cached) => Ok(Some(cached)),
            Err(e) => {
                tracing::warn!(error = %e, "discarding malformed durable cache payload");
                Ok(None)
            }
        }
    }

    /// Promote a fingerprint that has just proven it repeats, or refresh one already here.
    ///
    /// Silently does nothing for a zero-retention org rather than making every call site
    /// branch on it, matching every other cache tier's shape.
    pub async fn promote(
        &self,
        fingerprint: &str,
        org_id: Uuid,
        zero_retention: bool,
        response: &NormalizedResponse,
        served_model: &str,
    ) -> Result<()> {
        if zero_retention {
            return Ok(());
        }

        let entry = CachedResponse {
            response: response.clone(),
            served_model: served_model.to_string(),
            stored_at: chrono::Utc::now().timestamp(),
        };
        let plaintext = serde_json::to_vec(&entry)
            .map_err(|e| AegisError::Internal(format!("durable cache encode: {e}")))?;

        let key = crypto::derive_tenant_key(self.master_key, &org_id.to_string());
        let ciphertext = crypto::encrypt(&key, &plaintext)?;

        repo::upsert_cache_entry(self.pool, org_id, fingerprint, &ciphertext, self.ttl_days).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NormalizedResponse, TokenUsage};

    fn master_key() -> [u8; 32] {
        [9u8; 32]
    }

    fn response(content: &str) -> NormalizedResponse {
        NormalizedResponse {
            id: "r".into(),
            model: "openai/gpt-4o-mini".into(),
            content: content.into(),
            finish_reason: Some("stop".into()),
            tool_calls: None,
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 5,
                estimated: false,
                ..Default::default()
            },
            raw: None,
        }
    }

    // Everything below needs a real Postgres connection — the encryption logic itself
    // (derive → encrypt → decrypt round trip) is covered directly, with no database, so
    // the crypto correctness claim doesn't depend on infrastructure being available.

    #[test]
    fn a_promoted_entry_round_trips_through_encryption() {
        let key = crypto::derive_tenant_key(&master_key(), "11111111-1111-1111-1111-111111111111");
        let entry = CachedResponse {
            response: response("Paris"),
            served_model: "openai/gpt-4o-mini".into(),
            stored_at: 1_700_000_000,
        };
        let plaintext = serde_json::to_vec(&entry).unwrap();
        let ciphertext = crypto::encrypt(&key, &plaintext).unwrap();

        let decrypted = crypto::decrypt(&key, &ciphertext).unwrap();
        let round_tripped: CachedResponse = serde_json::from_slice(&decrypted).unwrap();
        assert_eq!(round_tripped.response.content, "Paris");
    }

    #[test]
    fn a_different_tenants_key_cannot_decrypt_it() {
        let key_a =
            crypto::derive_tenant_key(&master_key(), "11111111-1111-1111-1111-111111111111");
        let key_b =
            crypto::derive_tenant_key(&master_key(), "22222222-2222-2222-2222-222222222222");

        let entry = CachedResponse {
            response: response("confidential"),
            served_model: "m".into(),
            stored_at: 0,
        };
        let ciphertext = crypto::encrypt(&key_a, &serde_json::to_vec(&entry).unwrap()).unwrap();

        assert!(crypto::decrypt(&key_b, &ciphertext).is_err());
    }
}

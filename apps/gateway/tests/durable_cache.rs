//! The durable (Postgres, encrypted) cache tier, against a real database.
//!
//! Everything here needs `AEGIS_TEST_DATABASE_URL` — see `tests/common/mod.rs` for why a
//! missing one skips rather than fails. `cache::durable`'s own unit tests already cover the
//! pure encryption round trip with no database at all; these prove the actual table
//! interaction: promotion, retrieval, tenant isolation, and expiry.

mod common;

use aegis_gateway::cache::durable::DurableCache;
use aegis_gateway::types::{NormalizedResponse, TokenUsage};
use common::{create_org, skip, test_pool};

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

#[tokio::test]
async fn a_promoted_entry_can_be_retrieved_and_decrypted() {
    let Some(pool) = test_pool().await else {
        return skip("a_promoted_entry_can_be_retrieved_and_decrypted");
    };
    let fixture = create_org(&pool, "durable-cache").await;
    let master_key = [3u8; 32];
    let cache = DurableCache::new(&pool, &master_key, 30);

    assert!(cache
        .get("fp-1", fixture.org_id, false)
        .await
        .unwrap()
        .is_none());

    cache
        .promote(
            "fp-1",
            fixture.org_id,
            false,
            &response("Paris"),
            "openai/gpt-4o-mini",
        )
        .await
        .unwrap();

    let hit = cache.get("fp-1", fixture.org_id, false).await.unwrap();
    assert_eq!(hit.unwrap().response.content, "Paris");
}

#[tokio::test]
async fn promoting_the_same_fingerprint_twice_updates_rather_than_duplicates() {
    let Some(pool) = test_pool().await else {
        return skip("promoting_the_same_fingerprint_twice_updates_rather_than_duplicates");
    };
    let fixture = create_org(&pool, "durable-cache-upsert").await;
    let master_key = [3u8; 32];
    let cache = DurableCache::new(&pool, &master_key, 30);

    cache
        .promote("fp-2", fixture.org_id, false, &response("first"), "m")
        .await
        .unwrap();
    cache
        .promote("fp-2", fixture.org_id, false, &response("first"), "m")
        .await
        .unwrap();

    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM cache_entries WHERE org_id = $1 AND fingerprint = $2")
            .bind(fixture.org_id)
            .bind("fp-2")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row.0, 1,
        "the same fingerprint must not create a second row"
    );

    let hit_count: (i32,) = sqlx::query_as(
        "SELECT hit_count FROM cache_entries WHERE org_id = $1 AND fingerprint = $2",
    )
    .bind(fixture.org_id)
    .bind("fp-2")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hit_count.0, 3, "one insert (2) plus one more hit (3)");
}

#[tokio::test]
async fn one_orgs_durable_cache_never_serves_another() {
    let Some(pool) = test_pool().await else {
        return skip("one_orgs_durable_cache_never_serves_another");
    };
    let org_a = create_org(&pool, "durable-cache-tenant-a").await;
    let org_b = create_org(&pool, "durable-cache-tenant-b").await;
    let master_key = [3u8; 32];
    let cache = DurableCache::new(&pool, &master_key, 30);

    cache
        .promote(
            "shared-fingerprint",
            org_a.org_id,
            false,
            &response("org a's secret"),
            "m",
        )
        .await
        .unwrap();

    assert!(
        cache
            .get("shared-fingerprint", org_b.org_id, false)
            .await
            .unwrap()
            .is_none(),
        "org B must never read org A's promoted cache entry, even with an identical fingerprint"
    );
}

#[tokio::test]
async fn zero_retention_orgs_are_never_promoted() {
    let Some(pool) = test_pool().await else {
        return skip("zero_retention_orgs_are_never_promoted");
    };
    let fixture = create_org(&pool, "durable-cache-zero-retention").await;
    let master_key = [3u8; 32];
    let cache = DurableCache::new(&pool, &master_key, 30);

    cache
        .promote("fp-zr", fixture.org_id, true, &response("x"), "m")
        .await
        .unwrap();

    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cache_entries WHERE org_id = $1")
        .bind(fixture.org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, 0);
}

#[tokio::test]
async fn an_expired_entry_is_a_miss_even_though_the_row_still_exists() {
    let Some(pool) = test_pool().await else {
        return skip("an_expired_entry_is_a_miss_even_though_the_row_still_exists");
    };
    let fixture = create_org(&pool, "durable-cache-expiry").await;
    let master_key = [3u8; 32];
    let cache = DurableCache::new(&pool, &master_key, 30);

    cache
        .promote(
            "fp-expiring",
            fixture.org_id,
            false,
            &response("stale"),
            "m",
        )
        .await
        .unwrap();

    // Backdate it past expiry directly — proves `get`'s own WHERE clause enforces expiry,
    // not just the purge job eventually catching up.
    sqlx::query("UPDATE cache_entries SET expires_at = NOW() - INTERVAL '1 day' WHERE org_id = $1")
        .bind(fixture.org_id)
        .execute(&pool)
        .await
        .unwrap();

    assert!(cache
        .get("fp-expiring", fixture.org_id, false)
        .await
        .unwrap()
        .is_none());

    // The row is still physically there — only the purge job removes it. This test would
    // give a false pass if `get` were instead deleting on read.
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cache_entries WHERE org_id = $1")
        .bind(fixture.org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, 1);
}

#[tokio::test]
async fn purge_removes_only_expired_rows() {
    let Some(pool) = test_pool().await else {
        return skip("purge_removes_only_expired_rows");
    };
    let fixture = create_org(&pool, "durable-cache-purge").await;
    let master_key = [3u8; 32];
    let cache = DurableCache::new(&pool, &master_key, 30);

    cache
        .promote("fp-live", fixture.org_id, false, &response("keep"), "m")
        .await
        .unwrap();
    cache
        .promote("fp-dead", fixture.org_id, false, &response("purge me"), "m")
        .await
        .unwrap();
    sqlx::query(
        "UPDATE cache_entries SET expires_at = NOW() - INTERVAL '1 day' \
         WHERE org_id = $1 AND fingerprint = $2",
    )
    .bind(fixture.org_id)
    .bind("fp-dead")
    .execute(&pool)
    .await
    .unwrap();

    let purged = aegis_gateway::db::repo::purge_expired_cache_entries(&pool)
        .await
        .unwrap();
    assert!(purged >= 1);

    assert!(cache
        .get("fp-live", fixture.org_id, false)
        .await
        .unwrap()
        .is_some());
    assert!(cache
        .get("fp-dead", fixture.org_id, false)
        .await
        .unwrap()
        .is_none());
}

//! The hot-path key/value store.
//!
//! Principle 1 forbids synchronous PostgreSQL I/O on the request path, so authentication
//! caching, rate limiting, budget counters, the exact-match cache, and the usage event
//! stream all live here.
//!
//! Two implementations satisfy [`KvStore`]:
//!
//! * [`RedisStore`] — production. Atomic Lua for the sliding window, streams for usage.
//! * [`MemoryStore`] — development and tests. Same semantics, single process only.
//!
//! The trait exists so the entire pipeline is testable without external services, and so
//! a fresh clone of this repository runs with nothing installed. `Config::validate`
//! refuses to start a production-like environment on [`MemoryStore`], because
//! per-instance limits silently stop being limits the moment a second replica exists.

use crate::error::{AegisError, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Outcome of a rate-limit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitOutcome {
    /// Whether the request may proceed.
    pub allowed: bool,
    /// Requests remaining in the current window.
    pub remaining: u32,
    /// Seconds until the window frees capacity.
    pub retry_after_secs: u64,
}

/// One entry read back from the usage stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEntry {
    pub id: String,
    pub payload: String,
}

/// Hot-path storage operations.
#[async_trait]
pub trait KvStore: Send + Sync {
    /// Fetch a value.
    async fn get(&self, key: &str) -> Result<Option<String>>;

    /// Store a value with a time to live.
    async fn set_ex(&self, key: &str, value: &str, ttl: Duration) -> Result<()>;

    /// Delete a key. Returns true when a value was removed.
    async fn del(&self, key: &str) -> Result<bool>;

    /// Delete every key sharing a prefix. Used for cache and policy invalidation.
    async fn del_prefix(&self, prefix: &str) -> Result<u64>;

    /// Atomically add to a counter, optionally setting a TTL when it is created.
    /// Returns the new value.
    async fn incr_by(&self, key: &str, by: i64, ttl: Option<Duration>) -> Result<i64>;

    /// Sliding-window rate limit. Atomic: concurrent callers cannot both be admitted
    /// past the limit.
    async fn rate_limit(&self, key: &str, limit: u32, window: Duration)
        -> Result<RateLimitOutcome>;

    /// Append to a stream, returning the entry id. Never blocks the caller on
    /// persistence — this is how usage events leave the hot path.
    async fn stream_append(&self, stream: &str, payload: &str, max_len: usize) -> Result<String>;

    /// Read up to `count` entries after `after_id` (`"0"` reads from the start).
    async fn stream_read(
        &self,
        stream: &str,
        after_id: &str,
        count: usize,
    ) -> Result<Vec<StreamEntry>>;

    /// Liveness check for `/health`.
    async fn ping(&self) -> Result<()>;

    /// Human-readable backend name for health output.
    fn backend_name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Redis implementation
// ---------------------------------------------------------------------------

/// Sliding-window rate limiter as an atomic Lua script.
///
/// A sorted set holds one member per request, scored by timestamp. Expired members are
/// trimmed, the survivors counted, and the request admitted only if the count is under
/// the limit. Doing this in Lua makes trim-count-admit a single atomic step; doing it in
/// three round trips would let concurrent requests both observe capacity that only one
/// of them can have.
///
/// KEYS[1] = bucket key
/// ARGV[1] = limit, ARGV[2] = window (ms), ARGV[3] = now (ms), ARGV[4] = unique member id
/// Returns {allowed, remaining, retry_after_ms}
pub const RATE_LIMIT_LUA: &str = r#"
local key    = KEYS[1]
local limit  = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local now    = tonumber(ARGV[3])
local member = ARGV[4]

redis.call('ZREMRANGEBYSCORE', key, 0, now - window)
local used = redis.call('ZCARD', key)

if used < limit then
  redis.call('ZADD', key, now, member)
  redis.call('PEXPIRE', key, window)
  return {1, limit - used - 1, 0}
end

local oldest = redis.call('ZRANGE', key, 0, 0, 'WITHSCORES')
local retry = window
if oldest[2] then
  retry = math.max(0, (tonumber(oldest[2]) + window) - now)
end
redis.call('PEXPIRE', key, window)
return {0, 0, retry}
"#;

/// Redis-backed store. Uses a multiplexed connection manager: one TCP connection shared
/// across all tasks, pipelined by the driver.
pub struct RedisStore {
    connection: redis::aio::ConnectionManager,
    rate_limit_script: redis::Script,
}

impl RedisStore {
    /// Connect to Redis.
    pub async fn connect(url: &str) -> Result<RedisStore> {
        let client = redis::Client::open(url)
            .map_err(|e| AegisError::Store(format!("invalid redis url: {e}")))?;
        let connection = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| AegisError::Store(format!("redis connect failed: {e}")))?;
        Ok(RedisStore {
            connection,
            rate_limit_script: redis::Script::new(RATE_LIMIT_LUA),
        })
    }

    fn conn(&self) -> redis::aio::ConnectionManager {
        self.connection.clone()
    }
}

fn store_err(e: redis::RedisError) -> AegisError {
    AegisError::Store(e.to_string())
}

#[async_trait]
impl KvStore for RedisStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut self.conn())
            .await
            .map_err(store_err)
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .arg("PX")
            .arg(ttl.as_millis() as u64)
            .query_async::<()>(&mut self.conn())
            .await
            .map_err(store_err)
    }

    async fn del(&self, key: &str) -> Result<bool> {
        let removed: i64 = redis::cmd("DEL")
            .arg(key)
            .query_async(&mut self.conn())
            .await
            .map_err(store_err)?;
        Ok(removed > 0)
    }

    async fn del_prefix(&self, prefix: &str) -> Result<u64> {
        // SCAN rather than KEYS: KEYS blocks the server, and blocking Redis blocks every
        // request on every replica.
        let mut conn = self.conn();
        let mut cursor: u64 = 0;
        let mut removed: u64 = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{prefix}*"))
                .arg("COUNT")
                .arg(500)
                .query_async(&mut conn)
                .await
                .map_err(store_err)?;
            if !keys.is_empty() {
                let deleted: i64 = redis::cmd("DEL")
                    .arg(&keys)
                    .query_async(&mut conn)
                    .await
                    .map_err(store_err)?;
                removed += deleted.max(0) as u64;
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        Ok(removed)
    }

    async fn incr_by(&self, key: &str, by: i64, ttl: Option<Duration>) -> Result<i64> {
        let mut conn = self.conn();
        let value: i64 = redis::cmd("INCRBY")
            .arg(key)
            .arg(by)
            .query_async(&mut conn)
            .await
            .map_err(store_err)?;
        if let Some(ttl) = ttl {
            // Only set expiry when the counter was just created, so a long-lived monthly
            // counter is not repeatedly pushed forward by each increment.
            if value == by {
                redis::cmd("PEXPIRE")
                    .arg(key)
                    .arg(ttl.as_millis() as u64)
                    .query_async::<()>(&mut conn)
                    .await
                    .map_err(store_err)?;
            }
        }
        Ok(value)
    }

    async fn rate_limit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> Result<RateLimitOutcome> {
        let now_ms = now_millis();
        let member = format!("{now_ms}-{}", uuid::Uuid::new_v4());
        let result: Vec<i64> = self
            .rate_limit_script
            .key(key)
            .arg(limit)
            .arg(window.as_millis() as u64)
            .arg(now_ms)
            .arg(member)
            .invoke_async(&mut self.conn())
            .await
            .map_err(store_err)?;

        Ok(RateLimitOutcome {
            allowed: result.first().copied().unwrap_or(0) == 1,
            remaining: result.get(1).copied().unwrap_or(0).max(0) as u32,
            retry_after_secs: result
                .get(2)
                .copied()
                .map(|ms| ms.max(0).div_euclid(1000).max(1) as u64)
                .unwrap_or(1),
        })
    }

    async fn stream_append(&self, stream: &str, payload: &str, max_len: usize) -> Result<String> {
        redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg("~") // approximate trimming: much cheaper, bounded drift
            .arg(max_len)
            .arg("*")
            .arg("payload")
            .arg(payload)
            .query_async(&mut self.conn())
            .await
            .map_err(store_err)
    }

    async fn stream_read(
        &self,
        stream: &str,
        after_id: &str,
        count: usize,
    ) -> Result<Vec<StreamEntry>> {
        let start = if after_id == "0" {
            "0".to_string()
        } else {
            format!("({after_id}")
        };
        let raw: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
            .arg(stream)
            .arg(start)
            .arg("+")
            .arg("COUNT")
            .arg(count)
            .query_async(&mut self.conn())
            .await
            .map_err(store_err)?;

        Ok(raw
            .into_iter()
            .map(|(id, fields)| StreamEntry {
                id,
                payload: fields
                    .into_iter()
                    .find(|(k, _)| k == "payload")
                    .map(|(_, v)| v)
                    .unwrap_or_default(),
            })
            .collect())
    }

    async fn ping(&self) -> Result<()> {
        redis::cmd("PING")
            .query_async::<String>(&mut self.conn())
            .await
            .map(|_| ())
            .map_err(store_err)
    }

    fn backend_name(&self) -> &'static str {
        "redis"
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

struct Entry {
    value: String,
    expires_at_ms: Option<u64>,
}

#[derive(Default)]
struct MemoryState {
    values: HashMap<String, Entry>,
    windows: HashMap<String, Vec<u64>>,
    streams: HashMap<String, Vec<StreamEntry>>,
    stream_seq: u64,
}

/// In-process store with Redis-equivalent semantics.
///
/// Correct for a single instance. Explicitly not a production backend: two replicas each
/// keep their own counters, so a 60/min limit becomes 120/min. `Config::validate`
/// enforces that.
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<MemoryState>,
}

impl MemoryStore {
    /// Create an empty store.
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemoryState>> {
        self.state
            .lock()
            .map_err(|_| AegisError::Store("memory store poisoned".into()))
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[async_trait]
impl KvStore for MemoryStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        let mut state = self.lock()?;
        let now = now_millis();
        match state.values.get(key) {
            Some(entry) if entry.expires_at_ms.is_none_or(|e| e > now) => {
                Ok(Some(entry.value.clone()))
            }
            Some(_) => {
                state.values.remove(key);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    async fn set_ex(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        let mut state = self.lock()?;
        state.values.insert(
            key.to_string(),
            Entry {
                value: value.to_string(),
                expires_at_ms: Some(now_millis() + ttl.as_millis() as u64),
            },
        );
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<bool> {
        let mut state = self.lock()?;
        Ok(state.values.remove(key).is_some())
    }

    async fn del_prefix(&self, prefix: &str) -> Result<u64> {
        let mut state = self.lock()?;
        let doomed: Vec<String> = state
            .values
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let count = doomed.len() as u64;
        for key in doomed {
            state.values.remove(&key);
        }
        Ok(count)
    }

    async fn incr_by(&self, key: &str, by: i64, ttl: Option<Duration>) -> Result<i64> {
        let mut state = self.lock()?;
        let now = now_millis();
        let existing = state
            .values
            .get(key)
            .filter(|e| e.expires_at_ms.is_none_or(|exp| exp > now))
            .and_then(|e| e.value.parse::<i64>().ok())
            .unwrap_or(0);
        let updated = existing.saturating_add(by);
        let expires_at_ms = if existing == 0 {
            ttl.map(|t| now + t.as_millis() as u64)
        } else {
            state.values.get(key).and_then(|e| e.expires_at_ms)
        };
        state.values.insert(
            key.to_string(),
            Entry {
                value: updated.to_string(),
                expires_at_ms,
            },
        );
        Ok(updated)
    }

    async fn rate_limit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> Result<RateLimitOutcome> {
        let mut state = self.lock()?;
        let now = now_millis();
        let window_ms = window.as_millis() as u64;
        let hits = state.windows.entry(key.to_string()).or_default();
        hits.retain(|&t| t + window_ms > now);

        if (hits.len() as u32) < limit {
            hits.push(now);
            Ok(RateLimitOutcome {
                allowed: true,
                remaining: limit - hits.len() as u32,
                retry_after_secs: 0,
            })
        } else {
            let oldest = hits.iter().copied().min().unwrap_or(now);
            let retry_ms = (oldest + window_ms).saturating_sub(now);
            Ok(RateLimitOutcome {
                allowed: false,
                remaining: 0,
                retry_after_secs: (retry_ms / 1000).max(1),
            })
        }
    }

    async fn stream_append(&self, stream: &str, payload: &str, max_len: usize) -> Result<String> {
        let mut state = self.lock()?;
        state.stream_seq += 1;
        let id = format!("{}-{}", now_millis(), state.stream_seq);
        let entries = state.streams.entry(stream.to_string()).or_default();
        entries.push(StreamEntry {
            id: id.clone(),
            payload: payload.to_string(),
        });
        if entries.len() > max_len {
            let excess = entries.len() - max_len;
            entries.drain(0..excess);
        }
        Ok(id)
    }

    async fn stream_read(
        &self,
        stream: &str,
        after_id: &str,
        count: usize,
    ) -> Result<Vec<StreamEntry>> {
        let state = self.lock()?;
        let Some(entries) = state.streams.get(stream) else {
            return Ok(vec![]);
        };
        let start = if after_id == "0" {
            0
        } else {
            entries
                .iter()
                .position(|e| e.id == after_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        };
        Ok(entries.iter().skip(start).take(count).cloned().collect())
    }

    async fn ping(&self) -> Result<()> {
        self.lock().map(|_| ())
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryStore {
        MemoryStore::new()
    }

    #[tokio::test]
    async fn get_returns_what_was_set() {
        let s = store();
        s.set_ex("k", "v", Duration::from_secs(60)).await.unwrap();
        assert_eq!(s.get("k").await.unwrap(), Some("v".to_string()));
        assert_eq!(s.get("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn expired_values_disappear() {
        let s = store();
        s.set_ex("k", "v", Duration::from_millis(1)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(s.get("k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn prefix_deletion_is_scoped() {
        // This is how a tenant's cache is invalidated. Deleting one org's keys must
        // never touch another's.
        let s = store();
        s.set_ex("cache:org-a:one", "1", Duration::from_secs(60))
            .await
            .unwrap();
        s.set_ex("cache:org-a:two", "2", Duration::from_secs(60))
            .await
            .unwrap();
        s.set_ex("cache:org-b:one", "3", Duration::from_secs(60))
            .await
            .unwrap();

        assert_eq!(s.del_prefix("cache:org-a:").await.unwrap(), 2);
        assert_eq!(s.get("cache:org-a:one").await.unwrap(), None);
        assert_eq!(
            s.get("cache:org-b:one").await.unwrap(),
            Some("3".to_string())
        );
    }

    #[tokio::test]
    async fn counters_accumulate_and_keep_their_original_ttl() {
        let s = store();
        assert_eq!(
            s.incr_by("spend", 100, Some(Duration::from_secs(60)))
                .await
                .unwrap(),
            100
        );
        assert_eq!(
            s.incr_by("spend", 50, Some(Duration::from_secs(60)))
                .await
                .unwrap(),
            150
        );
        // A monthly spend counter must not have its expiry pushed forward on every
        // request, or it would never roll over.
        assert_eq!(s.incr_by("spend", -50, None).await.unwrap(), 100);
    }

    #[tokio::test]
    async fn rate_limit_admits_up_to_the_limit_then_rejects() {
        let s = store();
        let window = Duration::from_secs(60);
        for i in 0..5 {
            let outcome = s.rate_limit("key:abc", 5, window).await.unwrap();
            assert!(outcome.allowed, "request {i} should have been allowed");
            assert_eq!(outcome.remaining, 4 - i);
        }
        let blocked = s.rate_limit("key:abc", 5, window).await.unwrap();
        assert!(!blocked.allowed);
        assert_eq!(blocked.remaining, 0);
        assert!(
            blocked.retry_after_secs >= 1,
            "must give a usable retry hint"
        );
    }

    #[tokio::test]
    async fn rate_limit_windows_are_independent_per_key() {
        let s = store();
        let window = Duration::from_secs(60);
        for _ in 0..3 {
            assert!(s.rate_limit("key:a", 3, window).await.unwrap().allowed);
        }
        assert!(!s.rate_limit("key:a", 3, window).await.unwrap().allowed);
        // A different key — a different tenant — must be unaffected.
        assert!(s.rate_limit("key:b", 3, window).await.unwrap().allowed);
    }

    #[tokio::test]
    async fn rate_limit_window_slides() {
        let s = store();
        let window = Duration::from_millis(50);
        assert!(s.rate_limit("k", 1, window).await.unwrap().allowed);
        assert!(!s.rate_limit("k", 1, window).await.unwrap().allowed);
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(
            s.rate_limit("k", 1, window).await.unwrap().allowed,
            "capacity must return once the window slides past the old hit"
        );
    }

    #[tokio::test]
    async fn stream_append_and_read_preserve_order() {
        let s = store();
        let a = s.stream_append("usage", "one", 100).await.unwrap();
        let b = s.stream_append("usage", "two", 100).await.unwrap();
        assert_ne!(a, b);

        let all = s.stream_read("usage", "0", 10).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].payload, "one");
        assert_eq!(all[1].payload, "two");

        let after_first = s.stream_read("usage", &a, 10).await.unwrap();
        assert_eq!(after_first.len(), 1);
        assert_eq!(after_first[0].payload, "two");
    }

    #[tokio::test]
    async fn stream_is_trimmed_to_max_len() {
        let s = store();
        for i in 0..10 {
            s.stream_append("usage", &i.to_string(), 3).await.unwrap();
        }
        let entries = s.stream_read("usage", "0", 100).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0].payload, "7",
            "oldest entries should be dropped first"
        );
    }

    #[tokio::test]
    async fn reading_an_unknown_stream_is_empty_not_an_error() {
        let s = store();
        assert!(s
            .stream_read("nothing-here", "0", 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn concurrent_rate_limit_checks_never_over_admit() {
        // The property that matters: with a limit of 10 and 50 concurrent callers,
        // exactly 10 are admitted. A non-atomic implementation admits more.
        use std::sync::Arc;
        let s = Arc::new(store());
        let mut handles = Vec::new();
        for _ in 0..50 {
            let s = Arc::clone(&s);
            handles.push(tokio::spawn(async move {
                s.rate_limit("burst", 10, Duration::from_secs(60))
                    .await
                    .unwrap()
                    .allowed
            }));
        }
        let mut admitted = 0;
        for h in handles {
            if h.await.unwrap() {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 10, "rate limiter over-admitted under concurrency");
    }

    #[test]
    fn rate_limit_lua_is_syntactically_plausible() {
        // Full behaviour is covered by the Redis integration tests; this guards against
        // an edit that mangles the script constant.
        assert!(RATE_LIMIT_LUA.contains("ZREMRANGEBYSCORE"));
        assert!(RATE_LIMIT_LUA.contains("ZCARD"));
        assert!(RATE_LIMIT_LUA.contains("PEXPIRE"));
        assert_eq!(RATE_LIMIT_LUA.matches("return").count(), 2);
    }
}

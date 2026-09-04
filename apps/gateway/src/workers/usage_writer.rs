//! Usage writer — pipeline stage [12].
//!
//! Consumes the Redis usage stream in batches and persists to `usage_records`. This is the
//! bridge between the hot path (which must never touch PostgreSQL, per Principle 1) and
//! the billing source of truth (which must never lose a record, per Principle 2).
//!
//! # Why the cursor is stored in Redis
//!
//! The worker records its stream position after each batch. On restart it resumes from
//! there, so a deploy mid-batch re-reads a few entries rather than skipping them.
//! Re-reading is safe because the insert is idempotent on `request_id`; skipping would be
//! unrecoverable revenue loss. Given the choice, this worker always re-reads.

use crate::db::repo;
use crate::error::Result;
use crate::metering::usage::{UsageEvent, USAGE_STREAM};
use crate::store::KvStore;
use crate::AppState;
use std::time::Duration;

/// Redis key holding the last processed stream id.
pub const CURSOR_KEY: &str = "aegis:usage_writer:cursor";

/// Entries consumed per batch.
pub const BATCH_SIZE: usize = 100;

/// Pause between polls when the stream is empty.
pub const IDLE_INTERVAL: Duration = Duration::from_millis(100);

/// How long the cursor persists. Far longer than any plausible outage.
const CURSOR_TTL: Duration = Duration::from_secs(30 * 24 * 3_600);

/// Outcome of processing one batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchResult {
    /// Entries read from the stream.
    pub read: usize,
    /// Rows newly inserted.
    pub inserted: usize,
    /// Entries that were already persisted (redelivery).
    pub duplicates: usize,
    /// Entries that could not be decoded.
    pub malformed: usize,
}

impl BatchResult {
    /// True when there was nothing to do.
    pub fn is_empty(&self) -> bool {
        self.read == 0
    }
}

/// Read the stored cursor, or the beginning of the stream.
pub async fn read_cursor(store: &dyn KvStore) -> String {
    store
        .get(CURSOR_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "0".to_string())
}

/// Persist the cursor.
async fn write_cursor(store: &dyn KvStore, id: &str) {
    let _ = store.set_ex(CURSOR_KEY, id, CURSOR_TTL).await;
}

/// Process one batch of usage events.
pub async fn process_batch(state: &AppState) -> Result<BatchResult> {
    let cursor = read_cursor(state.store.as_ref()).await;
    let entries = state
        .store
        .stream_read(USAGE_STREAM, &cursor, BATCH_SIZE)
        .await?;

    if entries.is_empty() {
        return Ok(BatchResult::default());
    }

    let mut result = BatchResult {
        read: entries.len(),
        ..Default::default()
    };
    let pool = state.db()?;
    let mut last_id = cursor;

    for entry in entries {
        match serde_json::from_str::<UsageEvent>(&entry.payload) {
            Ok(event) => {
                match repo::insert_usage_record(pool, &event).await {
                    Ok(true) => {
                        result.inserted += 1;
                        // Rollups are derived data; a failure here is worth logging but
                        // must not stop the authoritative insert stream from advancing.
                        if let Err(e) = repo::upsert_daily_aggregate(pool, &event).await {
                            tracing::warn!(error = %e, "daily rollup failed");
                        }
                    }
                    Ok(false) => result.duplicates += 1,
                    Err(e) => {
                        // Do NOT advance the cursor past a row we failed to write: the
                        // whole point of this worker is that nothing is lost. Stop the
                        // batch here and retry from this entry next tick.
                        tracing::error!(
                            error = %e,
                            request_id = %event.request_id,
                            "usage record insert failed; halting batch to retry"
                        );
                        write_cursor(state.store.as_ref(), &last_id).await;
                        return Ok(result);
                    }
                }
            }
            Err(e) => {
                // A malformed entry can never become valid, so retrying it forever would
                // wedge the stream. Count it, log it loudly, and move on.
                tracing::error!(error = %e, "malformed usage event discarded");
                result.malformed += 1;
            }
        }
        last_id = entry.id;
    }

    write_cursor(state.store.as_ref(), &last_id).await;
    Ok(result)
}

/// Run the writer until the process ends.
pub async fn run(state: AppState) {
    tracing::info!("usage writer started");

    loop {
        match process_batch(&state).await {
            Ok(result) if result.is_empty() => {
                tokio::time::sleep(IDLE_INTERVAL).await;
            }
            Ok(result) => {
                tracing::debug!(
                    read = result.read,
                    inserted = result.inserted,
                    duplicates = result.duplicates,
                    "usage batch persisted"
                );
                if result.malformed > 0 {
                    tracing::warn!(count = result.malformed, "malformed usage events discarded");
                }
                // A full batch probably means more is waiting; poll again immediately.
                if result.read < BATCH_SIZE {
                    tokio::time::sleep(IDLE_INTERVAL).await;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "usage writer batch failed");
                // Back off harder on failure so a database outage does not become a hot
                // loop hammering a server that is already struggling.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Keep monthly partitions ahead of need.
///
/// A missing partition rejects inserts into the billing source of truth, so this runs at
/// startup and hourly thereafter.
pub async fn run_partition_maintenance(state: AppState) {
    let mut ticker = tokio::time::interval(Duration::from_secs(3_600));
    loop {
        ticker.tick().await;
        if let Some(pool) = state.db.as_ref() {
            match crate::db::pool::maintain_partitions(pool).await {
                Ok(partitions) => {
                    tracing::debug!(?partitions, "usage partitions verified")
                }
                Err(e) => tracing::error!(error = %e, "partition maintenance failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metering::savings::SavingsBreakdown;
    use crate::metering::usage;
    use crate::money::MicroCents;
    use crate::store::MemoryStore;
    use crate::types::{CacheOutcome, RoutingReason, TokenUsage};
    use uuid::Uuid;

    fn event(org_id: Uuid) -> UsageEvent {
        UsageEvent::new(
            Uuid::new_v4(),
            org_id,
            Some(Uuid::new_v4()),
            None,
            "gpt-4o".into(),
            "gpt-4o-mini".into(),
            "openai".into(),
            TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                estimated: false,
                ..Default::default()
            },
            SavingsBreakdown::compute(MicroCents(7_500), MicroCents(450), 2_000),
            200,
            0.4,
            CacheOutcome::Miss,
            RoutingReason::Complexity,
            Some(0.2),
            200,
        )
    }

    #[tokio::test]
    async fn an_empty_stream_yields_an_empty_batch() {
        let state = AppState::for_tests();
        let result = process_batch(&state).await.unwrap();
        assert!(result.is_empty());
        assert_eq!(result.inserted, 0);
    }

    #[tokio::test]
    async fn the_cursor_starts_at_the_beginning_of_the_stream() {
        let store = MemoryStore::new();
        assert_eq!(read_cursor(&store).await, "0");
    }

    #[tokio::test]
    async fn the_cursor_advances_and_persists() {
        let store = MemoryStore::new();
        write_cursor(&store, "1234-5").await;
        assert_eq!(read_cursor(&store).await, "1234-5");
    }

    #[tokio::test]
    async fn events_are_readable_from_the_stream_in_order() {
        // The writer's input contract: what the hot path emits is what the worker reads.
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();

        let mut expected = Vec::new();
        for _ in 0..5 {
            let event = event(org_id);
            expected.push(event.request_id);
            usage::emit(&store, &event).await.unwrap();
        }

        let entries = store
            .stream_read(USAGE_STREAM, "0", BATCH_SIZE)
            .await
            .unwrap();
        assert_eq!(entries.len(), 5);

        let decoded: Vec<Uuid> = entries
            .iter()
            .map(|e| {
                serde_json::from_str::<UsageEvent>(&e.payload)
                    .unwrap()
                    .request_id
            })
            .collect();
        assert_eq!(decoded, expected);
    }

    #[tokio::test]
    async fn reading_resumes_after_the_cursor() {
        let store = MemoryStore::new();
        let org_id = Uuid::new_v4();
        for _ in 0..10 {
            usage::emit(&store, &event(org_id)).await.unwrap();
        }

        let first = store.stream_read(USAGE_STREAM, "0", 4).await.unwrap();
        assert_eq!(first.len(), 4);

        let next = store
            .stream_read(USAGE_STREAM, &first.last().unwrap().id, 100)
            .await
            .unwrap();
        assert_eq!(next.len(), 6, "resuming must not re-read or skip");
    }

    #[tokio::test]
    async fn a_batch_without_a_database_reports_the_error_rather_than_losing_events() {
        // Running without persistence must not silently drain the stream.
        let state = AppState::for_tests();
        usage::emit(state.store.as_ref(), &event(Uuid::new_v4()))
            .await
            .unwrap();

        let result = process_batch(&state).await;
        assert!(
            result.is_err(),
            "expected a 503-shaped error with no database"
        );

        // The event is still in the stream, waiting.
        let entries = state
            .store
            .stream_read(USAGE_STREAM, "0", 10)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(read_cursor(state.store.as_ref()).await, "0");
    }

    #[test]
    fn batch_results_summarise_correctly() {
        let empty = BatchResult::default();
        assert!(empty.is_empty());

        let processed = BatchResult {
            read: 10,
            inserted: 8,
            duplicates: 2,
            malformed: 0,
        };
        assert!(!processed.is_empty());
        assert_eq!(processed.inserted + processed.duplicates, processed.read);
    }

    #[test]
    fn the_batch_size_is_within_the_stream_cap() {
        // A batch larger than the stream's own trim length could never fill.
        const { assert!(BATCH_SIZE < crate::metering::usage::USAGE_STREAM_MAX_LEN) };
        const { assert!(BATCH_SIZE > 0) };
    }
}

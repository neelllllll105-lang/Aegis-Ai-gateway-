//! Periodic pricing refresh.
//!
//! # The problem this closes
//!
//! `AppState::pricing` used to be loaded once, at process startup, into a bare
//! `Arc<PricingTable>` with no way to replace it short of restarting the process. That
//! meant `docs/runbooks/pricing-update.md`'s own step 5 — "restart the gateway" — was not
//! an inconvenience, it was structural: a human could run the entire runbook correctly,
//! commit a verified price to `model_pricing`, and every request in between that commit
//! and the next deploy would still bill at the old rate, silently.
//!
//! # What this does and does not fix
//!
//! No LLM provider exposes a live pricing API — prices are prose on marketing pages, not
//! a queryable service — so there is no way to check "is this still the real price" on
//! every request, or even every hour, against the provider itself. That gap cannot be
//! engineered away; a human still has to read the page. What *can* be engineered away is
//! the gap between "a human has already verified a price and committed it to the
//! database" and "the gateway is serving it" — that gap used to be "until the next
//! deploy" and is now at most [`REFRESH_INTERVAL`], or instant via the manual
//! `POST /api/admin/pricing/reload` endpoint.
//!
//! # Correctness under a race
//!
//! A refresh landing mid-request cannot corrupt that request's bill. `AppState::pricing`
//! hands out an owned `Arc<PricingTable>`; a handler that has already read it keeps using
//! that exact table to completion no matter what `set_pricing` does afterward, because
//! swapping the cell does not touch `Arc`s already handed out. The computed cost is then
//! baked into the `UsageEvent` at the moment of computation, not re-derived later from
//! "whatever the table currently says" — so a request served one second before a price
//! change and a request served one second after are each billed at the rate that was
//! actually in force for them, and neither can retroactively change the other.

use crate::metering::pricing::{PricingSource, PricingTable};
use crate::AppState;
use std::time::Duration;

/// How often the in-memory pricing table is re-read from the database.
///
/// This is not the runbook's monthly human-verification cadence — it is only how long a
/// *already-committed* price can sit unused before every replica picks it up on its own.
/// Five minutes means nobody has to remember to call the manual reload endpoint after
/// running the runbook, and a single indexed `SELECT ... WHERE effective_to IS NULL` every
/// five minutes costs nothing worth optimizing against — the health probe already runs a
/// comparable query every fifteen seconds.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Refresh forever. Spawned once per process, only when a database is configured.
pub async fn run(state: AppState) {
    let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
    // The first tick fires immediately; startup already loaded pricing once, so skip it
    // rather than doing the same read twice in the first second of the process's life.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        match refresh_once(&state).await {
            Ok(Some(count)) => {
                tracing::info!(models = count, "pricing table refreshed from database")
            }
            Ok(None) => {} // nothing to do: no database, or the table was intentionally left untouched
            Err(e) => tracing::error!(
                error = %e,
                "pricing refresh failed; continuing to serve the previously loaded table"
            ),
        }
    }
}

/// One refresh attempt.
///
/// `Ok(None)` means nothing happened (no database configured). `Ok(Some(n))` means the
/// table was reloaded with `n` rows. Any error leaves the previously loaded table in
/// place — a failed refresh must never mean "stop pricing requests", since the table that
/// was already loaded is still a perfectly valid table to keep serving from.
pub async fn refresh_once(state: &AppState) -> crate::error::Result<Option<usize>> {
    let Some(pool) = state.db.as_ref() else {
        return Ok(None);
    };

    let rows = crate::db::repo::load_pricing(pool).await?;
    if rows.is_empty() {
        // An empty result is far more likely to be a mistake — a truncated table, a
        // migration that half-ran — than a deliberate "price nothing" state. Swapping to
        // an empty table would make every request fail to price rather than merely serve
        // a table that is a few minutes stale, which is a strictly worse failure mode.
        tracing::warn!(
            "model_pricing returned zero rows on refresh; keeping the previously loaded \
             pricing table rather than swapping to an empty one"
        );
        return Ok(None);
    }

    let count = rows.len();
    let table = PricingTable::from_models(rows.into_iter().map(Into::into).collect());
    state.set_pricing(table, PricingSource::Database);
    Ok(Some(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn without_a_database_refresh_is_a_no_op() {
        let state = AppState::for_tests();
        let before = state.pricing().len();

        let result = refresh_once(&state).await.unwrap();

        assert_eq!(result, None);
        assert_eq!(
            state.pricing().len(),
            before,
            "no database means nothing to refresh from; the seed table must be untouched"
        );
    }

    #[tokio::test]
    async fn a_request_that_already_read_the_table_is_unaffected_by_a_later_swap() {
        // The correctness property this whole worker depends on: an Arc already handed
        // out to a handler is immune to what happens to the cell afterward.
        let state = AppState::for_tests();
        let held = state.pricing();
        let price_before = held.get("openai/gpt-4o").unwrap().input_per_mtok;

        state.set_pricing(PricingTable::new(), PricingSource::Database);

        assert_eq!(
            held.get("openai/gpt-4o").unwrap().input_per_mtok,
            price_before,
            "a table already read must not change out from under an in-flight request"
        );
        assert!(
            state.pricing().get("openai/gpt-4o").is_none(),
            "a *new* read must see the swapped-in table"
        );
    }
}

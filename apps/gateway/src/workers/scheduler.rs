//! Periodic jobs, and the lock that stops every replica running them at once.
//!
//! # The problem this module exists to solve
//!
//! A weekly digest is a `tokio::time::interval` and twenty lines of SQL. That works
//! perfectly on one process and is actively harmful on four: every replica wakes on the
//! same schedule, every replica queries the same organisations, and every customer
//! receives four identical emails. The same is true of the nightly pricing check and of
//! budget alerts — anything that has an external side effect.
//!
//! So the unit of work here is not "a timer" but "a claim". Before doing anything
//! observable, a replica asks the shared store to atomically claim a named slot for a
//! named period. Exactly one replica gets it; the rest do nothing and go back to sleep.
//!
//! The claim is built on `incr_by`, not on a get-then-set, because get-then-set is a
//! race: two replicas both read "unclaimed" and both write "claimed". `incr_by` returns
//! the post-increment value, so precisely one caller can ever observe `1`. Both store
//! implementations provide that atomically — Redis natively, the memory store under its
//! own lock — so the behaviour is identical in development and production.

use crate::error::Result;
use crate::metering::pricing::PricingTable;
use crate::store::KvStore;
use crate::AppState;
use chrono::{Datelike, Timelike, Utc};
use std::time::Duration;

/// How often the scheduler wakes to consider its jobs.
///
/// One minute. The jobs themselves decide whether it is their turn, so this only bounds
/// how late a job can be, and a digest that goes out at 09:00:47 instead of 09:00:00 has
/// never mattered to anyone.
const TICK: Duration = Duration::from_secs(60);

/// How long a claim is held.
///
/// Longer than any job takes, shorter than the shortest gap between runs. Two hours fits
/// both: no job here runs for hours, and no job runs twice within two hours.
const CLAIM_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// Try to claim `job` for `period`. Returns true for exactly one caller.
///
/// `period` is a coarse timestamp — `"2026-W34"` for a weekly job, `"2026-08-21"` for a
/// nightly one. Including it in the key is what makes the claim expire naturally: next
/// period is a different key, so nothing has to be cleaned up.
pub async fn claim(store: &dyn KvStore, job: &str, period: &str) -> bool {
    let key = format!("aegis:sched:{job}:{period}");

    match store.incr_by(&key, 1, Some(CLAIM_TTL)).await {
        // First and only caller to see 1.
        Ok(1) => true,
        Ok(_) => false,
        Err(e) => {
            // A store failure must not become a duplicate send. Declining the claim means
            // the job is skipped this period, which is recoverable; assuming the claim
            // means every replica proceeds, which is not.
            tracing::warn!(job, period, error = %e, "could not claim scheduled job; skipping");
            false
        }
    }
}

/// ISO-week identifier, e.g. `2026-W34`.
fn week_key(now: chrono::DateTime<Utc>) -> String {
    let iso = now.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

/// Day identifier, e.g. `2026-08-21`.
fn day_key(now: chrono::DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// Whether it is time to attempt the weekly digest.
///
/// Monday, 09:00 UTC. Checked as an inequality rather than an equality so a replica that
/// was asleep, restarting, or briefly partitioned at exactly 09:00 still sends it — the
/// claim is what prevents a duplicate, so being generous here is free.
fn is_digest_window(now: chrono::DateTime<Utc>) -> bool {
    now.weekday() == chrono::Weekday::Mon && now.hour() >= 9
}

/// Whether it is time to attempt the nightly pricing check. 03:00 UTC onwards.
fn is_pricing_window(now: chrono::DateTime<Utc>) -> bool {
    now.hour() >= 3
}

/// Run the scheduler until the process stops.
pub async fn run(state: AppState) {
    let mut ticker = tokio::time::interval(TICK);
    // If the process was paused (a laptop lid, a stopped container), do not then fire a
    // burst of catch-up ticks — the jobs are idempotent per period anyway, so the extra
    // work would be pure waste.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!("scheduler started");

    loop {
        ticker.tick().await;
        let now = Utc::now();

        if is_digest_window(now)
            && claim(state.store.as_ref(), "weekly-digest", &week_key(now)).await
        {
            if let Err(e) = send_weekly_digests(&state).await {
                tracing::error!(error = %e, "weekly digest run failed");
            }
        }

        if is_pricing_window(now)
            && claim(state.store.as_ref(), "pricing-check", &day_key(now)).await
        {
            if let Err(e) = check_pricing_drift(&state).await {
                tracing::error!(error = %e, "pricing drift check failed");
            }
        }
    }
}

/// Send one digest per organisation that actually used the gateway last week.
///
/// Organisations with no traffic are skipped. A weekly email saying "you saved $0.00"
/// teaches the recipient that Aegis email is noise, and the next one — the budget alert
/// that matters — gets filtered with it.
async fn send_weekly_digests(state: &AppState) -> Result<()> {
    let pool = state.db()?;
    let organisations = crate::db::repo::orgs_with_recent_usage(pool, 7).await?;

    tracing::info!(count = organisations.len(), "sending weekly digests");

    let mut sent = 0usize;
    for org in organisations {
        let to = Utc::now();
        let from = to - chrono::Duration::days(7);
        let summary = crate::db::repo::usage_summary(pool, org.id, from, to).await?;

        if summary.requests == 0 {
            continue;
        }

        let cache_hit_rate = if summary.requests > 0 {
            summary.cache_hits as f64 / summary.requests as f64 * 100.0
        } else {
            0.0
        };

        let alert = super::budget_alerts::render_weekly_digest(
            &org.name,
            summary.requests,
            crate::money::MicroCents(summary.actual_cost_mc),
            crate::money::MicroCents(summary.gross_savings_mc),
            crate::money::MicroCents(summary.aegis_fee_mc),
            cache_hit_rate,
        );

        // One organisation failing to receive its digest must not stop the rest. This is
        // a loop over customers, and an unwrapped error here would mean the first bad
        // billing address silences everyone alphabetically after it.
        if let Some(address) = org.billing_email.as_deref() {
            match super::budget_alerts::send_email(
                &state.http,
                &state.config,
                address,
                &alert.subject,
                &alert.body,
            )
            .await
            {
                Ok(true) => sent += 1,
                // `false` means no email provider is configured, so the digest was logged
                // rather than delivered. Not an error, and not a send either.
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(org_id = %org.id, error = %e, "weekly digest delivery failed")
                }
            }
        }
    }

    tracing::info!(sent, "weekly digests complete");
    Ok(())
}

/// Nightly pricing drift check (P7.5).
///
/// This deliberately does **not** update prices. A model price is what an invoice is
/// computed from, and silently rewriting it from a scraped source means a customer's bill
/// can change because a marketing page changed. Instead it compares the live table
/// against what the database holds and writes a proposal a human approves.
async fn check_pricing_drift(state: &AppState) -> Result<()> {
    let pool = state.db()?;
    let stored = crate::db::repo::load_pricing(pool).await?;
    let report = diff_pricing(&state.pricing, &stored);

    if report.is_empty() {
        tracing::info!("nightly pricing check: no drift");
        return Ok(());
    }

    for line in &report {
        tracing::warn!(target: "aegis::pricing_drift", "{line}");
    }
    tracing::warn!(
        count = report.len(),
        "nightly pricing check found drift — review docs/runbooks/pricing-update.md"
    );

    Ok(())
}

/// Compare the in-memory pricing table against stored rows.
///
/// Split out from the job so it is testable without a database, and so the wording of a
/// drift line is asserted rather than eyeballed in a log.
pub fn diff_pricing(table: &PricingTable, stored: &[crate::db::repo::PricingRow]) -> Vec<String> {
    let mut lines = Vec::new();

    for row in stored {
        match table.get(&row.model_id) {
            Some(model) => {
                if model.input_per_mtok.as_i64() != row.input_cost_per_mtok_mc
                    || model.output_per_mtok.as_i64() != row.output_cost_per_mtok_mc
                {
                    lines.push(format!(
                        "{}: stored {}/{} µ¢ per Mtok, table has {}/{} µ¢",
                        row.model_id,
                        row.input_cost_per_mtok_mc,
                        row.output_cost_per_mtok_mc,
                        model.input_per_mtok.as_i64(),
                        model.output_per_mtok.as_i64(),
                    ));
                }
            }
            None => lines.push(format!(
                "{}: present in the database but missing from the pricing table",
                row.model_id
            )),
        }
    }

    // A model the table knows and the database does not is equally a drift — it means a
    // migration was missed and the price in production is whatever the binary shipped
    // with, which nobody reviewed.
    for model in table.all() {
        if !stored.iter().any(|row| row.model_id == model.model_id) {
            lines.push(format!(
                "{}: present in the pricing table but missing from the database",
                model.model_id
            ));
        }
    }

    lines.sort();
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::PricingRow;
    use crate::store::MemoryStore;
    use chrono::TimeZone;

    fn at(year: i32, month: u32, day: u32, hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn only_one_caller_can_claim_a_period() {
        let store = MemoryStore::new();

        assert!(claim(&store, "digest", "2026-W34").await);
        assert!(!claim(&store, "digest", "2026-W34").await);
        assert!(!claim(&store, "digest", "2026-W34").await);
    }

    #[tokio::test]
    async fn a_new_period_can_be_claimed_again() {
        let store = MemoryStore::new();

        assert!(claim(&store, "digest", "2026-W34").await);
        assert!(claim(&store, "digest", "2026-W35").await);
    }

    #[tokio::test]
    async fn different_jobs_do_not_block_each_other() {
        let store = MemoryStore::new();

        assert!(claim(&store, "digest", "2026-W34").await);
        assert!(claim(&store, "pricing-check", "2026-W34").await);
    }

    #[tokio::test]
    async fn concurrent_replicas_produce_exactly_one_winner() {
        // The whole point of the module. If this ever fails, every customer gets N
        // copies of every email.
        let store = std::sync::Arc::new(MemoryStore::new());
        let mut handles = Vec::new();

        for _ in 0..64 {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                claim(store.as_ref(), "digest", "2026-W34").await
            }));
        }

        let mut winners = 0;
        for handle in handles {
            if handle.await.unwrap() {
                winners += 1;
            }
        }

        assert_eq!(winners, 1, "exactly one replica may run a scheduled job");
    }

    #[test]
    fn the_digest_window_is_monday_morning_onwards() {
        // 2026-08-17 is a Monday.
        assert!(!is_digest_window(at(2026, 8, 17, 8)));
        assert!(is_digest_window(at(2026, 8, 17, 9)));
        assert!(is_digest_window(at(2026, 8, 17, 23)));
        // Tuesday.
        assert!(!is_digest_window(at(2026, 8, 18, 9)));
    }

    #[test]
    fn the_pricing_window_opens_at_three() {
        assert!(!is_pricing_window(at(2026, 8, 17, 2)));
        assert!(is_pricing_window(at(2026, 8, 17, 3)));
    }

    #[test]
    fn week_keys_are_stable_within_a_week_and_change_across_one() {
        assert_eq!(week_key(at(2026, 8, 17, 9)), week_key(at(2026, 8, 20, 9)));
        assert_ne!(week_key(at(2026, 8, 17, 9)), week_key(at(2026, 8, 25, 9)));
    }

    fn row(model_id: &str, input: i64, output: i64) -> PricingRow {
        PricingRow {
            model_id: model_id.to_string(),
            provider: "openai".into(),
            tier: "standard".into(),
            display_name: model_id.to_string(),
            input_cost_per_mtok_mc: input,
            output_cost_per_mtok_mc: output,
            context_window: 128_000,
            supports_vision: false,
            supports_tools: true,
            is_active: true,
            source: "test".into(),
            ..Default::default()
        }
    }

    #[test]
    fn identical_prices_produce_no_drift() {
        let table = PricingTable::with_seed_data();
        let model = table.all().next().expect("seed data is not empty");

        let stored = vec![row(
            &model.model_id,
            model.input_per_mtok.as_i64(),
            model.output_per_mtok.as_i64(),
        )];

        // Only the row we constructed is compared for equality; the table has many more
        // models, so filter the report down to that one. The colon anchors the match to
        // the whole model id: `starts_with(&model.model_id)` alone would also match a
        // sibling like "openai/gpt-4o-mini" against the prefix "openai/gpt-4o", and this
        // pricing table genuinely contains such prefix-colliding pairs (gpt-4o /
        // gpt-4o-mini, gpt-4.1 / gpt-4.1-mini / gpt-4.1-nano, gemini-2.5-flash /
        // gemini-2.5-flash-lite). Caught when a real second contributor's commit added
        // two more models and happened to shift which entry `table.all().next()` yields.
        let needle = format!("{}:", model.model_id);
        let drift = diff_pricing(&table, &stored);
        assert!(
            !drift.iter().any(|line| line.starts_with(&needle)),
            "a matching price must not be reported as drift: {drift:?}"
        );
    }

    #[test]
    fn a_changed_price_is_reported_with_both_values() {
        let table = PricingTable::with_seed_data();
        let model = table.all().next().expect("seed data is not empty");
        let model_id = model.model_id.clone();
        let real_input = model.input_per_mtok.as_i64();

        let stored = vec![row(&model_id, real_input + 50_000, 999_999)];
        let drift = diff_pricing(&table, &stored);

        // Same colon-anchored match as above, for the same reason: an unanchored prefix
        // check can grab a sibling model's "missing from database" line instead of this
        // one's "stored X, table has Y" line, and the two are indistinguishable to
        // `starts_with` alone.
        let needle = format!("{model_id}:");
        let line = drift
            .iter()
            .find(|line| line.starts_with(&needle))
            .expect("the changed model must be reported");

        // Both numbers must appear, because a drift alert that says only "changed" sends
        // whoever reads it straight back to the database to find out by how much.
        assert!(line.contains(&(real_input + 50_000).to_string()));
        assert!(line.contains(&real_input.to_string()));
    }

    #[test]
    fn a_model_missing_from_the_database_is_reported() {
        let table = PricingTable::with_seed_data();
        let drift = diff_pricing(&table, &[]);

        assert!(
            drift
                .iter()
                .all(|line| line.contains("missing from the database")),
            "with no stored rows every model should be reported as missing: {drift:?}"
        );
        assert!(!drift.is_empty());
    }

    #[test]
    fn a_model_missing_from_the_table_is_reported() {
        let table = PricingTable::with_seed_data();
        let stored = vec![row("model-that-does-not-exist", 1, 1)];
        let drift = diff_pricing(&table, &stored);

        assert!(drift.iter().any(|line| line.contains(
            "model-that-does-not-exist: present in the database but missing from the pricing table"
        )));
    }
}

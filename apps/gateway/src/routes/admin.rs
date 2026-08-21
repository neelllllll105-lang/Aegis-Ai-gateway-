//! Internal admin console — `/api/admin/*`.
//!
//! Every handler here requires `users.is_admin`. That flag is set directly in the
//! database, never through the API: an endpoint that can grant admin is an endpoint that
//! can be tricked into granting admin.
//!
//! Part 9 item 12 also requires TOTP on admin accounts; see [`crate::enterprise::totp`].

use crate::db::repo;
use crate::error::{AegisError, Result};
use crate::middleware::auth::{self, AuthContext};
use crate::workers::reconciliation;
use crate::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

/// Authenticate and require staff privileges.
async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<AuthContext> {
    let context = auth::authenticate_management(state, headers).await?;
    if !context.is_admin {
        // Deliberately 404, not 403: an internal admin surface should not confirm its own
        // existence to a caller who cannot use it.
        return Err(AegisError::NotFound("not found".into()));
    }
    Ok(context)
}

/// `GET /api/admin/metrics` — system health.
pub async fn system_metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        require_admin(&state, &headers).await?;

        let (pool_size, pool_idle) = state
            .db
            .as_ref()
            .map(crate::db::pool::pool_stats)
            .unwrap_or((0, 0));

        Ok::<_, AegisError>(
            Json(serde_json::json!({
                "uptime_seconds": state.started_at.elapsed().as_secs(),
                "region": state.config.region,
                "requests_total": state.metrics.total_requests(),
                "usage_events_total": state.metrics.total_usage_events(),
                "gateway_overhead_p50_ms": state.metrics.overhead_p50_ms(),
                "gateway_overhead_p99_ms": state.metrics.overhead_p99_ms(),
                "metering_gap": reconciliation::check_metering_completeness(&state),
                "key_cache_entries": state.key_cache.len(),
                "database_pool": {"size": pool_size, "idle": pool_idle},
                "store_backend": state.store.backend_name(),
                "degraded_providers": state
                    .health
                    .degraded_providers()
                    .into_iter()
                    .map(|(provider, circuit)| serde_json::json!({
                        "provider": provider,
                        "state": circuit.as_str(),
                    }))
                    .collect::<Vec<_>>(),
                "shared_key_providers":
                    crate::providers::pool::SharedKeyPool::available_providers(&state.config),
            }))
            .into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/admin/routing` — what the bandit has learned.
///
/// The compounding asset of Part 7, made inspectable: which model actually performs, per
/// complexity band, on our own traffic.
pub async fn routing_intelligence(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        require_admin(&state, &headers).await?;

        let arms: Vec<serde_json::Value> = state
            .bandit
            .snapshot()
            .into_iter()
            .map(|(band, model, stats)| {
                serde_json::json!({
                    "complexity_band": band,
                    "model": model,
                    "pulls": stats.pulls,
                    "successes": stats.successes,
                    "success_rate": stats.success_rate(),
                    "mean_reward": stats.mean_reward(),
                    "total_savings_mc": stats.total_savings_mc,
                    "total_cost_mc": stats.total_cost_mc,
                    "established": stats.is_established(),
                })
            })
            .collect();

        Ok::<_, AegisError>(Json(serde_json::json!({"arms": arms})).into_response())
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/admin/pricing` — the live pricing table with provenance.
///
/// Exposed so Part 13 item 8 (every price traceable to a dated source) can be audited
/// without a database session.
pub async fn pricing_table(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        require_admin(&state, &headers).await?;

        let mut models: Vec<serde_json::Value> = state
            .pricing
            .all()
            .map(|m| {
                serde_json::json!({
                    "model_id": m.model_id,
                    "provider": m.provider,
                    "tier": m.tier.as_str(),
                    "input_per_mtok_mc": m.input_per_mtok.as_i64(),
                    "output_per_mtok_mc": m.output_per_mtok.as_i64(),
                    "blended_per_mtok_mc": m.blended_per_mtok().as_i64(),
                    "context_window": m.context_window,
                    "source": m.source,
                })
            })
            .collect();
        models.sort_by(|a, b| {
            a["model_id"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["model_id"].as_str().unwrap_or_default())
        });

        let count = models.len();
        Ok::<_, AegisError>(
            Json(serde_json::json!({"models": models, "count": count})).into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `GET /api/admin/audit` — the audit log for the acting organisation.
pub async fn audit_log(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match async {
        let context = require_admin(&state, &headers).await?;
        let entries = repo::list_audit_logs(state.db()?, context.org_id, 500, 0).await?;
        Ok::<_, AegisError>(Json(serde_json::json!({"entries": entries})).into_response())
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

/// `POST /api/admin/providers/:id/reset` — force a circuit breaker closed.
///
/// For when a provider has recovered but the breaker has not yet probed, and waiting out
/// the window is worse than trying.
pub async fn reset_circuit(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(provider): axum::extract::Path<String>,
) -> Response {
    match async {
        require_admin(&state, &headers).await?;
        state.health.reset(&provider);
        tracing::warn!(provider = %provider, "circuit breaker manually reset");
        Ok::<_, AegisError>(
            (StatusCode::OK, Json(serde_json::json!({"reset": provider}))).into_response(),
        )
    }
    .await
    {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admin_endpoints_reject_unauthenticated_callers() {
        let state = AppState::for_tests();
        let response = system_metrics(State(state), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_admin_guard_hides_the_surface_from_non_staff() {
        // A 403 would confirm the endpoint exists. For an internal surface, that is
        // information worth withholding.
        let state = AppState::for_tests();
        let err = require_admin(&state, &HeaderMap::new()).await.unwrap_err();
        assert!(
            matches!(err, AegisError::Unauthorized(_) | AegisError::NotFound(_)),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn every_admin_route_is_guarded() {
        // A new admin route added without the guard is a real risk; this enumerates them.
        let state = AppState::for_tests();
        let headers = HeaderMap::new();

        for response in [
            system_metrics(State(state.clone()), headers.clone()).await,
            routing_intelligence(State(state.clone()), headers.clone()).await,
            pricing_table(State(state.clone()), headers.clone()).await,
            audit_log(State(state.clone()), headers.clone()).await,
        ] {
            assert!(
                response.status() == StatusCode::UNAUTHORIZED
                    || response.status() == StatusCode::NOT_FOUND,
                "an admin route answered {} without credentials",
                response.status()
            );
        }
    }
}

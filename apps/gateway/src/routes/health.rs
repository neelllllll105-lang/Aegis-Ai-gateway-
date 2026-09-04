//! Health, readiness, and metrics endpoints.
//!
//! The distinction between the two health endpoints matters operationally and is a
//! frequent source of outages when confused:
//!
//! * **`/health`** — liveness. Is the process up? Answers 200 even when a dependency is
//!   degraded, because restarting the container will not fix a sick database, and a
//!   restart loop during a database blip turns a partial outage into a total one.
//! * **`/ready`** — readiness. Should traffic be routed here? Answers 503 when a
//!   dependency the hot path needs is unavailable, so the load balancer drains this
//!   instance without killing it.

use crate::db::pool;
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::time::Instant;

/// Status of one dependency.
#[derive(Debug, Serialize)]
pub struct DependencyStatus {
    pub name: &'static str,
    pub status: &'static str,
    /// Round-trip time in milliseconds.
    pub latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The `/health` payload.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub region: String,
    pub uptime_seconds: u64,
    pub dependencies: Vec<DependencyStatus>,
    /// Providers whose circuit is not closed.
    pub degraded_providers: Vec<String>,
}

/// `GET /health` — liveness.
pub async fn health(State(state): State<AppState>) -> Response {
    let dependencies = check_dependencies(&state).await;

    let response = HealthResponse {
        // Only a dependency that is configured and *failing* counts as degraded.
        // "not_configured" is a supported development mode, not a fault, and reporting it
        // as degraded would make every local run look broken.
        status: if dependencies.iter().any(|d| d.status == "error") {
            "degraded"
        } else {
            "ok"
        },
        version: env!("CARGO_PKG_VERSION"),
        region: state.config.region.clone(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        dependencies,
        degraded_providers: state
            .health
            .degraded_providers()
            .into_iter()
            .map(|(provider, circuit)| format!("{provider}:{}", circuit.as_str()))
            .collect(),
    };

    // Always 200: the process is alive, and a restart cannot fix a sick dependency.
    (StatusCode::OK, Json(response)).into_response()
}

/// `GET /ready` — readiness.
pub async fn ready(State(state): State<AppState>) -> Response {
    let dependencies = check_dependencies(&state).await;

    // The store is the only hard requirement for the hot path: without it there is no
    // rate limiting, no budget enforcement, and no usage stream. The database is not —
    // the gateway serves traffic from cached key contexts while it is down.
    let store_ok = dependencies
        .iter()
        .any(|d| d.name == "store" && d.status == "ok");

    let status = if store_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(serde_json::json!({
            "ready": store_ok,
            "dependencies": dependencies,
        })),
    )
        .into_response()
}

async fn check_dependencies(state: &AppState) -> Vec<DependencyStatus> {
    let mut dependencies = Vec::with_capacity(2);

    let started = Instant::now();
    let store_result = state.store.ping().await;
    dependencies.push(DependencyStatus {
        name: "store",
        status: if store_result.is_ok() { "ok" } else { "error" },
        latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
        detail: store_result
            .err()
            // Redacted: a connection error can contain a credentialed URL.
            .map(|e| crate::telemetry::redact(&e.to_string())),
    });

    match state.db.as_ref() {
        Some(db) => {
            let started = Instant::now();
            let result = pool::ping(db).await;
            let (size, idle) = pool::pool_stats(db);
            dependencies.push(DependencyStatus {
                name: "database",
                status: if result.is_ok() { "ok" } else { "error" },
                latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
                detail: Some(format!("pool {idle}/{size} idle")),
            });
        }
        None => dependencies.push(DependencyStatus {
            name: "database",
            status: "not_configured",
            latency_ms: 0.0,
            detail: Some("running without persistence".to_string()),
        }),
    }

    dependencies
}

/// `GET /metrics` — Prometheus exposition.
pub async fn metrics(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}

/// `GET /status` — the public status page feed (Phase 5).
///
/// Deliberately reveals nothing tenant-specific: provider circuit states and aggregate
/// latency only.
pub async fn public_status(State(state): State<AppState>) -> Response {
    let degraded = state.health.degraded_providers();

    Json(serde_json::json!({
        "status": if degraded.is_empty() { "operational" } else { "degraded" },
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "gateway_overhead_p50_ms": state.metrics.overhead_p50_ms(),
        "gateway_overhead_p99_ms": state.metrics.overhead_p99_ms(),
        "providers": degraded
            .into_iter()
            .map(|(provider, circuit)| serde_json::json!({
                "provider": provider,
                "state": circuit.as_str(),
            }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_reports_ok_with_a_working_store() {
        let state = AppState::for_tests();
        let response = health(State(state)).await;
        assert_eq!(response.status(), StatusCode::OK);

        let json = body_json(response).await;
        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
        assert!(json["uptime_seconds"].is_u64());
    }

    #[tokio::test]
    async fn health_lists_every_dependency() {
        let state = AppState::for_tests();
        let json = body_json(health(State(state)).await).await;

        let names: Vec<&str> = json["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"store"));
        assert!(names.contains(&"database"));
    }

    #[tokio::test]
    async fn an_unconfigured_database_is_reported_but_not_an_error() {
        // Running without persistence is a supported development mode, not a fault.
        let state = AppState::for_tests();
        let json = body_json(health(State(state)).await).await;

        let database = json["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == "database")
            .unwrap();
        assert_eq!(database["status"], "not_configured");
        assert_eq!(
            json["status"], "ok",
            "an absent database must not report degraded"
        );
    }

    #[tokio::test]
    async fn health_reports_per_dependency_latency() {
        let state = AppState::for_tests();
        let json = body_json(health(State(state)).await).await;
        for dependency in json["dependencies"].as_array().unwrap() {
            assert!(dependency["latency_ms"].is_number());
            assert!(dependency["latency_ms"].as_f64().unwrap() >= 0.0);
        }
    }

    #[tokio::test]
    async fn readiness_passes_when_the_store_is_up() {
        let state = AppState::for_tests();
        let response = ready(State(state)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["ready"], true);
    }

    #[tokio::test]
    async fn health_surfaces_degraded_providers() {
        let state = AppState::for_tests();
        for _ in 0..crate::engine::fallback::FAILURE_THRESHOLD {
            state.health.record_failure("openai");
        }

        let json = body_json(health(State(state)).await).await;
        let degraded = json["degraded_providers"].as_array().unwrap();
        assert_eq!(degraded.len(), 1);
        assert!(degraded[0].as_str().unwrap().starts_with("openai:"));
    }

    #[tokio::test]
    async fn metrics_render_in_prometheus_format() {
        let state = AppState::for_tests();
        state.metrics.record_request("/v1/chat/completions", 200);

        let response = metrics(State(state)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(content_type.starts_with("text/plain"));

        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("# TYPE aegis_requests_total counter"));
    }

    #[tokio::test]
    async fn the_public_status_page_leaks_no_tenant_data() {
        let state = AppState::for_tests();
        state.metrics.record_request("/v1/chat/completions", 200);
        state.metrics.record_overhead_ms(0.4);

        let json = body_json(public_status(State(state)).await).await;
        assert_eq!(json["status"], "operational");
        assert!(json["gateway_overhead_p99_ms"].is_number());

        // Nothing organisation-specific may appear on a public page.
        let rendered = json.to_string();
        assert!(!rendered.contains("org"));
        assert!(!rendered.contains("key"));
    }

    #[tokio::test]
    async fn the_status_page_reports_degradation() {
        let state = AppState::for_tests();
        for _ in 0..crate::engine::fallback::FAILURE_THRESHOLD {
            state.health.record_failure("groq");
        }
        let json = body_json(public_status(State(state)).await).await;
        assert_eq!(json["status"], "degraded");
        assert_eq!(json["providers"][0]["provider"], "groq");
        assert_eq!(json["providers"][0]["state"], "open");
    }
}

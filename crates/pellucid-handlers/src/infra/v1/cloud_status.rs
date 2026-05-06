//! `GET /api/infra/v1/cloud-status` — pure reader of the cloud
//! status-page snapshot.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "technology:cloud-status:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudStatusRow {
    pub provider: String,
    pub component: String,
    pub status: String,
    pub region: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudStatusResponse {
    pub rows: Vec<CloudStatusRow>,
    #[serde(rename = "incidentCount")]
    pub incident_count: usize,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<CloudStatusRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

fn is_incident(status: &str) -> bool {
    !matches!(
        status.to_ascii_lowercase().as_str(),
        "operational" | "ok" | "green" | "available"
    )
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<CloudStatusResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    let incident_count = snap.rows.iter().filter(|r| is_incident(&r.status)).count();
    Ok(Json(CloudStatusResponse {
        rows: snap.rows,
        incident_count,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::infra::v1::CLOUD_STATUS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            CLOUD_STATUS_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(CLOUD_STATUS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn counts_incidents() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "provider": "AWS", "component": "EC2", "status": "operational", "region": "us-east-1" },
                { "provider": "AWS", "component": "S3", "status": "degraded", "region": "us-east-1" },
                { "provider": "GCP", "component": "GCS", "status": "operational", "region": "us-central1" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(CLOUD_STATUS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/incidentCount").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(3));
    }

    #[test]
    fn is_incident_classifies() {
        assert!(!is_incident("operational"));
        assert!(!is_incident("OK"));
        assert!(is_incident("degraded"));
        assert!(is_incident("outage"));
    }
}

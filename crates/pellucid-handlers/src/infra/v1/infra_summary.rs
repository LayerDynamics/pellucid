//! `GET /api/infra/v1/summary` — composer over cloud-status +
//! grid-stress + cyber-incident counts. Powers InfraPanel.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{
    decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;

use super::{cloud_status, cyber_incidents, internet_outages};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InfraTile {
    pub code: String,
    pub label: String,
    pub value: String,
    /// `positive` | `negative` | `neutral`.
    pub tone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InfraSummaryResponse {
    pub tiles: Vec<InfraTile>,
    #[serde(rename = "availableTiles")]
    pub available_tiles: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct CloudSnap {
    #[serde(default)]
    rows: Vec<CloudRow>,
}

#[derive(Debug, Deserialize)]
struct CloudRow {
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct OutageSnap {
    #[serde(default)]
    rows: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct IncidentSnap {
    #[serde(default)]
    rows: Vec<IncidentRow>,
}

#[derive(Debug, Deserialize)]
struct IncidentRow {
    #[serde(default)]
    severity: String,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<InfraSummaryResponse>, HandlerError> {
    let raw_cloud = get_cached_json::<Value>(&state.pool, cloud_status::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_outage = get_cached_json::<Value>(&state.pool, internet_outages::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_inc = get_cached_json::<Value>(&state.pool, cyber_incidents::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (cloud, s1) = decode_optional::<CloudSnap>(raw_cloud)?;
    let (outage, s2) = decode_optional::<OutageSnap>(raw_outage)?;
    let (inc, s3) = decode_optional::<IncidentSnap>(raw_inc)?;

    let mut tiles: Vec<InfraTile> = Vec::new();
    if let Some(cloud) = cloud {
        let total = cloud.rows.len();
        let incidents = cloud
            .rows
            .iter()
            .filter(|r| !matches!(r.status.to_ascii_lowercase().as_str(), "operational" | "ok" | "green" | "available"))
            .count();
        tiles.push(InfraTile {
            code: "CLOUD".into(),
            label: "Cloud incidents".into(),
            value: format!("{incidents}/{total}"),
            tone: if incidents == 0 { "positive".into() } else if incidents <= 2 { "neutral".into() } else { "negative".into() },
        });
    }
    if let Some(outage) = outage {
        let n = outage.rows.len();
        tiles.push(InfraTile {
            code: "GRID".into(),
            label: "Network outages".into(),
            value: n.to_string(),
            tone: if n == 0 { "positive".into() } else { "negative".into() },
        });
    }
    if let Some(inc) = inc {
        let high = inc
            .rows
            .iter()
            .filter(|r| matches!(r.severity.to_ascii_lowercase().as_str(), "high" | "critical" | "extreme"))
            .count();
        tiles.push(InfraTile {
            code: "CYBER".into(),
            label: "High-severity cyber".into(),
            value: high.to_string(),
            tone: if high == 0 { "positive".into() } else if high <= 2 { "neutral".into() } else { "negative".into() },
        });
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let available = tiles.len();
    Ok(Json(InfraSummaryResponse {
        tiles,
        available_tiles: available,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: s1 || s2 || s3,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::infra::v1::SUMMARY_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(SUMMARY_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_all_empty() {
        let (app, _) = migrated().await;
        let resp = app.oneshot(Request::builder().uri(SUMMARY_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn picks_one_seeded() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "provider": "AWS", "component": "EC2", "status": "operational", "region": "us-east-1" },
                { "provider": "AWS", "component": "S3", "status": "degraded", "region": "us-east-1" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, cloud_status::CACHE_KEY, &Envelope::new(snap), 60_000).await.unwrap();
        let resp = app.oneshot(Request::builder().uri(SUMMARY_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/availableTiles").and_then(Value::as_u64), Some(1));
        assert_eq!(parsed.pointer("/tiles/0/code").and_then(Value::as_str), Some("CLOUD"));
        assert_eq!(parsed.pointer("/tiles/0/value").and_then(Value::as_str), Some("1/2"));
    }
}

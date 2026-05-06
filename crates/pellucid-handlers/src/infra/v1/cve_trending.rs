//! `GET /api/infra/v1/cve-trending` — pure reader for trending
//! CVEs from the cyber:cve-trending:v1 slot.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "cyber:cve-trending:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CveRow {
    #[serde(rename = "cveId", alias = "cve_id")]
    pub cve_id: String,
    pub summary: String,
    #[serde(rename = "cvssScore", alias = "cvss_score")]
    pub cvss_score: f64,
    pub severity: String,
    #[serde(rename = "publishedAtMs", alias = "published_at_ms")]
    pub published_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CveTrendingResponse {
    pub rows: Vec<CveRow>,
    #[serde(rename = "maxCvss")]
    pub max_cvss: f64,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<CveRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<CveTrendingResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    let max_cvss = snap
        .rows
        .iter()
        .map(|r| r.cvss_score)
        .fold(0.0_f64, f64::max);
    Ok(Json(CveTrendingResponse {
        rows: snap.rows,
        max_cvss,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::infra::v1::CVE_TRENDING_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            CVE_TRENDING_PATH,
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
                    .uri(CVE_TRENDING_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn computes_max_cvss() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "cve_id": "CVE-2026-1234", "summary": "x", "cvss_score": 9.8, "severity": "Critical", "published_at_ms": 1_700_000_000_000_i64 },
                { "cve_id": "CVE-2026-5678", "summary": "y", "cvss_score": 7.5, "severity": "High", "published_at_ms": 1_700_000_000_000_i64 },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(CVE_TRENDING_PATH)
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
            parsed.pointer("/maxCvss").and_then(Value::as_f64),
            Some(9.8)
        );
        assert_eq!(
            parsed.pointer("/rows/0/cveId").and_then(Value::as_str),
            Some("CVE-2026-1234")
        );
    }
}

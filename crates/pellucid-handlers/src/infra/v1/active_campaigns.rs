//! `GET /api/infra/v1/active-campaigns` — pure reader of the
//! security-advisory / active-campaign feed.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "cyber:active-campaigns:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CampaignRow {
    pub id: String,
    pub title: String,
    pub actor: String,
    pub severity: String,
    pub sectors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CampaignsResponse {
    pub rows: Vec<CampaignRow>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<CampaignRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<CampaignsResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    Ok(Json(CampaignsResponse {
        rows: snap.rows,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::infra::v1::ACTIVE_CAMPAIGNS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(ACTIVE_CAMPAIGNS_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app.oneshot(Request::builder().uri(ACTIVE_CAMPAIGNS_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_camelcase() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{ "id": "c1", "title": "APT29 phishing", "actor": "APT29", "severity": "high", "sectors": ["finance", "gov"] }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000).await.unwrap();
        let resp = app.oneshot(Request::builder().uri(ACTIVE_CAMPAIGNS_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/rows/0/actor").and_then(Value::as_str), Some("APT29"));
    }
}

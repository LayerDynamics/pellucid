//! `GET /api/military/v1/defense-patents` — pure reader for the
//! defense patent-trends snapshot (T4.5.7).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "defense:patent-trends:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatentClassRow {
    #[serde(rename = "cpcClass", alias = "cpc_class")]
    pub cpc_class: String,
    pub label: String,
    #[serde(rename = "filings30d", alias = "filings_30d")]
    pub filings_30d: u32,
    #[serde(rename = "yoyPct", alias = "yoy_pct")]
    pub yoy_pct: f64,
    #[serde(rename = "topFiler", alias = "top_filer")]
    pub top_filer: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DefensePatentsResponse {
    pub rows: Vec<PatentClassRow>,
    #[serde(rename = "totalFilings30d")]
    pub total_filings_30d: u32,
    pub period: String,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<PatentClassRow>,
    #[serde(default, alias = "totalFilings30d")]
    total_filings_30d: u32,
    #[serde(default)]
    period: String,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<DefensePatentsResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    Ok(Json(DefensePatentsResponse {
        rows: snap.rows,
        total_filings_30d: snap.total_filings_30d,
        period: snap.period,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::military::v1::DEFENSE_PATENTS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            DEFENSE_PATENTS_PATH,
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
                    .uri(DEFENSE_PATENTS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_camelcase() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{ "cpc_class": "B64G", "label": "B64G class", "filings_30d": 220, "yoy_pct": 41.0, "top_filer": "Lockheed Martin" }],
            "total_filings_30d": 220,
            "period": "2026-W18",
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(DEFENSE_PATENTS_PATH)
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
            parsed.pointer("/totalFilings30d").and_then(Value::as_u64),
            Some(220)
        );
        assert_eq!(
            parsed.pointer("/rows/0/cpcClass").and_then(Value::as_str),
            Some("B64G")
        );
    }
}

//! `GET /api/sanctions/v1/pressure` — pure reader for the
//! sanctions recent-additions slot, grouped by authority (T4.5.8).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;
use std::collections::BTreeMap;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "sanctions:recent-additions:24h:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SanctionRow {
    pub authority: String,
    pub entity: String,
    #[serde(rename = "entityType", alias = "entity_type")]
    pub entity_type: String,
    pub jurisdiction: String,
    #[serde(rename = "listedOn", alias = "listed_on")]
    pub listed_on: String,
    pub programme: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SanctionsPressureResponse {
    pub rows: Vec<SanctionRow>,
    #[serde(rename = "byAuthority")]
    pub by_authority: Vec<(String, u32)>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<SanctionRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

fn group_by_authority(rows: &[SanctionRow]) -> Vec<(String, u32)> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for r in rows {
        *counts.entry(r.authority.clone()).or_insert(0) += 1;
    }
    let mut out: Vec<(String, u32)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<SanctionsPressureResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    let by_authority = group_by_authority(&snap.rows);
    Ok(Json(SanctionsPressureResponse {
        rows: snap.rows,
        by_authority,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::sanctions::v1::PRESSURE_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(PRESSURE_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(PRESSURE_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn groups_by_authority() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "authority": "OFAC", "entity": "A", "entity_type": "company", "jurisdiction": "RU", "listed_on": "2026-04-29", "programme": "RUSSIA" },
                { "authority": "OFAC", "entity": "B", "entity_type": "vessel",  "jurisdiction": "IR", "listed_on": "2026-04-30", "programme": "IRAN"   },
                { "authority": "EU",   "entity": "C", "entity_type": "individual", "jurisdiction": "BY", "listed_on": "2026-04-29", "programme": "BELARUS" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(PRESSURE_PATH)
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
            parsed.pointer("/byAuthority/0/0").and_then(Value::as_str),
            Some("OFAC")
        );
        assert_eq!(
            parsed.pointer("/byAuthority/0/1").and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(3));
    }

    #[test]
    fn grouping_handles_empty() {
        assert!(group_by_authority(&[]).is_empty());
    }
}

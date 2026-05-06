//! `GET /api/supply-chain/v1/summary` — composer over supply-chain
//! stress-index + port-congestion + chokepoint-status (T4.5.9).
//! Tolerates per-slot absence: returns whatever is cached today.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

pub const STRESS_KEY: &str = "supply-chain:stress-index:current:v1";
pub const PORT_KEY: &str = "supply-chain:port-congestion:current:v1";
pub const CHOKEPOINT_KEY: &str = "maritime:chokepoint-status:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SupplyChainTile {
    pub code: String,
    pub label: String,
    pub value: String,
    /// `positive` | `negative` | `neutral`.
    pub tone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SupplyChainResponse {
    pub tiles: Vec<SupplyChainTile>,
    #[serde(rename = "availableTiles")]
    pub available_tiles: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct StressSnap {
    #[serde(default, alias = "indexValue", alias = "index_value")]
    index_value: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PortSnap {
    #[serde(default)]
    rows: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct ChokeSnap {
    #[serde(default)]
    rows: Vec<ChokeRow>,
}

#[derive(Debug, Deserialize)]
struct ChokeRow {
    #[serde(default)]
    status: String,
}

pub fn stress_tone(v: f64) -> &'static str {
    if v >= 0.6 {
        "negative"
    } else if v >= 0.3 {
        "neutral"
    } else {
        "positive"
    }
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<SupplyChainResponse>, HandlerError> {
    let raw_s = get_cached_json::<Value>(&state.pool, STRESS_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_p = get_cached_json::<Value>(&state.pool, PORT_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_c = get_cached_json::<Value>(&state.pool, CHOKEPOINT_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (stress, s1) = decode_optional::<StressSnap>(raw_s)?;
    let (port, s2) = decode_optional::<PortSnap>(raw_p)?;
    let (choke, s3) = decode_optional::<ChokeSnap>(raw_c)?;

    let mut tiles: Vec<SupplyChainTile> = Vec::new();
    if let Some(st) = stress {
        if let Some(v) = st.index_value {
            tiles.push(SupplyChainTile {
                code: "STRESS".into(),
                label: "Stress index".into(),
                value: format!("{v:.2}"),
                tone: stress_tone(v).into(),
            });
        }
    }
    if let Some(p) = port {
        let n = p.rows.len();
        tiles.push(SupplyChainTile {
            code: "PORTS".into(),
            label: "Congested ports".into(),
            value: n.to_string(),
            tone: if n == 0 {
                "positive".into()
            } else if n <= 3 {
                "neutral".into()
            } else {
                "negative".into()
            },
        });
    }
    if let Some(c) = choke {
        let degraded = c
            .rows
            .iter()
            .filter(|r| {
                !matches!(
                    r.status.to_ascii_lowercase().as_str(),
                    "open" | "normal" | "ok"
                )
            })
            .count();
        tiles.push(SupplyChainTile {
            code: "CHOKEPOINTS".into(),
            label: "Degraded chokepoints".into(),
            value: degraded.to_string(),
            tone: if degraded == 0 {
                "positive".into()
            } else {
                "negative".into()
            },
        });
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let available = tiles.len();
    Ok(Json(SupplyChainResponse {
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
    use crate::supply_chain::v1::SUMMARY_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(SUMMARY_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_all_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(SUMMARY_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn picks_stress_when_seeded() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({ "index_value": 0.72 });
        set_cached_json(&pool, STRESS_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(SUMMARY_PATH)
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
            parsed.pointer("/availableTiles").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            parsed.pointer("/tiles/0/code").and_then(Value::as_str),
            Some("STRESS")
        );
        assert_eq!(
            parsed.pointer("/tiles/0/tone").and_then(Value::as_str),
            Some("negative")
        );
    }

    #[test]
    fn stress_tone_bands() {
        assert_eq!(stress_tone(0.8), "negative");
        assert_eq!(stress_tone(0.4), "neutral");
        assert_eq!(stress_tone(0.1), "positive");
    }
}

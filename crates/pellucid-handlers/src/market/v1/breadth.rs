//! `GET /api/market/v1/breadth` handler.
//!
//! Pure cache reader. Reads the FAST-tier
//! `market:stocks-bootstrap:v1` snapshot and projects the basket
//! into a market-breadth envelope: advance/decline counts, the
//! advance-decline line, top advancers + decliners, and the
//! 52-week-style "new highs / new lows" approximation derived
//! from each row's percent change against
//! [`NEW_HIGH_THRESHOLD_PCT`] / [`NEW_LOW_THRESHOLD_PCT`].
//!
//! ## Tier
//!
//! Anonymous (tier 0). Breadth is a public surface.
//!
//! ## M4 outage path
//!
//! Same envelope as the rest of the family.

use std::cmp::Ordering;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::market::v1::list_market_quotes::CACHE_KEY;
use crate::state::AppState;

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Hard cap on `?topN` query — the panel only renders a handful
/// of advancers / decliners; a larger N just bloats the wire.
pub const MAX_TOP_N: usize = 25;

/// Default `?topN` when the caller doesn't ask.
pub const DEFAULT_TOP_N: usize = 5;

/// Symbols moving up by more than this percent count as a "new
/// high" in the snapshot's 1-day window. Mirrors the original
/// WorldMonitor breadth threshold.
pub const NEW_HIGH_THRESHOLD_PCT: f64 = 1.5;

/// Symbols moving down by more than this percent count as a
/// "new low".
pub const NEW_LOW_THRESHOLD_PCT: f64 = 1.5;

/// One symbol summary in the wire response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BreadthRow {
    /// Ticker.
    pub symbol: String,
    /// Percent change vs previous close.
    #[serde(rename = "percentChange")]
    pub percent_change: f64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BreadthResponse {
    /// Symbols whose `percent_change > 0`.
    pub advancers: usize,
    /// Symbols whose `percent_change < 0`.
    pub decliners: usize,
    /// Symbols with `percent_change == 0` (or absent / NaN).
    pub unchanged: usize,
    /// `advancers - decliners`. Positive readings indicate broad
    /// strength; negative readings indicate broad weakness.
    #[serde(rename = "advanceDeclineLine")]
    pub advance_decline_line: i64,
    /// Symbols whose `percent_change > NEW_HIGH_THRESHOLD_PCT`.
    #[serde(rename = "newHighs")]
    pub new_highs: usize,
    /// Symbols whose `percent_change < -NEW_LOW_THRESHOLD_PCT`.
    #[serde(rename = "newLows")]
    pub new_lows: usize,
    /// Top N advancers by percent change descending.
    #[serde(rename = "topAdvancers")]
    pub top_advancers: Vec<BreadthRow>,
    /// Top N decliners by percent change ascending.
    #[serde(rename = "topDecliners")]
    pub top_decliners: Vec<BreadthRow>,
    /// Universe size — total symbols in the snapshot.
    pub universe: usize,
    /// Whether the underlying snapshot was stale.
    pub stale: bool,
    /// Wall-clock ms from the snapshot's `assembledAtMs`.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct BreadthQuery {
    /// Cap on `topAdvancers` + `topDecliners` length. Defaults
    /// to [`DEFAULT_TOP_N`], clamped to [`MAX_TOP_N`].
    #[serde(default, rename = "topN")]
    pub top_n: Option<usize>,
}

/// Errors the handler can surface.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but the body did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// M4 outage path: empty cache slot.
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value emitted with the 503.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Cache(_) | Self::Shape(_) => StatusCode::BAD_GATEWAY,
            Self::Outage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl axum::response::IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        let mut body = serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        });
        if let Self::Outage { retry_after_secs } = &self {
            body["error"]["retry_after_secs"] = serde_json::Value::from(*retry_after_secs);
        }
        let status = self.status();
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static(self.code()),
        );
        if let Self::Outage { retry_after_secs } = &self {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

/// Cache payload shape — same reader the sibling handlers use,
/// expressed locally so we don't take a circular dep on the
/// sibling module's private structs.
#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<SeederQuoteRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SeederQuoteRow {
    symbol: String,
    #[serde(default)]
    percent_change: f64,
}

/// Pure projection — given the snapshot rows + a `top_n` cap,
/// build the breadth response. Exported for tests.
#[must_use]
pub fn compute_breadth(
    rows: Vec<BreadthRow>,
    stale: bool,
    assembled_at_ms: i64,
    top_n: usize,
) -> BreadthResponse {
    let universe = rows.len();
    let mut advancers = 0_usize;
    let mut decliners = 0_usize;
    let mut unchanged = 0_usize;
    let mut new_highs = 0_usize;
    let mut new_lows = 0_usize;
    for row in &rows {
        if !row.percent_change.is_finite() || row.percent_change == 0.0 {
            unchanged += 1;
            continue;
        }
        if row.percent_change > 0.0 {
            advancers += 1;
            if row.percent_change > NEW_HIGH_THRESHOLD_PCT {
                new_highs += 1;
            }
        } else {
            decliners += 1;
            if row.percent_change < -NEW_LOW_THRESHOLD_PCT {
                new_lows += 1;
            }
        }
    }
    let advance_decline_line = (advancers as i64) - (decliners as i64);

    let mut sorted = rows;
    sorted.sort_by(|a, b| {
        b.percent_change
            .partial_cmp(&a.percent_change)
            .unwrap_or(Ordering::Equal)
    });
    let cap = top_n.clamp(0, MAX_TOP_N);
    let top_advancers: Vec<BreadthRow> = sorted
        .iter()
        .filter(|r| r.percent_change > 0.0)
        .take(cap)
        .cloned()
        .collect();
    sorted.sort_by(|a, b| {
        a.percent_change
            .partial_cmp(&b.percent_change)
            .unwrap_or(Ordering::Equal)
    });
    let top_decliners: Vec<BreadthRow> = sorted
        .iter()
        .filter(|r| r.percent_change < 0.0)
        .take(cap)
        .cloned()
        .collect();

    BreadthResponse {
        advancers,
        decliners,
        unchanged,
        advance_decline_line,
        new_highs,
        new_lows,
        top_advancers,
        top_decliners,
        universe,
        stale,
        assembled_at_ms,
    }
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<BreadthQuery>,
) -> Result<Json<BreadthResponse>, HandlerError> {
    let raw: CacheHit<Value> = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => {
            return Err(HandlerError::Outage {
                retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
            });
        }
    };
    let inner = unwrap_envelope_data(value);
    let payload: SnapshotPayload =
        serde_json::from_value(inner).map_err(|e| HandlerError::Shape(e.to_string()))?;
    let rows: Vec<BreadthRow> = payload
        .rows
        .into_iter()
        .map(|r| BreadthRow {
            symbol: r.symbol,
            percent_change: r.percent_change,
        })
        .collect();
    let top_n = q.top_n.unwrap_or(DEFAULT_TOP_N).min(MAX_TOP_N);
    Ok(Json(compute_breadth(
        rows,
        stale,
        payload.assembled_at_ms,
        top_n,
    )))
}

/// Same envelope-unwrap helper used by every cache reader.
fn unwrap_envelope_data(v: Value) -> Value {
    if let Value::Object(map) = &v {
        if map.contains_key("_seed") {
            if let Some(inner) = map.get("data") {
                return inner.clone();
            }
        }
    }
    v
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    use crate::market::v1::BREADTH_PATH;

    fn row(symbol: &str, pct: f64) -> BreadthRow {
        BreadthRow {
            symbol: symbol.into(),
            percent_change: pct,
        }
    }

    fn snapshot(rows: Vec<(&str, f64)>) -> serde_json::Value {
        serde_json::json!({
            "rows": rows.into_iter().map(|(s, p)| serde_json::json!({
                "symbol": s,
                "price": 100.0,
                "previous_close": 100.0,
                "percent_change": p,
                "currency": "USD",
                "exchange": "NMS",
                "regular_market_time": 1_700_000_000,
            })).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            BREADTH_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn compute_breadth_counts_advancers_decliners_unchanged() {
        let rows = vec![
            row("A", 2.0),
            row("B", 0.5),
            row("C", -1.0),
            row("D", -3.0),
            row("E", 0.0),
        ];
        let resp = compute_breadth(rows, false, 0, DEFAULT_TOP_N);
        assert_eq!(resp.advancers, 2);
        assert_eq!(resp.decliners, 2);
        assert_eq!(resp.unchanged, 1);
        assert_eq!(resp.advance_decline_line, 0);
        assert_eq!(resp.universe, 5);
    }

    #[test]
    fn compute_breadth_counts_new_highs_and_lows_against_thresholds() {
        let rows = vec![
            row("A", 2.0),
            row("B", 1.5),
            row("C", 1.0),
            row("D", -2.0),
            row("E", -1.5),
            row("F", -1.0),
        ];
        let resp = compute_breadth(rows, false, 0, DEFAULT_TOP_N);
        // 2.0 > 1.5 → new high. 1.5 == 1.5 → NOT new high (strict >).
        assert_eq!(resp.new_highs, 1);
        assert_eq!(resp.new_lows, 1);
    }

    #[test]
    fn compute_breadth_treats_nan_as_unchanged() {
        let rows = vec![row("A", f64::NAN), row("B", 1.0)];
        let resp = compute_breadth(rows, false, 0, DEFAULT_TOP_N);
        assert_eq!(resp.advancers, 1);
        assert_eq!(resp.unchanged, 1);
    }

    #[test]
    fn compute_breadth_top_advancers_sorted_descending() {
        let rows = vec![
            row("LOW", 0.1),
            row("MID", 1.0),
            row("HIGH", 5.0),
        ];
        let resp = compute_breadth(rows, false, 0, /*top_n=*/ 2);
        assert_eq!(resp.top_advancers.len(), 2);
        assert_eq!(resp.top_advancers[0].symbol, "HIGH");
        assert_eq!(resp.top_advancers[1].symbol, "MID");
    }

    #[test]
    fn compute_breadth_top_decliners_sorted_ascending() {
        let rows = vec![
            row("LOSE", -5.0),
            row("MID", -1.0),
            row("OK", 0.5),
        ];
        let resp = compute_breadth(rows, false, 0, /*top_n=*/ 5);
        assert_eq!(resp.top_decliners.len(), 2);
        assert_eq!(resp.top_decliners[0].symbol, "LOSE");
        assert_eq!(resp.top_decliners[1].symbol, "MID");
    }

    #[test]
    fn compute_breadth_clamps_top_n_to_max() {
        let rows: Vec<BreadthRow> = (0..(MAX_TOP_N + 5))
            .map(|i| row(&format!("A{i}"), (i as f64) * 0.1))
            .collect();
        let resp = compute_breadth(rows, false, 0, MAX_TOP_N * 10);
        assert!(resp.top_advancers.len() <= MAX_TOP_N);
    }

    #[test]
    fn handler_error_status_codes() {
        assert_eq!(
            HandlerError::Cache("x".into()).status(),
            Code::BAD_GATEWAY,
        );
        assert_eq!(
            HandlerError::Shape("x".into()).status(),
            Code::BAD_GATEWAY,
        );
        assert_eq!(
            HandlerError::Outage { retry_after_secs: 0 }.status(),
            Code::SERVICE_UNAVAILABLE,
        );
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BREADTH_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_envelope_for_populated_cache() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            ("WIN", 2.5),
            ("MID", 0.3),
            ("FLAT", 0.0),
            ("LOSE", -2.5),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BREADTH_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: BreadthResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.advancers, 2);
        assert_eq!(parsed.decliners, 1);
        assert_eq!(parsed.unchanged, 1);
        assert_eq!(parsed.advance_decline_line, 1);
        assert_eq!(parsed.new_highs, 1);
        assert_eq!(parsed.new_lows, 1);
        assert!(!parsed.stale);
        assert_eq!(parsed.assembled_at_ms, 1_700_000_000_000);
    }

    #[tokio::test]
    async fn handler_clamps_top_n_via_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            ("A", 1.0),
            ("B", 2.0),
            ("C", 3.0),
            ("D", 4.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{}?topN=2", BREADTH_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: BreadthResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.top_advancers.len(), 2);
        assert_eq!(parsed.top_advancers[0].symbol, "D");
    }
}

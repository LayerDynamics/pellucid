//! `GET /api/market/v1/analyze-stock` — tier-2 stock analysis.
//!
//! Reads the same FAST-tier `market:stocks-bootstrap:v1` cache
//! slot the seeder populates and projects one row's worth of
//! quote data into a richer analytics payload. The handler is
//! gated to **tier 2** at the gateway layer — see
//! [`REQUIRED_TIER`] for the constant the binary's
//! `RouteEntitlementRules::require()` call uses.
//!
//! ## Why not a live upstream call
//!
//! Per SPEC-001 §24's H3 fix, edge handlers MUST NEVER call
//! relay-side upstreams synchronously — that path collapses
//! latency budgets under cold-cache load. The relay's
//! `seed_market_quotes` already publishes the per-symbol
//! snapshot every 60 s; analytics over the same data is
//! arithmetic + classification, not new I/O.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "symbol": "SPY",
//!   "price": 524.0,
//!   "previousClose": 522.0,
//!   "currency": "USD",
//!   "exchange": "PCX",
//!   "regularMarketTimeMs": 1714060800000,
//!   "metrics": {
//!     "dollarChange": 2.0,
//!     "percentChange": 0.383,
//!     "trend": "up",
//!     "magnitude": "small",
//!     "rangePosition": 0.5
//!   },
//!   "assembledAtMs": 1746360000000,
//!   "stale": false
//! }
//! ```

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;
use pellucid_gateway::traits::Tier;

use crate::market::v1::list_market_quotes::CACHE_KEY;
use crate::state::AppState;

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Tier required to call this route. The binary's
/// `RouteEntitlementRules::require()` call wires this to the
/// `/api/market/v1/analyze-stock` path; the handler does NOT
/// re-check (the gateway middleware enforces it before the
/// handler runs).
pub const REQUIRED_TIER: Tier = Tier::Tier2;

/// Threshold that splits `flat` from `up`/`down` trends, in
/// percent. Anything inside `[-FLAT_BAND_PCT, +FLAT_BAND_PCT]`
/// is `flat`.
pub const FLAT_BAND_PCT: f64 = 0.10;

/// Threshold (percent) that splits `small` from `medium`.
pub const SMALL_MAGNITUDE_PCT: f64 = 0.50;

/// Threshold (percent) that splits `medium` from `large`.
pub const LARGE_MAGNITUDE_PCT: f64 = 2.00;

/// Trend classification.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Trend {
    /// `percent_change > +FLAT_BAND_PCT`.
    Up,
    /// `percent_change < -FLAT_BAND_PCT`.
    Down,
    /// `|percent_change| <= FLAT_BAND_PCT`.
    Flat,
}

/// Magnitude classification (independent of direction).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Magnitude {
    /// `|pct| < SMALL_MAGNITUDE_PCT`.
    Small,
    /// `SMALL_MAGNITUDE_PCT <= |pct| < LARGE_MAGNITUDE_PCT`.
    Medium,
    /// `|pct| >= LARGE_MAGNITUDE_PCT`.
    Large,
}

/// Derived metrics. Pure projection of the cached quote.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Metrics {
    /// `price - previousClose` in the quote currency.
    #[serde(rename = "dollarChange")]
    pub dollar_change: f64,
    /// `(price - previousClose) / previousClose * 100`.
    #[serde(rename = "percentChange")]
    pub percent_change: f64,
    /// Direction band (up/down/flat).
    pub trend: Trend,
    /// Absolute size band (small/medium/large).
    pub magnitude: Magnitude,
    /// Position of `price` inside the basket's `[min, max]`
    /// price range, normalised to `[0.0, 1.0]`. Useful for the
    /// panel's "where does this symbol sit relative to its
    /// peers" affordance. `0.5` when the basket has only one
    /// row (no spread).
    #[serde(rename = "rangePosition")]
    pub range_position: f64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyzeStockResponse {
    /// Echo of the requested symbol (canonicalised — uppercased).
    pub symbol: String,
    /// Last traded price.
    pub price: f64,
    /// Previous-session close.
    #[serde(rename = "previousClose")]
    pub previous_close: f64,
    /// Currency the price is denominated in.
    pub currency: String,
    /// Exchange code.
    pub exchange: String,
    /// Wall-clock ms when the upstream stamped the quote.
    #[serde(rename = "regularMarketTimeMs")]
    pub regular_market_time_ms: i64,
    /// Derived analytics.
    pub metrics: Metrics,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// Whether the response was synthesised from a stale cache row.
    pub stale: bool,
}

/// Required + optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct AnalyzeStockQuery {
    /// Ticker symbol. Required — empty 400s as `invalid_request`.
    pub symbol: Option<String>,
}

/// Internal — snapshot row shape stored by the seeder.
#[derive(Debug, Deserialize, Clone)]
struct SeederQuoteRow {
    symbol: String,
    price: f64,
    previous_close: f64,
    #[allow(dead_code)]
    percent_change: f64,
    currency: String,
    exchange: String,
    regular_market_time: i64,
}

#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<SeederQuoteRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Missing or empty `?symbol=`.
    #[error("symbol query parameter is required")]
    MissingSymbol,
    /// Symbol not present in the cached basket.
    #[error("symbol not in basket: {0}")]
    UnknownSymbol(String),
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// M4 outage path — cache empty.
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    /// Stable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingSymbol => "invalid_request",
            Self::UnknownSymbol(_) => "symbol_not_found",
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::MissingSymbol => StatusCode::BAD_REQUEST,
            Self::UnknownSymbol(_) => StatusCode::NOT_FOUND,
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

/// Compute trend classification from a percent change. Pure.
#[must_use]
pub fn classify_trend(percent_change: f64) -> Trend {
    if percent_change > FLAT_BAND_PCT {
        Trend::Up
    } else if percent_change < -FLAT_BAND_PCT {
        Trend::Down
    } else {
        Trend::Flat
    }
}

/// Compute magnitude classification from a percent change. Pure.
#[must_use]
pub fn classify_magnitude(percent_change: f64) -> Magnitude {
    let abs = percent_change.abs();
    if abs >= LARGE_MAGNITUDE_PCT {
        Magnitude::Large
    } else if abs >= SMALL_MAGNITUDE_PCT {
        Magnitude::Medium
    } else {
        Magnitude::Small
    }
}

/// Compute range position for `price` against the basket's
/// `[min, max]` price spread. Returns `0.5` when the spread is
/// zero (single-row basket or all-equal prices). Pure.
#[must_use]
pub fn range_position(price: f64, basket_prices: &[f64]) -> f64 {
    if basket_prices.is_empty() {
        return 0.5;
    }
    let mut min = basket_prices[0];
    let mut max = basket_prices[0];
    for &p in basket_prices.iter().skip(1) {
        if p < min {
            min = p;
        }
        if p > max {
            max = p;
        }
    }
    let spread = max - min;
    if spread <= 0.0 {
        return 0.5;
    }
    ((price - min) / spread).clamp(0.0, 1.0)
}

/// Project one cached quote row into the analytics response.
/// Pure — exported for unit tests.
#[must_use]
pub fn project(
    row: SeederQuoteRowOwned,
    basket_prices: &[f64],
    assembled_at_ms: i64,
    stale: bool,
) -> AnalyzeStockResponse {
    let dollar_change = row.price - row.previous_close;
    let percent_change = if row.previous_close == 0.0 {
        0.0
    } else {
        (row.price - row.previous_close) / row.previous_close * 100.0
    };
    AnalyzeStockResponse {
        symbol: row.symbol,
        price: row.price,
        previous_close: row.previous_close,
        currency: row.currency,
        exchange: row.exchange,
        regular_market_time_ms: row
            .regular_market_time
            .checked_mul(1_000)
            .unwrap_or(row.regular_market_time),
        metrics: Metrics {
            dollar_change,
            percent_change,
            trend: classify_trend(percent_change),
            magnitude: classify_magnitude(percent_change),
            range_position: range_position(row.price, basket_prices),
        },
        assembled_at_ms,
        stale,
    }
}

/// Public mirror of the internal seeder row — exposed so tests
/// can build inputs without re-typing the snake_case fields.
#[derive(Clone, Debug)]
pub struct SeederQuoteRowOwned {
    /// Ticker.
    pub symbol: String,
    /// Most-recent price.
    pub price: f64,
    /// Previous close.
    pub previous_close: f64,
    /// Currency.
    pub currency: String,
    /// Exchange.
    pub exchange: String,
    /// Wall-clock seconds.
    pub regular_market_time: i64,
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<AnalyzeStockQuery>,
) -> Result<Json<AnalyzeStockResponse>, HandlerError> {
    let raw_symbol = q.symbol.unwrap_or_default();
    let canonical = raw_symbol.trim().to_ascii_uppercase();
    if canonical.is_empty() {
        return Err(HandlerError::MissingSymbol);
    }

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

    let basket_prices: Vec<f64> = payload.rows.iter().map(|r| r.price).collect();
    let row = payload
        .rows
        .iter()
        .find(|r| r.symbol.eq_ignore_ascii_case(&canonical))
        .cloned()
        .ok_or_else(|| HandlerError::UnknownSymbol(canonical.clone()))?;

    Ok(Json(project(
        SeederQuoteRowOwned {
            symbol: row.symbol,
            price: row.price,
            previous_close: row.previous_close,
            currency: row.currency,
            exchange: row.exchange,
            regular_market_time: row.regular_market_time,
        },
        &basket_prices,
        payload.assembled_at_ms,
        stale,
    )))
}

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
    use crate::market::v1::ANALYZE_STOCK_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn snapshot_value(rows: &[(&str, f64, f64)]) -> Value {
        serde_json::json!({
            "rows": rows.iter().map(|(s, p, prev)| {
                let pct = if *prev == 0.0 { 0.0 } else { (p - prev) / prev * 100.0 };
                serde_json::json!({
                    "symbol": s,
                    "price": p,
                    "previous_close": prev,
                    "percent_change": pct,
                    "currency": "USD",
                    "exchange": "PCX",
                    "regular_market_time": 1_714_060_800_i64,
                })
            }).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            ANALYZE_STOCK_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn required_tier_is_tier_2() {
        assert_eq!(REQUIRED_TIER, Tier::Tier2);
    }

    #[test]
    fn classify_trend_uses_flat_band() {
        assert_eq!(classify_trend(0.0), Trend::Flat);
        assert_eq!(classify_trend(FLAT_BAND_PCT), Trend::Flat);
        assert_eq!(classify_trend(-FLAT_BAND_PCT), Trend::Flat);
        assert_eq!(classify_trend(FLAT_BAND_PCT + 0.001), Trend::Up);
        assert_eq!(classify_trend(-FLAT_BAND_PCT - 0.001), Trend::Down);
    }

    #[test]
    fn classify_magnitude_thresholds() {
        assert_eq!(classify_magnitude(0.0), Magnitude::Small);
        assert_eq!(classify_magnitude(0.49), Magnitude::Small);
        assert_eq!(classify_magnitude(0.50), Magnitude::Medium);
        assert_eq!(classify_magnitude(-1.99), Magnitude::Medium);
        assert_eq!(classify_magnitude(2.00), Magnitude::Large);
        assert_eq!(classify_magnitude(-5.0), Magnitude::Large);
    }

    #[test]
    fn range_position_handles_basket_edges() {
        let basket = &[100.0_f64, 200.0, 300.0];
        assert!((range_position(100.0, basket) - 0.0).abs() < 1e-9);
        assert!((range_position(300.0, basket) - 1.0).abs() < 1e-9);
        assert!((range_position(200.0, basket) - 0.5).abs() < 1e-9);
        // out-of-range inputs are clamped.
        assert!((range_position(0.0, basket) - 0.0).abs() < 1e-9);
        assert!((range_position(1_000.0, basket) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn range_position_zero_spread_returns_half() {
        assert!((range_position(100.0, &[100.0_f64, 100.0]) - 0.5).abs() < 1e-9);
        assert!((range_position(100.0, &[]) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn project_computes_dollar_change_and_metrics() {
        let row = SeederQuoteRowOwned {
            symbol: "SPY".into(),
            price: 524.0,
            previous_close: 522.0,
            currency: "USD".into(),
            exchange: "PCX".into(),
            regular_market_time: 1_714_060_800,
        };
        let resp = project(row, &[522.0, 524.0, 526.0], 1, false);
        assert!((resp.metrics.dollar_change - 2.0).abs() < 1e-9);
        assert!((resp.metrics.percent_change - 0.383_141_762).abs() < 1e-6);
        assert_eq!(resp.metrics.trend, Trend::Up);
        assert_eq!(resp.metrics.magnitude, Magnitude::Small);
        // Range position: 524 inside [522, 526] → 0.5.
        assert!((resp.metrics.range_position - 0.5).abs() < 1e-9);
        assert_eq!(resp.regular_market_time_ms, 1_714_060_800_000);
    }

    #[test]
    fn project_handles_zero_previous_close_without_div_by_zero() {
        let row = SeederQuoteRowOwned {
            symbol: "NEW".into(),
            price: 100.0,
            previous_close: 0.0,
            currency: "USD".into(),
            exchange: "PCX".into(),
            regular_market_time: 0,
        };
        let resp = project(row, &[100.0], 1, false);
        assert_eq!(resp.metrics.percent_change, 0.0);
        assert_eq!(resp.metrics.trend, Trend::Flat);
    }

    #[tokio::test]
    async fn handler_returns_400_when_symbol_missing() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(ANALYZE_STOCK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/error/code").and_then(Value::as_str),
            Some("invalid_request"),
        );
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{ANALYZE_STOCK_PATH}?symbol=SPY"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_404_when_symbol_not_in_basket() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[("SPY", 524.0, 522.0)]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{ANALYZE_STOCK_PATH}?symbol=NVDA"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/error/code").and_then(Value::as_str),
            Some("symbol_not_found"),
        );
    }

    #[tokio::test]
    async fn handler_returns_full_analytics_payload() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[
            ("SPY", 524.0, 522.0),
            ("QQQ", 460.0, 458.0),
            ("DIA", 388.0, 392.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{ANALYZE_STOCK_PATH}?symbol=spy"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: AnalyzeStockResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.symbol, "SPY");
        assert_eq!(parsed.price, 524.0);
        assert_eq!(parsed.previous_close, 522.0);
        assert_eq!(parsed.metrics.trend, Trend::Up);
        assert_eq!(parsed.metrics.magnitude, Magnitude::Small);
        assert_eq!(parsed.assembled_at_ms, 1_700_000_000_000);
        assert!(!parsed.stale);
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[("SPY", 524.0, 522.0)]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{ANALYZE_STOCK_PATH}?symbol=SPY"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: AnalyzeStockResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "rows": "not-an-array" }));
        set_cached_json(&pool, CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{ANALYZE_STOCK_PATH}?symbol=SPY"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

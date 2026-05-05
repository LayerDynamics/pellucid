//! `GET /api/market/v1/list-market-quotes` handler.
//!
//! Pure cache reader for the snapshot
//! `seed_market_quotes` writes to `market:stocks-bootstrap:v1`
//! (FAST_KEYS slot). The webview's `MarketPanel` (T4.2.1)
//! consumes the response.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "rows": [{
//!     "symbol": "SPY",
//!     "price": 524.0,
//!     "previousClose": 522.0,
//!     "percentChange": 0.383,
//!     "currency": "USD",
//!     "exchange": "PCX",
//!     "regularMarketTimeMs": 1714060800000
//!   }],
//!   "assembledAtMs": 1746360000000,
//!   "total": 8,
//!   "stale": false
//! }
//! ```
//!
//! ## M4 outage path
//!
//! Same envelope as the news + intel handlers: 503 +
//! `Retry-After` + `bootstrap_upstream_empty`.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — pinned to the seeder slot.
pub const CACHE_KEY: &str = "market:stocks-bootstrap:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Hard cap on `?limit=`.
pub const MAX_LIMIT: usize = 100;

/// Default `?limit` when the client doesn't ask. The seeder's
/// default basket is 8 symbols; the cap leaves headroom for a
/// larger custom basket without ballooning the wire bytes.
pub const DEFAULT_LIMIT: usize = 50;

/// One quote row in the wire response. Field renames give the
/// wire camelCase while the deserializer accepts the seeder's
/// snake_case via `#[serde(alias = "…")]`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MarketQuote {
    /// Ticker symbol.
    pub symbol: String,
    /// Most-recent regular-session price.
    pub price: f64,
    /// Previous-session close.
    #[serde(rename = "previousClose", alias = "previous_close")]
    pub previous_close: f64,
    /// Percent change vs `previousClose`.
    #[serde(rename = "percentChange", alias = "percent_change")]
    pub percent_change: f64,
    /// Currency the price is quoted in.
    pub currency: String,
    /// Exchange code.
    pub exchange: String,
    /// Wall-clock ms when the upstream stamped this row.
    /// Wire field is ms even though the seeder stores seconds —
    /// the handler converts to keep all wire timestamps in the
    /// same unit across the v1 surface.
    #[serde(rename = "regularMarketTimeMs")]
    pub regular_market_time_ms: i64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListMarketQuotesResponse {
    /// Quote rows — clamped to `min(?limit, MAX_LIMIT)`.
    pub rows: Vec<MarketQuote>,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// Total quotes in the cache slot before the limit clamp.
    pub total: usize,
    /// Whether the response was synthesised from a stale row.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct ListMarketQuotesQuery {
    /// Cap on rows. Defaults to [`DEFAULT_LIMIT`], clamped to
    /// [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Comma-separated allow-list of ticker symbols. When set,
    /// only matching rows are returned (case-insensitive).
    /// Useful for the panel's "favourites" view that subsets the
    /// full basket.
    #[serde(default)]
    pub symbols: Option<String>,
}

/// Internal — snapshot shape stored by the seeder.
#[derive(Debug, Deserialize)]
struct SeederQuoteRow {
    symbol: String,
    price: f64,
    previous_close: f64,
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
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// M4 outage path.
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
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status.
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

/// Apply `?limit` + `?symbols` filters. Pure — exported so unit
/// tests pin every boundary without touching the cache.
#[must_use]
pub fn apply_filters(
    rows: Vec<MarketQuote>,
    q: &ListMarketQuotesQuery,
) -> (Vec<MarketQuote>, usize) {
    let mut filtered: Vec<MarketQuote> = if let Some(raw) = q.symbols.as_deref() {
        let allow: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if allow.is_empty() {
            rows
        } else {
            rows.into_iter()
                .filter(|r| allow.iter().any(|a| a == &r.symbol.to_ascii_uppercase()))
                .collect()
        }
    } else {
        rows
    };
    let total = filtered.len();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    if filtered.len() > limit {
        filtered.truncate(limit);
    }
    (filtered, total)
}

/// Project a seeder row to the wire shape (camelCase + ms-unit
/// timestamp). Pure — exported for tests.
#[must_use]
pub fn project_row(r: SeederQuoteRowOwned) -> MarketQuote {
    MarketQuote {
        symbol: r.symbol,
        price: r.price,
        previous_close: r.previous_close,
        percent_change: r.percent_change,
        currency: r.currency,
        exchange: r.exchange,
        regular_market_time_ms: r
            .regular_market_time
            .checked_mul(1_000)
            .unwrap_or(r.regular_market_time),
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
    /// Percent change vs previous close.
    pub percent_change: f64,
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
    Query(q): Query<ListMarketQuotesQuery>,
) -> Result<Json<ListMarketQuotesResponse>, HandlerError> {
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
    let payload: SnapshotPayload = serde_json::from_value(inner)
        .map_err(|e| HandlerError::Shape(e.to_string()))?;
    let projected: Vec<MarketQuote> = payload
        .rows
        .into_iter()
        .map(|r| {
            project_row(SeederQuoteRowOwned {
                symbol: r.symbol,
                price: r.price,
                previous_close: r.previous_close,
                percent_change: r.percent_change,
                currency: r.currency,
                exchange: r.exchange,
                regular_market_time: r.regular_market_time,
            })
        })
        .collect();
    let (rows, total) = apply_filters(projected, &q);
    Ok(Json(ListMarketQuotesResponse {
        rows,
        assembled_at_ms: payload.assembled_at_ms,
        total,
        stale,
    }))
}

/// Same envelope-unwrap helper used by every other handler.
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
    use crate::market::v1::LIST_MARKET_QUOTES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn quote(symbol: &str, price: f64, prev: f64) -> MarketQuote {
        let pct = if prev == 0.0 {
            0.0
        } else {
            (price - prev) / prev * 100.0
        };
        MarketQuote {
            symbol: symbol.into(),
            price,
            previous_close: prev,
            percent_change: pct,
            currency: "USD".into(),
            exchange: "PCX".into(),
            regular_market_time_ms: 1_714_060_800_000,
        }
    }

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
            LIST_MARKET_QUOTES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn cache_key_pinned_to_seeder_slot() {
        assert_eq!(CACHE_KEY, "market:stocks-bootstrap:v1");
    }

    #[test]
    fn project_row_converts_seconds_to_ms() {
        let r = SeederQuoteRowOwned {
            symbol: "SPY".into(),
            price: 524.0,
            previous_close: 522.0,
            percent_change: 0.383,
            currency: "USD".into(),
            exchange: "PCX".into(),
            regular_market_time: 1_714_060_800,
        };
        let q = project_row(r);
        assert_eq!(q.regular_market_time_ms, 1_714_060_800_000);
    }

    #[test]
    fn apply_filters_default_limit_is_fifty() {
        let rows: Vec<MarketQuote> = (0..120)
            .map(|i| quote(&format!("S{i}"), i as f64, (i as f64) - 1.0))
            .collect();
        let (out, total) = apply_filters(rows, &ListMarketQuotesQuery::default());
        assert_eq!(out.len(), DEFAULT_LIMIT);
        assert_eq!(total, 120);
    }

    #[test]
    fn apply_filters_clamps_limit_to_max() {
        let rows: Vec<MarketQuote> = (0..(MAX_LIMIT + 30))
            .map(|i| quote(&format!("S{i}"), i as f64, (i as f64) - 1.0))
            .collect();
        let q = ListMarketQuotesQuery {
            limit: Some(MAX_LIMIT * 5),
            symbols: None,
        };
        let (out, total) = apply_filters(rows, &q);
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 30);
    }

    #[test]
    fn apply_filters_zero_limit_floors_to_one() {
        let rows = vec![quote("A", 1.0, 1.0), quote("B", 1.0, 1.0)];
        let q = ListMarketQuotesQuery {
            limit: Some(0),
            symbols: None,
        };
        let (out, _) = apply_filters(rows, &q);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn apply_filters_symbols_allowlist_is_case_insensitive_csv() {
        let rows = vec![
            quote("SPY", 1.0, 1.0),
            quote("QQQ", 1.0, 1.0),
            quote("DIA", 1.0, 1.0),
        ];
        let q = ListMarketQuotesQuery {
            limit: None,
            symbols: Some("spy, dia".into()),
        };
        let (out, total) = apply_filters(rows, &q);
        let syms: Vec<&str> = out.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(syms, vec!["SPY", "DIA"]);
        assert_eq!(total, 2);
    }

    #[test]
    fn apply_filters_empty_symbols_string_passes_everything_through() {
        let rows = vec![quote("SPY", 1.0, 1.0), quote("QQQ", 1.0, 1.0)];
        let q = ListMarketQuotesQuery {
            limit: None,
            symbols: Some("   ,, ".into()),
        };
        let (out, _) = apply_filters(rows, &q);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn handler_error_codes_match_other_handlers() {
        assert_eq!(HandlerError::Cache("x".into()).code(), "cache_failure");
        assert_eq!(HandlerError::Shape("x".into()).code(), "cache_shape");
        assert_eq!(
            HandlerError::Outage { retry_after_secs: 30 }.code(),
            "bootstrap_upstream_empty",
        );
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_MARKET_QUOTES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_returns_rows_with_camelcase_field_names() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[("SPY", 524.0, 522.0)]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_MARKET_QUOTES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/rows/0/previousClose").is_some());
        assert!(parsed.pointer("/rows/0/percentChange").is_some());
        assert!(parsed.pointer("/rows/0/regularMarketTimeMs").is_some());
        assert_eq!(
            parsed.pointer("/assembledAtMs").and_then(Value::as_i64),
            Some(1_700_000_000_000),
        );
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(1));
    }

    #[tokio::test]
    async fn handler_filters_by_symbols_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[
            ("SPY", 524.0, 522.0),
            ("QQQ", 460.0, 458.0),
            ("DIA", 390.0, 389.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{LIST_MARKET_QUOTES_PATH}?symbols=spy,dia"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: ListMarketQuotesResponse = serde_json::from_slice(&body).unwrap();
        let syms: Vec<&str> = parsed.rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(syms, vec!["SPY", "DIA"]);
        assert_eq!(parsed.total, 2);
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
                    .uri(LIST_MARKET_QUOTES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
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
                    .uri(LIST_MARKET_QUOTES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: ListMarketQuotesResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_clamps_limit_via_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[
            ("SPY", 1.0, 1.0),
            ("QQQ", 1.0, 1.0),
            ("DIA", 1.0, 1.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{LIST_MARKET_QUOTES_PATH}?limit=2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: ListMarketQuotesResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.total, 3);
    }
}

//! `GET /api/market/v1/earnings` handler.
//!
//! Pure cache reader of the SLOW-tier
//! `market:earnings-calendar:7d:v1` snapshot the
//! `seed_earnings_calendar` seeder writes. Anonymous tier —
//! upcoming earnings is a public surface.

use std::collections::BTreeMap;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — pinned to the seeder.
pub const CACHE_KEY: &str = "market:earnings-calendar:7d:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Hard cap on `?limit=`.
pub const MAX_LIMIT: usize = 200;

/// Default `?limit=` when the client doesn't ask.
pub const DEFAULT_LIMIT: usize = 100;

/// Earnings event timing — kebab-case wire mirroring the seeder.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EarningsTiming {
    /// Before the open.
    BeforeOpen,
    /// After the close.
    AfterClose,
    /// During hours.
    DuringHours,
    /// Unknown.
    Unknown,
}

/// One event row in the wire response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EarningsEvent {
    /// Ticker symbol.
    pub symbol: String,
    /// Company name.
    pub company: String,
    /// Earnings date (`YYYY-MM-DD`).
    pub date: String,
    /// Timing.
    pub timing: EarningsTiming,
    /// EPS estimate.
    #[serde(
        rename = "epsEstimate",
        alias = "eps_estimate",
        skip_serializing_if = "Option::is_none"
    )]
    pub eps_estimate: Option<f64>,
    /// Prior-quarter EPS actual.
    #[serde(
        rename = "epsActualPrior",
        alias = "eps_actual_prior",
        skip_serializing_if = "Option::is_none"
    )]
    pub eps_actual_prior: Option<f64>,
}

/// Per-day grouping of events.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EarningsDayGroup {
    /// Date (`YYYY-MM-DD`).
    pub date: String,
    /// Events on this day, sorted ascending by symbol.
    pub events: Vec<EarningsEvent>,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EarningsResponse {
    /// Day groupings — sorted ascending by date.
    pub days: Vec<EarningsDayGroup>,
    /// Total events before the limit clamp.
    pub total: usize,
    /// Echo of the seeder's lookahead window.
    #[serde(rename = "lookaheadDays")]
    pub lookahead_days: u32,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when the response was synthesised from a stale row.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct EarningsQuery {
    /// Cap on events. Defaults to [`DEFAULT_LIMIT`], clamped to
    /// [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Comma-separated allow-list of ticker symbols.
    #[serde(default)]
    pub symbols: Option<String>,
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

/// Internal — minimal seeder shape.
#[derive(Debug, Deserialize)]
struct SeederEvent {
    symbol: String,
    company: String,
    date: String,
    timing: EarningsTiming,
    #[serde(default)]
    eps_estimate: Option<f64>,
    #[serde(default)]
    eps_actual_prior: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    events: Vec<SeederEvent>,
    #[serde(default)]
    lookahead_days: u32,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Filter + group the events into daily buckets. Pure.
#[must_use]
pub fn group_by_day(
    events: Vec<EarningsEvent>,
    symbols: Option<&str>,
    limit: usize,
) -> (Vec<EarningsDayGroup>, usize) {
    let filtered: Vec<EarningsEvent> = if let Some(raw) = symbols {
        let allow: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if allow.is_empty() {
            events
        } else {
            events
                .into_iter()
                .filter(|e| allow.iter().any(|a| a == &e.symbol.to_ascii_uppercase()))
                .collect()
        }
    } else {
        events
    };
    let total = filtered.len();
    let cap = limit.clamp(1, MAX_LIMIT);
    let truncated: Vec<EarningsEvent> = filtered.into_iter().take(cap).collect();

    let mut by_day: BTreeMap<String, Vec<EarningsEvent>> = BTreeMap::new();
    for e in truncated {
        by_day.entry(e.date.clone()).or_default().push(e);
    }
    let mut days: Vec<EarningsDayGroup> = by_day
        .into_iter()
        .map(|(date, mut events)| {
            events.sort_by(|a, b| a.symbol.cmp(&b.symbol));
            EarningsDayGroup { date, events }
        })
        .collect();
    days.sort_by(|a, b| a.date.cmp(&b.date));
    (days, total)
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<EarningsQuery>,
) -> Result<Json<EarningsResponse>, HandlerError> {
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

    let projected: Vec<EarningsEvent> = payload
        .events
        .into_iter()
        .map(|e| EarningsEvent {
            symbol: e.symbol,
            company: e.company,
            date: e.date,
            timing: e.timing,
            eps_estimate: e.eps_estimate,
            eps_actual_prior: e.eps_actual_prior,
        })
        .collect();

    let (days, total) = group_by_day(
        projected,
        q.symbols.as_deref(),
        q.limit.unwrap_or(DEFAULT_LIMIT),
    );

    Ok(Json(EarningsResponse {
        days,
        total,
        lookahead_days: payload.lookahead_days,
        assembled_at_ms: payload.assembled_at_ms,
        stale,
    }))
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
    use crate::market::v1::EARNINGS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn snapshot(events: &[(&str, &str, &str, EarningsTiming)]) -> Value {
        serde_json::json!({
            "events": events.iter().map(|(sym, co, date, timing)| serde_json::json!({
                "symbol": sym,
                "company": co,
                "date": date,
                "timing": timing,
                "eps_estimate": 1.23,
                "eps_actual_prior": 1.10,
            })).collect::<Vec<_>>(),
            "lookahead_days": 7,
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(EARNINGS_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    fn ev(symbol: &str, date: &str) -> EarningsEvent {
        EarningsEvent {
            symbol: symbol.into(),
            company: format!("{symbol} Co"),
            date: date.into(),
            timing: EarningsTiming::BeforeOpen,
            eps_estimate: None,
            eps_actual_prior: None,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:earnings-calendar:7d:v1");
    }

    #[test]
    fn group_by_day_buckets_and_sorts_within_day() {
        let events = vec![
            ev("QQQ", "2026-05-08"),
            ev("SPY", "2026-05-06"),
            ev("DIA", "2026-05-06"),
        ];
        let (days, total) = group_by_day(events, None, 100);
        assert_eq!(total, 3);
        assert_eq!(days.len(), 2);
        // Sorted ascending by date.
        assert_eq!(days[0].date, "2026-05-06");
        assert_eq!(days[1].date, "2026-05-08");
        // Within day sorted ascending by symbol.
        let day0_syms: Vec<&str> = days[0].events.iter().map(|e| e.symbol.as_str()).collect();
        assert_eq!(day0_syms, vec!["DIA", "SPY"]);
    }

    #[test]
    fn group_by_day_filters_by_symbols_csv() {
        let events = vec![
            ev("QQQ", "2026-05-08"),
            ev("SPY", "2026-05-06"),
            ev("DIA", "2026-05-06"),
        ];
        let (days, total) = group_by_day(events, Some("spy,qqq"), 100);
        assert_eq!(total, 2);
        assert_eq!(days.len(), 2);
    }

    #[test]
    fn group_by_day_clamps_limit_to_max() {
        let events: Vec<EarningsEvent> = (0..(MAX_LIMIT + 30))
            .map(|i| ev(&format!("S{i}"), "2026-05-06"))
            .collect();
        let (days, total) = group_by_day(events, None, MAX_LIMIT * 5);
        assert_eq!(total, MAX_LIMIT + 30);
        assert_eq!(days[0].events.len(), MAX_LIMIT);
    }

    #[test]
    fn group_by_day_zero_limit_floors_to_one() {
        let events = vec![ev("SPY", "2026-05-06"), ev("QQQ", "2026-05-06")];
        let (days, _) = group_by_day(events, None, 0);
        assert_eq!(days[0].events.len(), 1);
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(EARNINGS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_grouped_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(&[
            ("SPY", "S&P 500", "2026-05-06", EarningsTiming::BeforeOpen),
            (
                "QQQ",
                "Invesco QQQ",
                "2026-05-08",
                EarningsTiming::AfterClose,
            ),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(EARNINGS_PATH)
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
        let days = parsed.pointer("/days").unwrap().as_array().unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(parsed.pointer("/lookaheadDays").unwrap().as_u64(), Some(7));
        assert!(parsed.pointer("/days/0/events/0/epsEstimate").is_some());
    }

    #[tokio::test]
    async fn handler_filters_by_symbols_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(&[
            ("SPY", "S&P 500", "2026-05-06", EarningsTiming::BeforeOpen),
            (
                "QQQ",
                "Invesco QQQ",
                "2026-05-08",
                EarningsTiming::AfterClose,
            ),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{EARNINGS_PATH}?symbols=spy"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: EarningsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.days[0].events[0].symbol, "SPY");
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(&[(
            "SPY",
            "S&P 500",
            "2026-05-06",
            EarningsTiming::BeforeOpen,
        )]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(EARNINGS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: EarningsResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "events": "not-an-array" }));
        set_cached_json(&pool, CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(EARNINGS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

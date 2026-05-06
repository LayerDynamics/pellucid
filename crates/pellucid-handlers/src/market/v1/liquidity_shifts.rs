//! `GET /api/market/v1/liquidity-shifts` handler.
//!
//! Pure cache reader of the SLOW-tier
//! `market:liquidity-shifts:v1` slot. Anonymous tier.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — pinned to the seeder.
pub const CACHE_KEY: &str = "market:liquidity-shifts:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// One series row in the wire response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LiquiditySeries {
    /// FRED series code (`WALCL`, `M2SL`, `RRPONTSYD`).
    #[serde(rename = "seriesCode", alias = "series_code")]
    pub series_code: String,
    /// Latest observation value ($B).
    #[serde(rename = "latestValue", alias = "latest_value")]
    pub latest_value: f64,
    /// Latest observation date.
    #[serde(rename = "latestDate", alias = "latest_date")]
    pub latest_date: String,
    /// Prior-period observation value, when present.
    #[serde(
        rename = "priorValue",
        alias = "prior_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub prior_value: Option<f64>,
    /// Latest minus prior.
    #[serde(
        rename = "periodDelta",
        alias = "period_delta",
        skip_serializing_if = "Option::is_none"
    )]
    pub period_delta: Option<f64>,
    /// Period delta as a percent of prior.
    #[serde(
        rename = "periodDeltaPct",
        alias = "period_delta_pct",
        skip_serializing_if = "Option::is_none"
    )]
    pub period_delta_pct: Option<f64>,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiquidityShiftsResponse {
    /// Series rows in canonical order.
    pub series: Vec<LiquiditySeries>,
    /// `WALCL - RRPONTSYD` net-liquidity proxy ($B).
    #[serde(rename = "netLiquidityBillionUsd")]
    pub net_liquidity_billion_usd: f64,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when the response was synthesised from a stale row.
    pub stale: bool,
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
struct SeederSeries {
    series_code: String,
    latest_value: f64,
    latest_date: String,
    #[serde(default)]
    prior_value: Option<f64>,
    #[serde(default)]
    period_delta: Option<f64>,
    #[serde(default)]
    period_delta_pct: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    series: Vec<SeederSeries>,
    #[serde(default)]
    net_liquidity_billion_usd: f64,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<LiquidityShiftsResponse>, HandlerError> {
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

    let series: Vec<LiquiditySeries> = payload
        .series
        .into_iter()
        .map(|s| LiquiditySeries {
            series_code: s.series_code,
            latest_value: s.latest_value,
            latest_date: s.latest_date,
            prior_value: s.prior_value,
            period_delta: s.period_delta,
            period_delta_pct: s.period_delta_pct,
        })
        .collect();

    Ok(Json(LiquidityShiftsResponse {
        series,
        net_liquidity_billion_usd: payload.net_liquidity_billion_usd,
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
    use crate::market::v1::LIQUIDITY_SHIFTS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn snapshot() -> Value {
        serde_json::json!({
            "series": [
                {
                    "series_code": "WALCL",
                    "latest_value": 7_200.0_f64,
                    "latest_date": "2026-05-01",
                    "prior_value": 7_180.0_f64,
                    "period_delta": 20.0_f64,
                    "period_delta_pct": 0.279_f64,
                },
                {
                    "series_code": "M2SL",
                    "latest_value": 21_000.0_f64,
                    "latest_date": "2026-05-01",
                    "prior_value": 20_980.0_f64,
                    "period_delta": 20.0_f64,
                    "period_delta_pct": 0.095_f64,
                },
                {
                    "series_code": "RRPONTSYD",
                    "latest_value": 450.0_f64,
                    "latest_date": "2026-05-01",
                    "prior_value": 480.0_f64,
                    "period_delta": -30.0_f64,
                    "period_delta_pct": -6.25_f64,
                },
            ],
            "net_liquidity_billion_usd": 6_750.0_f64,
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            LIQUIDITY_SHIFTS_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:liquidity-shifts:v1");
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIQUIDITY_SHIFTS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_envelope_with_camelcase_fields() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot());
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIQUIDITY_SHIFTS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/series/0/seriesCode").is_some());
        assert!(parsed.pointer("/series/0/periodDelta").is_some());
        assert!(parsed.pointer("/series/0/periodDeltaPct").is_some());
        assert_eq!(
            parsed
                .pointer("/netLiquidityBillionUsd")
                .and_then(Value::as_f64),
            Some(6_750.0),
        );
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot());
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIQUIDITY_SHIFTS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: LiquidityShiftsResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "series": "not-an-array" }));
        set_cached_json(&pool, CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIQUIDITY_SHIFTS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

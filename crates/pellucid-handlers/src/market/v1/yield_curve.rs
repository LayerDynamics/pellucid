//! `GET /api/market/v1/yield-curve` handler.
//!
//! Pure cache reader of the SLOW-tier
//! `market:yield-curve:treasury:v1` snapshot. Anonymous tier.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — pinned to the seeder.
pub const CACHE_KEY: &str = "market:yield-curve:treasury:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// One curve point.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct YieldPoint {
    /// FRED series code.
    #[serde(rename = "seriesCode", alias = "series_code")]
    pub series_code: String,
    /// Maturity label (`1M`, `2Y`, …).
    #[serde(rename = "maturityLabel", alias = "maturity_label")]
    pub maturity_label: String,
    /// Maturity in months.
    #[serde(rename = "maturityMonths", alias = "maturity_months")]
    pub maturity_months: u32,
    /// Yield in percent.
    #[serde(rename = "yieldPct", alias = "yield_pct")]
    pub yield_pct: f64,
    /// Observation date.
    #[serde(rename = "observationDate", alias = "observation_date")]
    pub observation_date: String,
}

/// Spreads computed by the handler.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct YieldSpreads {
    /// 10Y minus 2Y in percent (the headline inversion gauge).
    /// `None` when either point is missing.
    #[serde(rename = "tenMinusTwo", skip_serializing_if = "Option::is_none")]
    pub ten_minus_two: Option<f64>,
    /// 10Y minus 3M in percent.
    #[serde(rename = "tenMinusThreeMonth", skip_serializing_if = "Option::is_none")]
    pub ten_minus_three_month: Option<f64>,
    /// 30Y minus 5Y in percent (long-end steepness).
    #[serde(rename = "thirtyMinusFive", skip_serializing_if = "Option::is_none")]
    pub thirty_minus_five: Option<f64>,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YieldCurveResponse {
    /// Curve points in canonical maturity order.
    pub points: Vec<YieldPoint>,
    /// Pre-computed inversion + steepness spreads.
    pub spreads: YieldSpreads,
    /// True when the curve has any inverted segment (any
    /// adjacent maturities where the longer yield < shorter).
    pub inverted: bool,
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
struct SeederPoint {
    series_code: String,
    maturity_label: String,
    maturity_months: u32,
    yield_pct: f64,
    observation_date: String,
}

#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    points: Vec<SeederPoint>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Compute the three named spreads. Pure.
#[must_use]
pub fn spreads(points: &[YieldPoint]) -> YieldSpreads {
    let yld = |months: u32| -> Option<f64> {
        points
            .iter()
            .find(|p| p.maturity_months == months)
            .map(|p| p.yield_pct)
    };
    YieldSpreads {
        ten_minus_two: match (yld(120), yld(24)) {
            (Some(t), Some(two)) => Some(t - two),
            _ => None,
        },
        ten_minus_three_month: match (yld(120), yld(3)) {
            (Some(t), Some(three_m)) => Some(t - three_m),
            _ => None,
        },
        thirty_minus_five: match (yld(360), yld(60)) {
            (Some(thirty), Some(five)) => Some(thirty - five),
            _ => None,
        },
    }
}

/// Detect any inversion (longer maturity has lower yield than a
/// shorter one). Pure.
#[must_use]
pub fn is_inverted(points: &[YieldPoint]) -> bool {
    let mut sorted: Vec<&YieldPoint> = points.iter().collect();
    sorted.sort_by_key(|p| p.maturity_months);
    sorted.windows(2).any(|w| w[1].yield_pct < w[0].yield_pct)
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<YieldCurveResponse>, HandlerError> {
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

    let mut points: Vec<YieldPoint> = payload
        .points
        .into_iter()
        .map(|p| YieldPoint {
            series_code: p.series_code,
            maturity_label: p.maturity_label,
            maturity_months: p.maturity_months,
            yield_pct: p.yield_pct,
            observation_date: p.observation_date,
        })
        .collect();
    points.sort_by_key(|p| p.maturity_months);
    let inverted = is_inverted(&points);
    let spreads = spreads(&points);

    Ok(Json(YieldCurveResponse {
        points,
        spreads,
        inverted,
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
    use crate::market::v1::YIELD_CURVE_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn snapshot(points: &[(&str, &str, u32, f64)]) -> Value {
        serde_json::json!({
            "points": points.iter().map(|(code, label, months, yld)| serde_json::json!({
                "series_code": code,
                "maturity_label": label,
                "maturity_months": months,
                "yield_pct": yld,
                "observation_date": "2026-05-05",
            })).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            YIELD_CURVE_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    fn pt(months: u32, yld: f64) -> YieldPoint {
        YieldPoint {
            series_code: format!("DGS{months}"),
            maturity_label: format!("{months}M"),
            maturity_months: months,
            yield_pct: yld,
            observation_date: "2026-05-05".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:yield-curve:treasury:v1");
    }

    #[test]
    fn spreads_compute_from_canonical_maturities() {
        let points = vec![
            pt(3, 5.0),
            pt(24, 4.5),
            pt(60, 4.2),
            pt(120, 4.1),
            pt(360, 4.4),
        ];
        let s = spreads(&points);
        assert!((s.ten_minus_two.unwrap() - -0.4).abs() < 1e-9);
        assert!((s.ten_minus_three_month.unwrap() - -0.9).abs() < 1e-9);
        assert!((s.thirty_minus_five.unwrap() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn spreads_missing_endpoint_yields_none() {
        // Only the 3M and 10Y are present.
        let points = vec![pt(3, 5.0), pt(120, 4.0)];
        let s = spreads(&points);
        assert!(s.ten_minus_two.is_none());
        assert!(s.ten_minus_three_month.is_some());
        assert!(s.thirty_minus_five.is_none());
    }

    #[test]
    fn is_inverted_true_when_short_above_long() {
        let points = vec![pt(3, 5.0), pt(24, 4.5), pt(120, 4.2)];
        assert!(is_inverted(&points));
    }

    #[test]
    fn is_inverted_false_for_normal_curve() {
        let points = vec![pt(3, 4.0), pt(24, 4.5), pt(120, 4.7)];
        assert!(!is_inverted(&points));
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(YIELD_CURVE_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_curve_with_camelcase_fields_and_spreads() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(&[
            ("DGS3MO", "3M", 3, 5.0),
            ("DGS2", "2Y", 24, 4.5),
            ("DGS5", "5Y", 60, 4.2),
            ("DGS10", "10Y", 120, 4.1),
            ("DGS30", "30Y", 360, 4.4),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(YIELD_CURVE_PATH)
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
        assert!(parsed.pointer("/points/0/seriesCode").is_some());
        assert!(parsed.pointer("/spreads/tenMinusTwo").is_some());
        assert_eq!(
            parsed.pointer("/inverted").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(&[("DGS10", "10Y", 120, 4.1)]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(YIELD_CURVE_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: YieldCurveResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "points": "not-an-array" }));
        set_cached_json(&pool, CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(YIELD_CURVE_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

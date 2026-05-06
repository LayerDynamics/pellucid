//! `GET /api/market/v1/stablecoins` handler.
//!
//! Pure cache reader of the FAST-tier
//! `market:stablecoin-snapshot:v1` slot. Anonymous tier.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — pinned to the seeder.
pub const CACHE_KEY: &str = "market:stablecoin-snapshot:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Threshold (absolute %) that triggers the depeg-risk badge.
pub const DEPEG_RISK_THRESHOLD_PCT: f64 = 1.0;

/// One stablecoin row in the wire response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StablecoinRow {
    /// CoinGecko id.
    pub id: String,
    /// Display ticker (uppercase).
    pub symbol: String,
    /// USD price.
    pub usd: f64,
    /// 24h percent change vs USD.
    #[serde(rename = "usd24hChange", alias = "usd_24h_change")]
    pub usd_24h_change: f64,
    /// USD market cap.
    #[serde(rename = "usdMarketCap", alias = "usd_market_cap")]
    pub usd_market_cap: f64,
    /// Pre-computed peg deviation in percent.
    #[serde(rename = "pegDeviationPct", alias = "peg_deviation_pct")]
    pub peg_deviation_pct: f64,
    /// True when `|peg_deviation_pct| >= DEPEG_RISK_THRESHOLD_PCT`.
    #[serde(rename = "depegRisk")]
    pub depeg_risk: bool,
    /// Wall-clock seconds when CoinGecko stamped the row.
    #[serde(rename = "lastUpdatedAt", alias = "last_updated_at")]
    pub last_updated_at: i64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StablecoinsResponse {
    /// Rows sorted descending by market cap.
    pub rows: Vec<StablecoinRow>,
    /// Aggregate market cap across the basket (USD).
    #[serde(rename = "totalMarketCapUsd")]
    pub total_market_cap_usd: f64,
    /// Count of rows currently flagged as depeg risk.
    #[serde(rename = "depegCount")]
    pub depeg_count: usize,
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
struct SeederRow {
    id: String,
    symbol: String,
    usd: f64,
    usd_24h_change: f64,
    usd_market_cap: f64,
    peg_deviation_pct: f64,
    last_updated_at: i64,
}

#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<SeederRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Project + sort + compute aggregates. Pure.
#[must_use]
pub fn project(rows: Vec<SeederRowOwned>) -> (Vec<StablecoinRow>, f64, usize) {
    let mut projected: Vec<StablecoinRow> = rows
        .into_iter()
        .map(|r| {
            let depeg_risk = r.peg_deviation_pct.abs() >= DEPEG_RISK_THRESHOLD_PCT;
            StablecoinRow {
                id: r.id,
                symbol: r.symbol.to_uppercase(),
                usd: r.usd,
                usd_24h_change: r.usd_24h_change,
                usd_market_cap: r.usd_market_cap,
                peg_deviation_pct: r.peg_deviation_pct,
                depeg_risk,
                last_updated_at: r.last_updated_at,
            }
        })
        .collect();
    projected.sort_by(|a, b| {
        b.usd_market_cap
            .partial_cmp(&a.usd_market_cap)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total: f64 = projected.iter().map(|r| r.usd_market_cap).sum();
    let depeg = projected.iter().filter(|r| r.depeg_risk).count();
    (projected, total, depeg)
}

/// Public mirror of the seeder row.
#[derive(Clone, Debug)]
pub struct SeederRowOwned {
    /// CoinGecko id.
    pub id: String,
    /// Display ticker.
    pub symbol: String,
    /// USD price.
    pub usd: f64,
    /// 24h change.
    pub usd_24h_change: f64,
    /// USD market cap.
    pub usd_market_cap: f64,
    /// Peg deviation in percent.
    pub peg_deviation_pct: f64,
    /// Last-update wall-clock seconds.
    pub last_updated_at: i64,
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<StablecoinsResponse>, HandlerError> {
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

    let owned: Vec<SeederRowOwned> = payload
        .rows
        .into_iter()
        .map(|r| SeederRowOwned {
            id: r.id,
            symbol: r.symbol,
            usd: r.usd,
            usd_24h_change: r.usd_24h_change,
            usd_market_cap: r.usd_market_cap,
            peg_deviation_pct: r.peg_deviation_pct,
            last_updated_at: r.last_updated_at,
        })
        .collect();
    let (rows, total_cap, depeg) = project(owned);

    Ok(Json(StablecoinsResponse {
        rows,
        total_market_cap_usd: total_cap,
        depeg_count: depeg,
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
    use crate::market::v1::STABLECOINS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn snapshot(rows: &[(&str, &str, f64, f64, f64, f64)]) -> Value {
        serde_json::json!({
            "rows": rows.iter().map(|(id, sym, usd, change, cap, peg)| serde_json::json!({
                "id": id,
                "symbol": sym,
                "usd": usd,
                "usd_24h_change": change,
                "usd_market_cap": cap,
                "peg_deviation_pct": peg,
                "last_updated_at": 1_714_060_800_i64,
            })).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            STABLECOINS_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:stablecoin-snapshot:v1");
    }

    #[test]
    fn project_sorts_by_market_cap_desc_and_flags_depegs() {
        let rows = vec![
            SeederRowOwned {
                id: "dai".into(),
                symbol: "dai".into(),
                usd: 0.985,
                usd_24h_change: 0.0,
                usd_market_cap: 5_000_000_000.0,
                peg_deviation_pct: -1.5,
                last_updated_at: 0,
            },
            SeederRowOwned {
                id: "tether".into(),
                symbol: "usdt".into(),
                usd: 1.001,
                usd_24h_change: 0.0,
                usd_market_cap: 100_000_000_000.0,
                peg_deviation_pct: 0.1,
                last_updated_at: 0,
            },
        ];
        let (projected, total, depeg) = project(rows);
        // Sorted desc by market cap → USDT first.
        assert_eq!(projected[0].symbol, "USDT");
        assert_eq!(projected[1].symbol, "DAI");
        assert!((total - 105_000_000_000.0).abs() < 1e-3);
        // DAI's |1.5| >= 1.0 → depeg risk.
        assert_eq!(depeg, 1);
        assert!(projected[1].depeg_risk);
        assert!(!projected[0].depeg_risk);
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(STABLECOINS_PATH)
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
        let env = Envelope::new(snapshot(&[
            ("tether", "usdt", 1.001, 0.0, 100_000_000_000.0, 0.1),
            ("usd-coin", "usdc", 0.998, 0.0, 30_000_000_000.0, -0.2),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(STABLECOINS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/rows/0/depegRisk").is_some());
        assert!(parsed.pointer("/totalMarketCapUsd").is_some());
        assert_eq!(
            parsed.pointer("/depegCount").and_then(Value::as_u64),
            Some(0),
        );
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(&[
            ("tether", "usdt", 1.0, 0.0, 1.0, 0.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(STABLECOINS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: StablecoinsResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "rows": "not-an-array" }));
        set_cached_json(&pool, CACHE_KEY, &bad, 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(STABLECOINS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

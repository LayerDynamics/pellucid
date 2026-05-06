//! `GET /api/market/v1/etf-flows` handler.
//!
//! Pure cache reader of the FAST-tier
//! `market:etf-flows:current:v1` snapshot the
//! `seed_etf_flows` seeder writes (T3.8 markets domain). The
//! seeder publishes one row per ETF with the most-recent
//! session's dollar-volume, the trailing-window average, and
//! the ratio between them; the panel renders the rows as a
//! sortable activity table with a "vs typical" badge.
//!
//! ## Tier
//!
//! Anonymous (tier 0). ETF flow data is a public surface.
//!
//! ## M4 outage path
//!
//! Same envelope as every other family handler: 503 +
//! `Retry-After` + `bootstrap_upstream_empty`.

use std::cmp::Ordering;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key the seeder writes — see
/// `pellucid_seeders::markets::seed_etf_flows::CACHE_KEY`.
pub const CACHE_KEY: &str = "market:etf-flows:current:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Default `?limit` when the client doesn't ask. The seeder's
/// default basket is 10 ETFs; the cap leaves headroom for a
/// custom basket without bloating the wire bytes.
pub const DEFAULT_LIMIT: usize = 50;

/// Hard cap on `?limit`.
pub const MAX_LIMIT: usize = 100;

/// Sort modes. Defaults to `activity-ratio-desc` so the panel's
/// first render highlights symbols trading well above their
/// typical volume.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SortMode {
    /// Activity ratio descending (most-active first).
    ActivityRatioDesc,
    /// Activity ratio ascending (least-active first).
    ActivityRatioAsc,
    /// Latest dollar volume descending.
    DollarVolumeDesc,
    /// Symbol ascending (alphabetical).
    SymbolAsc,
}

impl SortMode {
    /// Parse the wire string. Reject typos so a misconfigured
    /// caller surfaces a 400 rather than silently fall through.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "activity-ratio-desc" => Some(Self::ActivityRatioDesc),
            "activity-ratio-asc" => Some(Self::ActivityRatioAsc),
            "dollar-volume-desc" => Some(Self::DollarVolumeDesc),
            "symbol-asc" => Some(Self::SymbolAsc),
            _ => None,
        }
    }
}

/// One row in the wire response. Field renames give the wire
/// camelCase while the deserializer accepts the seeder's
/// snake_case via `#[serde(alias = "…")]`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EtfFlowRow {
    pub symbol: String,
    #[serde(rename = "latestDollarVolume", alias = "latest_dollar_volume")]
    pub latest_dollar_volume: f64,
    #[serde(rename = "avgDollarVolume", alias = "avg_dollar_volume")]
    pub avg_dollar_volume: f64,
    #[serde(rename = "activityRatio", alias = "activity_ratio")]
    pub activity_ratio: f64,
    /// Wall-clock seconds the seeder stamped the row at.
    #[serde(rename = "latestSessionTs", alias = "latest_session_ts")]
    pub latest_session_ts: i64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EtfFlowsResponse {
    pub rows: Vec<EtfFlowRow>,
    /// Trailing-window size in days that the seeder used when
    /// computing `avg_dollar_volume`. The panel labels the
    /// activity-ratio column "vs N-day avg".
    #[serde(rename = "lookbackDays")]
    pub lookback_days: u32,
    /// Total rows in the cache slot before the limit clamp.
    pub total: usize,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs", alias = "assembled_at_ms")]
    pub assembled_at_ms: i64,
    /// Whether the response was synthesised from a stale row.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct EtfFlowsQuery {
    /// Cap on rows. Defaults to [`DEFAULT_LIMIT`], clamped to
    /// [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Sort mode — wire string, parsed via [`SortMode::parse`].
    /// When absent or unparseable the default is
    /// `activity-ratio-desc`.
    #[serde(default)]
    pub sort: Option<String>,
    /// Comma-separated allow-list of ETF symbols. Filters the
    /// seeder snapshot before the limit clamp.
    #[serde(default)]
    pub symbols: Option<String>,
}

/// Errors the handler can surface.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// `?sort=` value is not in the documented set.
    #[error("invalid request: sort {0:?} is not a known sort mode")]
    InvalidSort(String),
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
            Self::InvalidSort(_) => "invalid_request",
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::InvalidSort(_) => StatusCode::BAD_REQUEST,
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

/// Cache payload shape the seeder writes. Mirror the
/// `EtfFlowsSnapshot` struct with snake_case + camelCase
/// aliases so we can read either a freshly published envelope
/// or a hand-written test fixture.
#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<SeederRow>,
    #[serde(default, rename = "lookback_days", alias = "lookbackDays")]
    lookback_days: u32,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SeederRow {
    symbol: String,
    #[serde(default, rename = "latest_dollar_volume", alias = "latestDollarVolume")]
    latest_dollar_volume: f64,
    #[serde(default, rename = "avg_dollar_volume", alias = "avgDollarVolume")]
    avg_dollar_volume: f64,
    #[serde(default, rename = "activity_ratio", alias = "activityRatio")]
    activity_ratio: f64,
    #[serde(default, rename = "latest_session_ts", alias = "latestSessionTs")]
    latest_session_ts: i64,
}

/// Apply the `?symbols` allow-list, sort mode, and `?limit` to
/// a freshly read snapshot. Pure — exported for tests so each
/// boundary is pinned without hitting the cache.
#[must_use]
pub fn apply_filters_and_sort(
    rows: Vec<EtfFlowRow>,
    sort: SortMode,
    q: &EtfFlowsQuery,
) -> (Vec<EtfFlowRow>, usize) {
    let mut filtered: Vec<EtfFlowRow> = if let Some(raw) = q.symbols.as_deref() {
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
    sort_in_place(&mut filtered, sort);
    let total = filtered.len();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    if filtered.len() > limit {
        filtered.truncate(limit);
    }
    (filtered, total)
}

fn sort_in_place(rows: &mut [EtfFlowRow], mode: SortMode) {
    match mode {
        SortMode::ActivityRatioDesc => rows.sort_by(|a, b| {
            b.activity_ratio
                .partial_cmp(&a.activity_ratio)
                .unwrap_or(Ordering::Equal)
        }),
        SortMode::ActivityRatioAsc => rows.sort_by(|a, b| {
            a.activity_ratio
                .partial_cmp(&b.activity_ratio)
                .unwrap_or(Ordering::Equal)
        }),
        SortMode::DollarVolumeDesc => rows.sort_by(|a, b| {
            b.latest_dollar_volume
                .partial_cmp(&a.latest_dollar_volume)
                .unwrap_or(Ordering::Equal)
        }),
        SortMode::SymbolAsc => rows.sort_by(|a, b| a.symbol.cmp(&b.symbol)),
    }
}

/// Project a seeder row to the wire shape.
fn project_row(r: SeederRow) -> EtfFlowRow {
    EtfFlowRow {
        symbol: r.symbol,
        latest_dollar_volume: r.latest_dollar_volume,
        avg_dollar_volume: r.avg_dollar_volume,
        activity_ratio: r.activity_ratio,
        latest_session_ts: r.latest_session_ts,
    }
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<EtfFlowsQuery>,
) -> Result<Json<EtfFlowsResponse>, HandlerError> {
    let sort = match q.sort.as_deref() {
        Some(raw) => {
            SortMode::parse(raw).ok_or_else(|| HandlerError::InvalidSort(raw.to_string()))?
        }
        None => SortMode::ActivityRatioDesc,
    };
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
    let projected: Vec<EtfFlowRow> = payload.rows.into_iter().map(project_row).collect();
    let (rows, total) = apply_filters_and_sort(projected, sort, &q);
    Ok(Json(EtfFlowsResponse {
        rows,
        lookback_days: payload.lookback_days,
        total,
        assembled_at_ms: payload.assembled_at_ms,
        stale,
    }))
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

    use crate::market::v1::ETF_FLOWS_PATH;

    fn row(symbol: &str, latest: f64, avg: f64) -> EtfFlowRow {
        let activity = if avg <= 0.0 { 0.0 } else { latest / avg };
        EtfFlowRow {
            symbol: symbol.into(),
            latest_dollar_volume: latest,
            avg_dollar_volume: avg,
            activity_ratio: activity,
            latest_session_ts: 1_700_000_000,
        }
    }

    fn snapshot(rows: Vec<(&str, f64, f64)>) -> serde_json::Value {
        serde_json::json!({
            "rows": rows.into_iter().map(|(s, latest, avg)| {
                let activity = if avg <= 0.0 { 0.0 } else { latest / avg };
                serde_json::json!({
                    "symbol": s,
                    "latest_dollar_volume": latest,
                    "avg_dollar_volume": avg,
                    "activity_ratio": activity,
                    "latest_session_ts": 1_700_000_000_i64,
                })
            }).collect::<Vec<_>>(),
            "lookback_days": 5,
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            ETF_FLOWS_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn sort_mode_parse_accepts_only_known_strings() {
        assert_eq!(
            SortMode::parse("activity-ratio-desc"),
            Some(SortMode::ActivityRatioDesc),
        );
        assert_eq!(
            SortMode::parse("activity-ratio-asc"),
            Some(SortMode::ActivityRatioAsc),
        );
        assert_eq!(
            SortMode::parse("dollar-volume-desc"),
            Some(SortMode::DollarVolumeDesc),
        );
        assert_eq!(SortMode::parse("symbol-asc"), Some(SortMode::SymbolAsc));
        assert_eq!(SortMode::parse("ActivityRatioDesc"), None);
        assert_eq!(SortMode::parse(""), None);
    }

    #[test]
    fn sort_activity_ratio_descending_orders_most_active_first() {
        let rows = vec![
            row("A", 100.0, 100.0),
            row("HOT", 500.0, 100.0),
            row("COLD", 25.0, 100.0),
        ];
        let (out, _) =
            apply_filters_and_sort(rows, SortMode::ActivityRatioDesc, &EtfFlowsQuery::default());
        assert_eq!(out[0].symbol, "HOT");
        assert_eq!(out[1].symbol, "A");
        assert_eq!(out[2].symbol, "COLD");
    }

    #[test]
    fn sort_dollar_volume_descending_orders_largest_first() {
        let rows = vec![row("S", 50.0, 50.0), row("L", 1000.0, 1000.0)];
        let (out, _) =
            apply_filters_and_sort(rows, SortMode::DollarVolumeDesc, &EtfFlowsQuery::default());
        assert_eq!(out[0].symbol, "L");
    }

    #[test]
    fn sort_symbol_asc_orders_alphabetically() {
        let rows = vec![
            row("ZZZ", 1.0, 1.0),
            row("AAA", 1.0, 1.0),
            row("MMM", 1.0, 1.0),
        ];
        let (out, _) = apply_filters_and_sort(rows, SortMode::SymbolAsc, &EtfFlowsQuery::default());
        assert_eq!(
            out.iter().map(|r| r.symbol.as_str()).collect::<Vec<_>>(),
            vec!["AAA", "MMM", "ZZZ"]
        );
    }

    #[test]
    fn apply_filters_filters_by_symbols_csv_case_insensitive() {
        let rows = vec![
            row("SPY", 1.0, 1.0),
            row("QQQ", 1.0, 1.0),
            row("DIA", 1.0, 1.0),
        ];
        let q = EtfFlowsQuery {
            symbols: Some("spy,DIA".into()),
            ..EtfFlowsQuery::default()
        };
        let (out, total) = apply_filters_and_sort(rows, SortMode::SymbolAsc, &q);
        assert_eq!(
            out.iter().map(|r| r.symbol.as_str()).collect::<Vec<_>>(),
            vec!["DIA", "SPY"]
        );
        assert_eq!(total, 2);
    }

    #[test]
    fn apply_filters_clamps_limit_to_max() {
        let rows: Vec<EtfFlowRow> = (0..(MAX_LIMIT + 5))
            .map(|i| row(&format!("S{i}"), 1.0, 1.0))
            .collect();
        let q = EtfFlowsQuery {
            limit: Some(MAX_LIMIT * 10),
            ..EtfFlowsQuery::default()
        };
        let (out, total) = apply_filters_and_sort(rows, SortMode::SymbolAsc, &q);
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 5);
    }

    #[test]
    fn handler_error_status_codes() {
        assert_eq!(
            HandlerError::InvalidSort("x".into()).status(),
            Code::BAD_REQUEST,
        );
        assert_eq!(HandlerError::Cache("x".into()).status(), Code::BAD_GATEWAY,);
        assert_eq!(
            HandlerError::Outage {
                retry_after_secs: 0
            }
            .status(),
            Code::SERVICE_UNAVAILABLE,
        );
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(ETF_FLOWS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_400_for_invalid_sort() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![("SPY", 1.0, 1.0)]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{}?sort=bogus", ETF_FLOWS_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_returns_envelope_for_populated_cache() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            ("SPY", 1_000.0, 500.0),
            ("QQQ", 2_000.0, 4_000.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(ETF_FLOWS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: EtfFlowsResponse = serde_json::from_slice(&body).unwrap();
        // Default sort is activity-ratio-desc → SPY (2.0) before QQQ (0.5).
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[0].symbol, "SPY");
        assert_eq!(parsed.lookback_days, 5);
        assert_eq!(parsed.total, 2);
        assert_eq!(parsed.assembled_at_ms, 1_700_000_000_000);
        assert!(!parsed.stale);
    }

    #[tokio::test]
    async fn handler_clamps_limit_via_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            ("A", 1.0, 1.0),
            ("B", 2.0, 1.0),
            ("C", 3.0, 1.0),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{}?limit=2", ETF_FLOWS_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: EtfFlowsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.total, 3);
    }
}

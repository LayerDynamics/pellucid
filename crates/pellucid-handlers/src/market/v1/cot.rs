//! `GET /api/market/v1/cot` handler.
//!
//! Pure cache reader of the SLOW-tier
//! `market:cot-report:weekly:v1` snapshot the `seed_cot` seeder
//! writes (T3.8 markets domain). Anonymous tier — CFTC
//! Commitments-of-Traders data is a public surface.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "rows": [{
//!     "contractCode": "088691",
//!     "contractName": "GOLD",
//!     "reportDate": "2026-04-29",
//!     "openInterestAll": 480000,
//!     "producerLong": 100000, "producerShort": 120000,
//!     "swapLong":      80000, "swapShort":      90000,
//!     "managedMoneyLong":  120000, "managedMoneyShort":  60000,
//!     "managedMoneyNet":   60000,
//!     "managedMoneyNetPctOi": 12.5
//!   }],
//!   "total": 8,
//!   "assembledAtMs": 1746360000000,
//!   "stale": false
//! }
//! ```
//!
//! ## M4 outage path
//!
//! Same envelope as every other family handler.

use std::cmp::Ordering;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key the handler reads — pinned to the seeder's slot so
/// a key bump on the seeder side surfaces here as a compile-
/// time string mismatch.
pub const CACHE_KEY: &str = "market:cot-report:weekly:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Default `?limit` when the client doesn't ask. The seeder's
/// default basket is 8 contracts; the cap leaves headroom for a
/// custom basket.
pub const DEFAULT_LIMIT: usize = 50;

/// Hard cap on `?limit`.
pub const MAX_LIMIT: usize = 100;

/// Sort modes. Default `managed-money-net-desc` so the panel's
/// first render highlights contracts with the most-bullish
/// managed-money positioning.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SortMode {
    /// Net managed-money position descending (most-bullish first).
    ManagedMoneyNetDesc,
    /// Net managed-money position ascending (most-bearish first).
    ManagedMoneyNetAsc,
    /// Open interest descending.
    OpenInterestDesc,
    /// Contract name ascending (alphabetical).
    NameAsc,
}

impl SortMode {
    /// Parse the wire string. Reject typos so a misconfigured
    /// caller surfaces a 400 rather than fall through.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "managed-money-net-desc" => Some(Self::ManagedMoneyNetDesc),
            "managed-money-net-asc" => Some(Self::ManagedMoneyNetAsc),
            "open-interest-desc" => Some(Self::OpenInterestDesc),
            "name-asc" => Some(Self::NameAsc),
            _ => None,
        }
    }
}

/// One row in the wire response. Field renames give the wire
/// camelCase while the deserializer accepts the seeder's
/// snake_case via `#[serde(alias = "…")]`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CotRow {
    /// CFTC contract market code (e.g. `"088691"`).
    #[serde(rename = "contractCode", alias = "contract_code")]
    pub contract_code: String,
    /// Human-readable contract name.
    #[serde(rename = "contractName", alias = "contract_name")]
    pub contract_name: String,
    /// Report week (`YYYY-MM-DD`).
    #[serde(rename = "reportDate", alias = "report_date")]
    pub report_date: String,
    /// Aggregate open interest.
    #[serde(rename = "openInterestAll", alias = "open_interest_all")]
    pub open_interest_all: i64,
    /// Producer / merchant / processor / user long.
    #[serde(rename = "producerLong", alias = "producer_long")]
    pub producer_long: i64,
    /// Producer / merchant / processor / user short.
    #[serde(rename = "producerShort", alias = "producer_short")]
    pub producer_short: i64,
    /// Swap dealer long.
    #[serde(rename = "swapLong", alias = "swap_long")]
    pub swap_long: i64,
    /// Swap dealer short.
    #[serde(rename = "swapShort", alias = "swap_short")]
    pub swap_short: i64,
    /// Managed-money long.
    #[serde(rename = "managedMoneyLong", alias = "managed_money_long")]
    pub managed_money_long: i64,
    /// Managed-money short.
    #[serde(rename = "managedMoneyShort", alias = "managed_money_short")]
    pub managed_money_short: i64,
    /// Net managed-money position (long − short). Pre-computed
    /// by the seeder.
    #[serde(rename = "managedMoneyNet", alias = "managed_money_net")]
    pub managed_money_net: i64,
    /// Net managed-money as % of open interest. Computed by the
    /// handler so the panel doesn't have to. `0.0` when
    /// `openInterestAll` is `0`.
    #[serde(rename = "managedMoneyNetPctOi")]
    pub managed_money_net_pct_oi: f64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CotResponse {
    /// Rows after sort + limit clamp.
    pub rows: Vec<CotRow>,
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
pub struct CotQuery {
    /// Cap on rows. Defaults to [`DEFAULT_LIMIT`], clamped to
    /// [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Sort mode — wire string, parsed via [`SortMode::parse`].
    /// Defaults to `managed-money-net-desc` when absent;
    /// unknown strings produce a 400.
    #[serde(default)]
    pub sort: Option<String>,
    /// Comma-separated allow-list of CFTC contract codes.
    /// Filters the snapshot before the limit clamp.
    #[serde(default)]
    pub contracts: Option<String>,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Unknown `?sort` value.
    #[error("invalid sort mode: {0}")]
    InvalidSort(String),
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
            Self::InvalidSort(_) => "invalid_request",
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status.
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

/// Internal — minimal seeder row shape.
#[derive(Debug, Deserialize, Clone)]
struct SeederRow {
    contract_code: String,
    contract_name: String,
    report_date: String,
    open_interest_all: i64,
    producer_long: i64,
    producer_short: i64,
    swap_long: i64,
    swap_short: i64,
    managed_money_long: i64,
    managed_money_short: i64,
    managed_money_net: i64,
}

#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<SeederRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Project a seeder row into the wire shape, computing the
/// managed-money / open-interest ratio. Pure, exported for tests.
#[must_use]
pub fn project(row: SeederRowOwned) -> CotRow {
    let pct_oi = if row.open_interest_all == 0 {
        0.0
    } else {
        (row.managed_money_net as f64) / (row.open_interest_all as f64) * 100.0
    };
    CotRow {
        contract_code: row.contract_code,
        contract_name: row.contract_name,
        report_date: row.report_date,
        open_interest_all: row.open_interest_all,
        producer_long: row.producer_long,
        producer_short: row.producer_short,
        swap_long: row.swap_long,
        swap_short: row.swap_short,
        managed_money_long: row.managed_money_long,
        managed_money_short: row.managed_money_short,
        managed_money_net: row.managed_money_net,
        managed_money_net_pct_oi: pct_oi,
    }
}

/// Public mirror of the internal seeder row — exposed so tests
/// can build inputs without re-typing the snake_case fields.
#[derive(Clone, Debug)]
pub struct SeederRowOwned {
    /// CFTC contract market code.
    pub contract_code: String,
    /// Contract name.
    pub contract_name: String,
    /// Report date.
    pub report_date: String,
    /// Open interest.
    pub open_interest_all: i64,
    /// Producer long.
    pub producer_long: i64,
    /// Producer short.
    pub producer_short: i64,
    /// Swap long.
    pub swap_long: i64,
    /// Swap short.
    pub swap_short: i64,
    /// Managed-money long.
    pub managed_money_long: i64,
    /// Managed-money short.
    pub managed_money_short: i64,
    /// Pre-computed net managed-money position.
    pub managed_money_net: i64,
}

/// Apply the `?contracts` allow-list, sort, and `?limit` clamp.
/// Pure — exported so unit tests pin every boundary.
#[must_use]
pub fn apply_filters_and_sort(
    rows: Vec<CotRow>,
    sort: SortMode,
    contracts: Option<&str>,
    limit: usize,
) -> (Vec<CotRow>, usize) {
    let mut filtered: Vec<CotRow> = if let Some(raw) = contracts {
        let allow: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if allow.is_empty() {
            rows
        } else {
            rows.into_iter()
                .filter(|r| allow.iter().any(|a| a == &r.contract_code))
                .collect()
        }
    } else {
        rows
    };
    let total = filtered.len();
    filtered.sort_by(|a, b| match sort {
        SortMode::ManagedMoneyNetDesc => b.managed_money_net.cmp(&a.managed_money_net),
        SortMode::ManagedMoneyNetAsc => a.managed_money_net.cmp(&b.managed_money_net),
        SortMode::OpenInterestDesc => b.open_interest_all.cmp(&a.open_interest_all),
        SortMode::NameAsc => a.contract_name.cmp(&b.contract_name),
    });
    // Tie-breaker on contract code so the sort is stable across
    // upstream re-orderings (the SQLite read is order-preserving
    // but we don't want a flaky panel render on identical scores).
    filtered.sort_by(|a, b| {
        let primary = match sort {
            SortMode::ManagedMoneyNetDesc => b.managed_money_net.cmp(&a.managed_money_net),
            SortMode::ManagedMoneyNetAsc => a.managed_money_net.cmp(&b.managed_money_net),
            SortMode::OpenInterestDesc => b.open_interest_all.cmp(&a.open_interest_all),
            SortMode::NameAsc => a.contract_name.cmp(&b.contract_name),
        };
        match primary {
            Ordering::Equal => a.contract_code.cmp(&b.contract_code),
            other => other,
        }
    });
    let capped = limit.clamp(1, MAX_LIMIT);
    if filtered.len() > capped {
        filtered.truncate(capped);
    }
    (filtered, total)
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<CotQuery>,
) -> Result<Json<CotResponse>, HandlerError> {
    let sort = match q.sort.as_deref() {
        None => SortMode::ManagedMoneyNetDesc,
        Some(raw) => SortMode::parse(raw)
            .ok_or_else(|| HandlerError::InvalidSort(raw.to_string()))?,
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
    let payload: SnapshotPayload = serde_json::from_value(inner)
        .map_err(|e| HandlerError::Shape(e.to_string()))?;

    let projected: Vec<CotRow> = payload
        .rows
        .into_iter()
        .map(|r| {
            project(SeederRowOwned {
                contract_code: r.contract_code,
                contract_name: r.contract_name,
                report_date: r.report_date,
                open_interest_all: r.open_interest_all,
                producer_long: r.producer_long,
                producer_short: r.producer_short,
                swap_long: r.swap_long,
                swap_short: r.swap_short,
                managed_money_long: r.managed_money_long,
                managed_money_short: r.managed_money_short,
                managed_money_net: r.managed_money_net,
            })
        })
        .collect();

    let (rows, total) = apply_filters_and_sort(
        projected,
        sort,
        q.contracts.as_deref(),
        q.limit.unwrap_or(DEFAULT_LIMIT),
    );

    Ok(Json(CotResponse {
        rows,
        total,
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
    use crate::market::v1::COT_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn snapshot_value(rows: &[(&str, &str, i64, i64, i64)]) -> Value {
        serde_json::json!({
            "rows": rows.iter().map(|(code, name, oi, mm_long, mm_short)| serde_json::json!({
                "contract_code": code,
                "contract_name": name,
                "report_date": "2026-04-29",
                "open_interest_all": oi,
                "producer_long": 100_000_i64,
                "producer_short": 120_000_i64,
                "swap_long": 80_000_i64,
                "swap_short": 90_000_i64,
                "managed_money_long": mm_long,
                "managed_money_short": mm_short,
                "managed_money_net": mm_long - mm_short,
            })).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            COT_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn cache_key_pinned_to_seeder_slot() {
        assert_eq!(CACHE_KEY, "market:cot-report:weekly:v1");
    }

    #[test]
    fn sort_mode_parse_accepts_only_known_strings() {
        assert_eq!(
            SortMode::parse("managed-money-net-desc"),
            Some(SortMode::ManagedMoneyNetDesc)
        );
        assert_eq!(
            SortMode::parse("managed-money-net-asc"),
            Some(SortMode::ManagedMoneyNetAsc)
        );
        assert_eq!(
            SortMode::parse("open-interest-desc"),
            Some(SortMode::OpenInterestDesc)
        );
        assert_eq!(SortMode::parse("name-asc"), Some(SortMode::NameAsc));
        assert_eq!(SortMode::parse("nonsense"), None);
    }

    #[test]
    fn project_computes_pct_of_oi_and_zero_when_oi_is_zero() {
        let row = SeederRowOwned {
            contract_code: "088691".into(),
            contract_name: "GOLD".into(),
            report_date: "2026-04-29".into(),
            open_interest_all: 480_000,
            producer_long: 100_000,
            producer_short: 120_000,
            swap_long: 80_000,
            swap_short: 90_000,
            managed_money_long: 120_000,
            managed_money_short: 60_000,
            managed_money_net: 60_000,
        };
        let projected = project(row);
        // 60_000 / 480_000 * 100 = 12.5
        assert!((projected.managed_money_net_pct_oi - 12.5).abs() < 1e-9);

        let zero_oi = SeederRowOwned {
            contract_code: "x".into(),
            contract_name: "X".into(),
            report_date: "2026-04-29".into(),
            open_interest_all: 0,
            producer_long: 0,
            producer_short: 0,
            swap_long: 0,
            swap_short: 0,
            managed_money_long: 0,
            managed_money_short: 0,
            managed_money_net: 0,
        };
        assert_eq!(project(zero_oi).managed_money_net_pct_oi, 0.0);
    }

    fn row(code: &str, name: &str, mm_net: i64, oi: i64) -> CotRow {
        CotRow {
            contract_code: code.into(),
            contract_name: name.into(),
            report_date: "2026-04-29".into(),
            open_interest_all: oi,
            producer_long: 0,
            producer_short: 0,
            swap_long: 0,
            swap_short: 0,
            managed_money_long: mm_net.max(0),
            managed_money_short: (-mm_net).max(0),
            managed_money_net: mm_net,
            managed_money_net_pct_oi: if oi > 0 {
                (mm_net as f64) / (oi as f64) * 100.0
            } else {
                0.0
            },
        }
    }

    #[test]
    fn apply_filters_and_sort_managed_money_net_desc() {
        let rows = vec![
            row("a", "A", 50, 100),
            row("b", "B", 100, 100),
            row("c", "C", -25, 100),
        ];
        let (out, total) = apply_filters_and_sort(
            rows,
            SortMode::ManagedMoneyNetDesc,
            None,
            10,
        );
        let codes: Vec<&str> = out.iter().map(|r| r.contract_code.as_str()).collect();
        assert_eq!(codes, vec!["b", "a", "c"]);
        assert_eq!(total, 3);
    }

    #[test]
    fn apply_filters_and_sort_managed_money_net_asc() {
        let rows = vec![
            row("a", "A", 50, 100),
            row("b", "B", 100, 100),
            row("c", "C", -25, 100),
        ];
        let (out, _) = apply_filters_and_sort(
            rows,
            SortMode::ManagedMoneyNetAsc,
            None,
            10,
        );
        let codes: Vec<&str> = out.iter().map(|r| r.contract_code.as_str()).collect();
        assert_eq!(codes, vec!["c", "a", "b"]);
    }

    #[test]
    fn apply_filters_and_sort_open_interest_desc() {
        let rows = vec![
            row("a", "A", 0, 100),
            row("b", "B", 0, 500),
            row("c", "C", 0, 250),
        ];
        let (out, _) = apply_filters_and_sort(
            rows,
            SortMode::OpenInterestDesc,
            None,
            10,
        );
        let codes: Vec<&str> = out.iter().map(|r| r.contract_code.as_str()).collect();
        assert_eq!(codes, vec!["b", "c", "a"]);
    }

    #[test]
    fn apply_filters_and_sort_name_asc() {
        let rows = vec![
            row("a", "BBB", 0, 0),
            row("b", "AAA", 0, 0),
            row("c", "CCC", 0, 0),
        ];
        let (out, _) = apply_filters_and_sort(
            rows,
            SortMode::NameAsc,
            None,
            10,
        );
        let names: Vec<&str> = out.iter().map(|r| r.contract_name.as_str()).collect();
        assert_eq!(names, vec!["AAA", "BBB", "CCC"]);
    }

    #[test]
    fn apply_filters_and_sort_contract_allowlist_csv() {
        let rows = vec![
            row("aaa", "A", 0, 0),
            row("bbb", "B", 0, 0),
            row("ccc", "C", 0, 0),
        ];
        let (out, total) = apply_filters_and_sort(
            rows,
            SortMode::NameAsc,
            Some("aaa,ccc"),
            10,
        );
        let codes: Vec<&str> = out.iter().map(|r| r.contract_code.as_str()).collect();
        assert_eq!(codes, vec!["aaa", "ccc"]);
        assert_eq!(total, 2);
    }

    #[test]
    fn apply_filters_and_sort_clamps_limit_to_max() {
        let rows: Vec<CotRow> = (0..(MAX_LIMIT + 30))
            .map(|i| row(&format!("c{i}"), &format!("C{i}"), i as i64, 0))
            .collect();
        let (out, total) = apply_filters_and_sort(
            rows,
            SortMode::NameAsc,
            None,
            MAX_LIMIT * 5,
        );
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 30);
    }

    #[test]
    fn apply_filters_and_sort_zero_limit_floors_to_one() {
        let rows = vec![row("a", "A", 0, 0), row("b", "B", 0, 0)];
        let (out, _) = apply_filters_and_sort(
            rows,
            SortMode::NameAsc,
            None,
            0,
        );
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn handler_returns_400_for_invalid_sort() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COT_PATH}?sort=nonsense"))
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
                    .uri(COT_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_envelope_with_camelcase_field_names() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[
            ("088691", "GOLD", 480_000, 120_000, 60_000),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(COT_PATH)
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
        assert!(parsed.pointer("/rows/0/contractCode").is_some());
        assert!(parsed.pointer("/rows/0/managedMoneyNet").is_some());
        assert!(parsed.pointer("/rows/0/managedMoneyNetPctOi").is_some());
        assert_eq!(
            parsed.pointer("/total").and_then(Value::as_u64),
            Some(1),
        );
    }

    #[tokio::test]
    async fn handler_filters_by_contracts_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[
            ("088691", "GOLD", 480_000, 120_000, 60_000),
            ("084691", "SILVER", 200_000, 50_000, 30_000),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COT_PATH}?contracts=088691"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CotResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0].contract_code, "088691");
        assert_eq!(parsed.total, 1);
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot_value(&[
            ("088691", "GOLD", 480_000, 120_000, 60_000),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(COT_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CotResponse = serde_json::from_slice(&body).unwrap();
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
                    .uri(COT_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

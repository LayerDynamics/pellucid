//! `GET /api/telegram/v1/feed` handler.
//!
//! Pure cache reader for the snapshot
//! `seed_telegram_intel_min` writes to
//! `telegram:recent-feed:v1` (FAST_KEYS slot). The webview's
//! `TelegramIntelPanel` (T4.1.5) consumes the response.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "rows": [{
//!     "channel": "rt_intl_news",
//!     "dataPost": "rt_intl_news/12345",
//!     "url": "https://t.me/rt_intl_news/12345",
//!     "datetime": "2026-05-04T12:00:00Z",
//!     "text": "…",
//!     "views": "12.4K"
//!   }],
//!   "channels": ["rt_intl_news", "…"],
//!   "assembledAtMs": 1746360000000,
//!   "total": 75,
//!   "stale": false
//! }
//! ```
//!
//! ## M4 outage path
//!
//! Same envelope as `news/v1/list_articles` and
//! `intelligence/v1/gdelt-feed`: 503 + `Retry-After` +
//! `bootstrap_upstream_empty`.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — pinned to the seeder slot.
pub const CACHE_KEY: &str = "telegram:recent-feed:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Hard cap on `?limit=`.
pub const MAX_LIMIT: usize = 200;

/// Default `?limit` when the client doesn't ask.
pub const DEFAULT_LIMIT: usize = 50;

/// One row in the wire response. Field renames give the wire
/// camelCase while the deserializer accepts the seeder's
/// snake_case (`data_post`) via `#[serde(alias = "…")]`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TelegramMessage {
    /// Channel slug (without leading @).
    pub channel: String,
    /// `<channel>/<id>` data-post identifier.
    #[serde(rename = "dataPost", alias = "data_post")]
    pub data_post: String,
    /// Permalink.
    pub url: String,
    /// ISO-8601 timestamp from the upstream.
    pub datetime: String,
    /// Plain-text message body.
    pub text: String,
    /// View counter as the upstream rendered it (`"12.4K"` etc.).
    pub views: String,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedResponse {
    /// Message rows — clamped to `min(?limit, MAX_LIMIT)`.
    pub rows: Vec<TelegramMessage>,
    /// Echo of the seeder's channel basket. The panel renders
    /// these as a basket-summary chip row.
    pub channels: Vec<String>,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// Total rows before the limit clamp.
    pub total: usize,
    /// Whether the response was synthesised from a stale cache row.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct FeedQuery {
    /// Cap on rows. Defaults to [`DEFAULT_LIMIT`], clamped to
    /// [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Filter to a single channel slug (case-insensitive,
    /// trim-tolerant). Useful when the panel narrows to one feed.
    #[serde(default)]
    pub channel: Option<String>,
}

/// Internal — snapshot shape stored by the seeder. Deserializer
/// accepts both snake and camel for forward-compat.
#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<TelegramMessage>,
    channels: Vec<String>,
    #[serde(rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Errors the handler can produce. Codes match the news + intel
/// domain so the loader's discriminator stays uniform.
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

/// Apply `?limit` + `?channel` filters. Pure.
#[must_use]
pub fn apply_filters(rows: Vec<TelegramMessage>, q: &FeedQuery) -> (Vec<TelegramMessage>, usize) {
    let mut filtered: Vec<TelegramMessage> = if let Some(channel) = q.channel.as_deref() {
        let needle = channel.trim().to_ascii_lowercase();
        rows.into_iter()
            .filter(|r| r.channel.to_ascii_lowercase() == needle)
            .collect()
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

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<FeedQuery>,
) -> Result<Json<FeedResponse>, HandlerError> {
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
    let (rows, total) = apply_filters(payload.rows, &q);
    Ok(Json(FeedResponse {
        rows,
        channels: payload.channels,
        assembled_at_ms: payload.assembled_at_ms,
        total,
        stale,
    }))
}

/// Same envelope-unwrap helper used by the news + intel handlers.
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
    use crate::telegram::v1::FEED_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn message(channel: &str, id: u64) -> TelegramMessage {
        TelegramMessage {
            channel: channel.into(),
            data_post: format!("{channel}/{id}"),
            url: format!("https://t.me/{channel}/{id}"),
            datetime: "2026-05-04T12:00:00Z".into(),
            text: format!("body-{channel}-{id}"),
            views: "1.2K".into(),
        }
    }

    fn snapshot(rows: Vec<TelegramMessage>) -> Value {
        serde_json::json!({
            "rows": rows.iter().map(|r| serde_json::json!({
                "channel": r.channel,
                "data_post": r.data_post,
                "url": r.url,
                "datetime": r.datetime,
                "text": r.text,
                "views": r.views,
            })).collect::<Vec<_>>(),
            "channels": ["rt_intl_news", "isw_warstudies"],
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(FEED_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[test]
    fn cache_key_pinned_to_seeder_slot() {
        assert_eq!(CACHE_KEY, "telegram:recent-feed:v1");
    }

    #[test]
    fn apply_filters_default_limit_is_fifty() {
        let rows: Vec<TelegramMessage> = (0..120).map(|i| message("rt_intl_news", i)).collect();
        let (out, total) = apply_filters(rows, &FeedQuery::default());
        assert_eq!(out.len(), DEFAULT_LIMIT);
        assert_eq!(total, 120);
    }

    #[test]
    fn apply_filters_clamps_limit_to_max() {
        let rows: Vec<TelegramMessage> = (0..(MAX_LIMIT + 30))
            .map(|i| message("rt_intl_news", i as u64))
            .collect();
        let q = FeedQuery {
            limit: Some(MAX_LIMIT * 5),
            channel: None,
        };
        let (out, total) = apply_filters(rows, &q);
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 30);
    }

    #[test]
    fn apply_filters_zero_limit_floors_to_one() {
        let rows = vec![message("rt_intl_news", 1), message("rt_intl_news", 2)];
        let q = FeedQuery {
            limit: Some(0),
            channel: None,
        };
        let (out, _) = apply_filters(rows, &q);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn apply_filters_channel_is_case_insensitive_and_trim_tolerant() {
        let rows = vec![
            message("rt_intl_news", 1),
            message("RT_INTL_NEWS", 2),
            message("isw_warstudies", 3),
        ];
        let q = FeedQuery {
            limit: None,
            channel: Some("  RT_intl_news  ".into()),
        };
        let (out, total) = apply_filters(rows, &q);
        assert_eq!(out.len(), 2);
        assert_eq!(total, 2);
    }

    #[test]
    fn handler_error_codes_match_other_handlers() {
        assert_eq!(HandlerError::Cache("x".into()).code(), "cache_failure");
        assert_eq!(HandlerError::Shape("x".into()).code(), "cache_shape");
        assert_eq!(
            HandlerError::Outage {
                retry_after_secs: 30
            }
            .code(),
            "bootstrap_upstream_empty",
        );
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_returns_rows_with_camel_case_data_post() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![message("rt_intl_news", 1)]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEED_PATH)
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
        // camelCase on the wire.
        assert!(parsed.pointer("/rows/0/dataPost").is_some());
        assert_eq!(
            parsed.pointer("/assembledAtMs").and_then(Value::as_i64),
            Some(1_700_000_000_000),
        );
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(1));
        let channels = parsed
            .pointer("/channels")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(channels.len(), 2);
    }

    #[tokio::test]
    async fn handler_filters_by_channel_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            message("rt_intl_news", 1),
            message("isw_warstudies", 2),
            message("rt_intl_news", 3),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{FEED_PATH}?channel=rt_intl_news"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: FeedResponse = serde_json::from_slice(&body).unwrap();
        let channels: Vec<&str> = parsed.rows.iter().map(|r| r.channel.as_str()).collect();
        assert_eq!(channels, vec!["rt_intl_news", "rt_intl_news"]);
        assert_eq!(parsed.total, 2);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = serde_json::json!({ "rows": "not-an-array" });
        let env = Envelope::new(bad);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEED_PATH)
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
        let env = Envelope::new(snapshot(vec![message("rt_intl_news", 1)]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: FeedResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }
}

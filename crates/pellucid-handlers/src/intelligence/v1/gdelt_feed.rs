//! `GET /api/intelligence/v1/gdelt-feed` handler.
//!
//! Pure cache reader for the snapshot the
//! `seed_gdelt_intel` seeder publishes to
//! `conflict:incident-feed:v1` (already in `FAST_KEYS`). The
//! webview's `GdeltIntelPanel` (T4.1.4) consumes the response.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "rows": [{
//!     "url": "https://…",
//!     "title": "…",
//!     "seenDate": "20260504T120000Z",
//!     "socialImage": "https://…",
//!     "domain": "example.com",
//!     "language": "English",
//!     "sourceCountry": "Iran"
//!   }],
//!   "query": "(theme:KILL OR …)",
//!   "timespan": "24h",
//!   "assembledAtMs": 1746360000000,
//!   "total": 75,
//!   "stale": false
//! }
//! ```
//!
//! Field names use camelCase on the wire to match the rest of
//! the v1 surface (`publishedAtMs`, `severityFloor`, …).
//!
//! ## M4 outage path
//!
//! Same envelope as `news/v1/list_articles`: 503 + `Retry-After`
//! + `bootstrap_upstream_empty`. The webview's intel-family
//!   outage banner sizes its retry policy off the same shape.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key the handler reads. Pinned to the
/// `seed_gdelt_intel` seeder's slot so a key bump on the seeder
/// side surfaces here as a compile-time string mismatch.
pub const CACHE_KEY: &str = "conflict:incident-feed:v1";

/// SPEC-001 §11.2 default Retry-After for the M4 outage path.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Hard cap on `?limit=`. The seeder caps at 75 records per
/// cycle so a 200 ceiling still gives the client room without
/// the wire response ballooning.
pub const MAX_LIMIT: usize = 200;

/// Default `?limit` when the client doesn't ask. 50 keeps the
/// initial render budget under ~25 KiB.
pub const DEFAULT_LIMIT: usize = 50;

/// One row in the wire response. Mirrors `GdeltArticleRow` in
/// `crates/pellucid-seeders/src/conflict/seed_gdelt_intel.rs`,
/// renaming fields to camelCase via `#[serde(rename = "…")]`
/// so the wire matches the rest of the v1 surface without
/// changing the seeder's storage shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GdeltArticle {
    /// Article URL.
    pub url: String,
    /// Headline.
    pub title: String,
    /// `YYYYMMDDTHHMMSSZ` timestamp from GDELT.
    #[serde(rename = "seenDate", alias = "seen_date")]
    pub seen_date: String,
    /// Social-share image URL.
    #[serde(rename = "socialImage", alias = "social_image")]
    pub social_image: String,
    /// Source domain.
    pub domain: String,
    /// Reported article language.
    pub language: String,
    /// Source country (GDELT-reported).
    #[serde(rename = "sourceCountry", alias = "source_country")]
    pub source_country: String,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GdeltFeedResponse {
    /// Article rows — clamped to `min(?limit, MAX_LIMIT)`.
    pub rows: Vec<GdeltArticle>,
    /// Echo of the query string baked into the snapshot.
    pub query: String,
    /// Echo of the timespan baked into the snapshot.
    pub timespan: String,
    /// Wall-clock ms when the seeder assembled the snapshot.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// Total rows in the cache slot before the limit clamp.
    pub total: usize,
    /// Whether the response was synthesised from a stale cache row.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct GdeltFeedQuery {
    /// Cap on rows returned. Defaults to [`DEFAULT_LIMIT`],
    /// clamped to [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Filter rows whose `source_country` (case-insensitive)
    /// matches this string. Useful for the `CountryDeepDivePanel`
    /// (T4.1.7) when it asks for country-scoped intel.
    #[serde(default)]
    pub country: Option<String>,
}

/// Internal — the snapshot shape the seeder writes inside the
/// `SeedEnvelope.data` field. Separate from
/// [`GdeltFeedResponse`] because it owns the raw seeder field
/// names (snake_case) — we re-pack into the wire shape after
/// applying filters.
#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<GdeltArticle>,
    query: String,
    timespan: String,
    #[serde(rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Errors the handler can produce. Codes match the news domain
/// so the loader's discriminator stays uniform across the v1
/// surface.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failure.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but body did not deserialise.
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

    /// HTTP status mapping.
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

/// Apply `?limit` + `?country` filters. Pure — extracted so
/// unit tests pin the boundaries without touching the cache.
#[must_use]
pub fn apply_filters(
    rows: Vec<GdeltArticle>,
    q: &GdeltFeedQuery,
) -> (Vec<GdeltArticle>, usize) {
    let mut filtered: Vec<GdeltArticle> = if let Some(country) = q.country.as_deref() {
        let needle = country.trim().to_ascii_lowercase();
        rows.into_iter()
            .filter(|r| r.source_country.to_ascii_lowercase() == needle)
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
    Query(q): Query<GdeltFeedQuery>,
) -> Result<Json<GdeltFeedResponse>, HandlerError> {
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
    let (rows, total) = apply_filters(payload.rows, &q);
    Ok(Json(GdeltFeedResponse {
        rows,
        query: payload.query,
        timespan: payload.timespan,
        assembled_at_ms: payload.assembled_at_ms,
        total,
        stale,
    }))
}

/// Same envelope-unwrap helper used by `news::v1::list_articles`.
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
    use crate::intelligence::v1::GDELT_FEED_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use axum::response::IntoResponse;
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn row(country: &str, url: &str) -> GdeltArticle {
        GdeltArticle {
            url: url.into(),
            title: format!("title-{url}"),
            seen_date: "20260504T120000Z".into(),
            social_image: String::new(),
            domain: "example.com".into(),
            language: "English".into(),
            source_country: country.into(),
        }
    }

    fn snapshot(rows: Vec<GdeltArticle>) -> Value {
        // Match the seeder's storage shape (snake_case) — the
        // SnapshotPayload deserializer accepts both via aliases.
        serde_json::json!({
            "rows": rows.iter().map(|r| serde_json::json!({
                "url": r.url,
                "title": r.title,
                "seen_date": r.seen_date,
                "social_image": r.social_image,
                "domain": r.domain,
                "language": r.language,
                "source_country": r.source_country,
            })).collect::<Vec<_>>(),
            "query": "(theme:KILL)",
            "timespan": "24h",
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            GDELT_FEED_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn cache_key_pinned_to_seeder_slot() {
        // If the seeder bumps its key, this assertion + the
        // FAST_KEYS entry must move together.
        assert_eq!(CACHE_KEY, "conflict:incident-feed:v1");
    }

    #[test]
    fn apply_filters_default_limit_is_fifty() {
        let rows: Vec<GdeltArticle> =
            (0..120).map(|i| row("Iran", &format!("u{i}"))).collect();
        let (out, total) = apply_filters(rows, &GdeltFeedQuery::default());
        assert_eq!(out.len(), DEFAULT_LIMIT);
        assert_eq!(total, 120);
    }

    #[test]
    fn apply_filters_clamps_limit_to_max() {
        let rows: Vec<GdeltArticle> =
            (0..(MAX_LIMIT + 30)).map(|i| row("Iran", &format!("u{i}"))).collect();
        let q = GdeltFeedQuery {
            limit: Some(MAX_LIMIT * 5),
            country: None,
        };
        let (out, total) = apply_filters(rows, &q);
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 30);
    }

    #[test]
    fn apply_filters_zero_limit_floors_to_one() {
        let rows = vec![row("Iran", "a"), row("Iran", "b")];
        let q = GdeltFeedQuery {
            limit: Some(0),
            country: None,
        };
        let (out, _) = apply_filters(rows, &q);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn apply_filters_country_is_case_insensitive_and_trim_tolerant() {
        let rows = vec![
            row("Iran", "a"),
            row("iran", "b"),
            row("Iraq", "c"),
            row("Saudi Arabia", "d"),
        ];
        let q = GdeltFeedQuery {
            limit: None,
            country: Some("  IRAN  ".into()),
        };
        let (out, total) = apply_filters(rows, &q);
        let urls: Vec<&str> = out.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(urls, vec!["a", "b"]);
        assert_eq!(total, 2);
    }

    #[test]
    fn handler_error_codes_match_news_domain() {
        assert_eq!(HandlerError::Cache("x".into()).code(), "cache_failure");
        assert_eq!(HandlerError::Shape("x".into()).code(), "cache_shape");
        assert_eq!(
            HandlerError::Outage { retry_after_secs: 30 }.code(),
            "bootstrap_upstream_empty",
        );
    }

    #[test]
    fn handler_error_status_codes_match_news_domain() {
        assert_eq!(HandlerError::Cache("x".into()).status(), Code::BAD_GATEWAY);
        assert_eq!(HandlerError::Shape("x".into()).status(), Code::BAD_GATEWAY);
        assert_eq!(
            HandlerError::Outage { retry_after_secs: 30 }.status(),
            Code::SERVICE_UNAVAILABLE,
        );
    }

    #[tokio::test]
    async fn outage_response_carries_retry_after_header() {
        let resp = HandlerError::Outage { retry_after_secs: 30 }.into_response();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
        assert_eq!(
            resp.headers().get(GATEWAY_ERROR_CODE_HEADER).unwrap(),
            "bootstrap_upstream_empty",
        );
    }

    #[test]
    fn unwrap_envelope_data_strips_seed_wrapper() {
        let envelope = serde_json::json!({
            "_seed": { "fetched_at_ms": 1 },
            "data":  { "rows": [] }
        });
        let inner = unwrap_envelope_data(envelope);
        assert_eq!(inner, serde_json::json!({ "rows": [] }));
    }

    #[test]
    fn unwrap_envelope_data_passes_through_raw() {
        let raw = serde_json::json!({ "rows": [] });
        assert_eq!(unwrap_envelope_data(raw.clone()), raw);
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GDELT_FEED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_returns_rows_with_camel_case_field_names() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![row("Iran", "a")]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GDELT_FEED_PATH)
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
        // camelCase on the wire — these are the field names the
        // GdeltIntelPanel reads.
        assert!(parsed.pointer("/rows/0/seenDate").is_some());
        assert!(parsed.pointer("/rows/0/socialImage").is_some());
        assert!(parsed.pointer("/rows/0/sourceCountry").is_some());
        assert_eq!(
            parsed.pointer("/assembledAtMs").and_then(Value::as_i64),
            Some(1_700_000_000_000),
        );
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(1));
        assert_eq!(parsed.pointer("/stale").and_then(Value::as_bool), Some(false));
    }

    #[tokio::test]
    async fn handler_filters_by_country_query_param() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            row("Iran", "a"),
            row("Iraq", "b"),
            row("iran", "c"),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{GDELT_FEED_PATH}?country=Iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GdeltFeedResponse = serde_json::from_slice(&body).unwrap();
        let urls: Vec<&str> = parsed.rows.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(urls, vec!["a", "c"]);
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
                    .uri(GDELT_FEED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/error/code").and_then(Value::as_str),
            Some("cache_shape"),
        );
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![row("Iran", "a")]));
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GDELT_FEED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GdeltFeedResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_clamps_limit_via_query_param() {
        let (app, pool) = migrated_router().await;
        let rows: Vec<GdeltArticle> =
            (0..10).map(|i| row("Iran", &format!("u{i}"))).collect();
        let env = Envelope::new(snapshot(rows));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{GDELT_FEED_PATH}?limit=3"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GdeltFeedResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.rows.len(), 3);
        assert_eq!(parsed.total, 10);
    }
}

//! `GET /api/news/v1/get-breaking` handler.
//!
//! Returns the highest-severity articles from the same news cache
//! key that `list_articles` reads. Drives the
//! [`BreakingNewsBanner`] panel (T4.1.3): the banner polls this
//! endpoint, opens a Radix Toast for every article it has not
//! seen before, and drops the toast on auto-dismiss.
//!
//! ## Why a separate endpoint and not a query parameter on `list-articles`
//!
//! - **Different default cap.** The banner only shows a handful
//!   of items (3–5). The list panel defaults to 50.
//! - **Different default severity floor.** `list-articles`
//!   defaults to no floor (everything passes). The banner only
//!   considers `high` and `critical` by default — info / warn
//!   are noise in a toast.
//! - **`?since_ms` filter.** A polling client wants to skip
//!   articles it already rendered. Putting that on `list-articles`
//!   would muddy the wire shape; here it's a first-class knob.
//!
//! ## Wire shape
//!
//! Response is identical to `list-articles` so the panel can
//! consume the same `NewsArticle` rows without reshaping.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::news::v1::list_articles::{
    ListArticlesPayload, NewsArticle, Severity, CACHE_KEY, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;

/// Default `?limit` when the client doesn't ask. 5 keeps the
/// toast queue small enough that the user can read each item
/// before the next auto-dismiss tick.
pub const DEFAULT_LIMIT: usize = 5;

/// Hard cap on `?limit=`. Anything larger is silently clamped —
/// the banner is for triage, not for paginating an archive.
pub const MAX_LIMIT: usize = 20;

/// Default severity floor when `?severity=` is omitted. `High`
/// excludes `info` + `warn` so only actionable items toast.
pub const DEFAULT_SEVERITY_FLOOR: Severity = Severity::High;

/// Wire-format response. Same shape as
/// `news::v1::list_articles::ListArticlesResponse` so the panel
/// shares the response type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetBreakingResponse {
    /// Articles at or above the severity floor, sorted descending
    /// by `published_at_ms`, capped at `min(?limit, MAX_LIMIT)`.
    pub articles: Vec<NewsArticle>,
    /// Total candidates that passed the severity + `since_ms`
    /// filters, before the limit clamp. The banner uses this to
    /// decide whether to render a "+N more" affordance.
    pub total: usize,
    /// Whether the underlying cache row was stale.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Deserialize, Default)]
pub struct GetBreakingQuery {
    /// Cap on the number of articles returned. Defaults to
    /// [`DEFAULT_LIMIT`], clamped to [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Severity floor. Defaults to [`DEFAULT_SEVERITY_FLOOR`].
    /// Articles below the floor are filtered out.
    #[serde(default)]
    pub severity: Option<Severity>,
    /// Wall-clock ms cutoff. When set, articles whose
    /// `published_at_ms` is `<= since_ms` are filtered out — lets
    /// a polling client skip toasts it already rendered.
    #[serde(default, rename = "sinceMs")]
    pub since_ms: Option<i64>,
}

/// Errors the handler can produce. Same code surface as
/// `list_articles::HandlerError` so the loader's discriminator
/// stays stable across the two endpoints.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// M4 outage path: no fresh, no stale, no negative-sentinel.
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value emitted with the 503.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    /// Stable error code — matches `list_articles::HandlerError`.
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

/// Apply the breaking-banner filters to a raw payload. Pure —
/// extracted so unit tests pin every boundary without touching
/// the cache layer.
///
/// Returns `(articles, total)` where `total` is the count of
/// items that passed the severity + `since_ms` filters before the
/// limit clamp (so the banner can render a "+N more" affordance).
#[must_use]
pub fn apply_breaking_filters(
    payload: ListArticlesPayload,
    q: &GetBreakingQuery,
) -> (Vec<NewsArticle>, usize) {
    let floor = q.severity.clone().unwrap_or(DEFAULT_SEVERITY_FLOOR);
    let floor_rank = severity_rank(&floor);
    let since_cutoff = q.since_ms;

    let mut filtered: Vec<NewsArticle> = payload
        .articles
        .into_iter()
        .filter(|a| {
            // Severity: row must carry a tag and meet the floor.
            let sev_ok = a
                .severity
                .as_ref()
                .map(|s| severity_rank(s) >= floor_rank)
                .unwrap_or(false);
            // since_ms: strictly newer than the cutoff.
            let time_ok = match since_cutoff {
                Some(cutoff) => a.published_at_ms > cutoff,
                None => true,
            };
            sev_ok && time_ok
        })
        .collect();

    // Sort descending by published_at_ms so the banner shows the
    // freshest signal first regardless of upstream ordering.
    filtered.sort_by(|a, b| b.published_at_ms.cmp(&a.published_at_ms));

    let total = filtered.len();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    if filtered.len() > limit {
        filtered.truncate(limit);
    }
    (filtered, total)
}

const fn severity_rank(s: &Severity) -> u8 {
    match s {
        Severity::Info => 0,
        Severity::Warn => 1,
        Severity::High => 2,
        Severity::Critical => 3,
    }
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<GetBreakingQuery>,
) -> Result<Json<GetBreakingResponse>, HandlerError> {
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
    let payload: ListArticlesPayload = serde_json::from_value(inner)
        .map_err(|e| HandlerError::Shape(e.to_string()))?;
    let (articles, total) = apply_breaking_filters(payload, &q);
    Ok(Json(GetBreakingResponse {
        articles,
        total,
        stale,
    }))
}

/// Same envelope-unwrap helper used by `list_articles` —
/// duplicated here to keep the handler module self-contained
/// (the helper in `list_articles.rs` is private).
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
    use crate::news::v1::GET_BREAKING_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use axum::response::IntoResponse;
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn article(id: &str, sev: Option<Severity>, ts: i64) -> NewsArticle {
        NewsArticle {
            id: id.into(),
            title: format!("title-{id}"),
            source: "src".into(),
            published_at_ms: ts,
            url: Some(format!("https://example.com/{id}")),
            summary: None,
            severity: sev,
        }
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            GET_BREAKING_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn default_severity_floor_is_high() {
        // The default floor matches the documented public constant.
        assert!(matches!(DEFAULT_SEVERITY_FLOOR, Severity::High));
    }

    #[test]
    fn cache_key_is_shared_with_list_articles() {
        // The breaking handler MUST read the same key the seeder
        // populates; drift would silently strand the banner.
        assert_eq!(CACHE_KEY, "news:articles:list:v1");
    }

    #[test]
    fn apply_filters_default_floor_excludes_info_and_warn() {
        let payload = ListArticlesPayload {
            articles: vec![
                article("low", Some(Severity::Info), 1),
                article("mid", Some(Severity::Warn), 2),
                article("hi", Some(Severity::High), 3),
                article("crit", Some(Severity::Critical), 4),
                article("none", None, 5),
            ],
        };
        let (out, total) = apply_breaking_filters(payload, &GetBreakingQuery::default());
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        // High + Critical pass; everything else (incl. unsevered) drops.
        assert_eq!(ids, vec!["crit", "hi"]);
        assert_eq!(total, 2);
    }

    #[test]
    fn apply_filters_custom_severity_floor_passes_lower_levels() {
        let payload = ListArticlesPayload {
            articles: vec![
                article("low", Some(Severity::Info), 1),
                article("mid", Some(Severity::Warn), 2),
                article("hi", Some(Severity::High), 3),
            ],
        };
        let q = GetBreakingQuery {
            severity: Some(Severity::Warn),
            ..Default::default()
        };
        let (out, total) = apply_breaking_filters(payload, &q);
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["hi", "mid"]);
        assert_eq!(total, 2);
    }

    #[test]
    fn apply_filters_since_ms_drops_older_articles() {
        let payload = ListArticlesPayload {
            articles: vec![
                article("old", Some(Severity::Critical), 100),
                article("eq", Some(Severity::Critical), 200),
                article("new", Some(Severity::Critical), 300),
            ],
        };
        let q = GetBreakingQuery {
            since_ms: Some(200),
            ..Default::default()
        };
        let (out, total) = apply_breaking_filters(payload, &q);
        // Strictly greater than: 200 is excluded (already-seen
        // boundary).
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["new"]);
        assert_eq!(total, 1);
    }

    #[test]
    fn apply_filters_sorts_descending_by_published_at_ms() {
        let payload = ListArticlesPayload {
            articles: vec![
                article("a", Some(Severity::Critical), 100),
                article("c", Some(Severity::Critical), 300),
                article("b", Some(Severity::Critical), 200),
            ],
        };
        let (out, _) = apply_breaking_filters(payload, &GetBreakingQuery::default());
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
    }

    #[test]
    fn apply_filters_clamps_limit_to_max_and_floor_at_one() {
        let payload = ListArticlesPayload {
            articles: (0..(MAX_LIMIT + 30))
                .map(|i| article(&format!("a{i}"), Some(Severity::Critical), i as i64))
                .collect(),
        };
        // Over-asking is clamped down.
        let q = GetBreakingQuery {
            limit: Some(MAX_LIMIT * 5),
            ..Default::default()
        };
        let (out, total) = apply_breaking_filters(payload.clone(), &q);
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 30);

        // Zero is bumped to a 1-item floor (banner still shows
        // something rather than a silent no-op).
        let q0 = GetBreakingQuery {
            limit: Some(0),
            ..Default::default()
        };
        let (out0, _) = apply_breaking_filters(payload, &q0);
        assert_eq!(out0.len(), 1);
    }

    #[test]
    fn apply_filters_default_limit_is_five() {
        let payload = ListArticlesPayload {
            articles: (0..50)
                .map(|i| article(&format!("a{i}"), Some(Severity::Critical), i as i64))
                .collect(),
        };
        let (out, total) = apply_breaking_filters(payload, &GetBreakingQuery::default());
        assert_eq!(out.len(), DEFAULT_LIMIT);
        assert_eq!(total, 50);
    }

    #[test]
    fn handler_error_codes_match_list_articles() {
        // Stable codes — the loader branches on these without
        // remapping per endpoint.
        assert_eq!(HandlerError::Cache("x".into()).code(), "cache_failure");
        assert_eq!(HandlerError::Shape("x".into()).code(), "cache_shape");
        assert_eq!(
            HandlerError::Outage { retry_after_secs: 30 }.code(),
            "bootstrap_upstream_empty",
        );
    }

    #[test]
    fn handler_error_status_codes_match_list_articles() {
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
            "data":  { "articles": [] }
        });
        let inner = unwrap_envelope_data(envelope);
        assert_eq!(inner, serde_json::json!({ "articles": [] }));
    }

    #[test]
    fn unwrap_envelope_data_passes_through_raw() {
        let raw = serde_json::json!({ "articles": [{"id":"x"}] });
        assert_eq!(unwrap_envelope_data(raw.clone()), raw);
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GET_BREAKING_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/error/code").and_then(serde_json::Value::as_str),
            Some("bootstrap_upstream_empty"),
        );
    }

    #[tokio::test]
    async fn handler_returns_only_high_and_critical_by_default() {
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: vec![
                article("info", Some(Severity::Info), 1),
                article("warn", Some(Severity::Warn), 2),
                article("hi", Some(Severity::High), 3),
                article("crit", Some(Severity::Critical), 4),
            ],
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GET_BREAKING_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GetBreakingResponse = serde_json::from_slice(&body).unwrap();
        let ids: Vec<&str> = parsed.articles.iter().map(|a| a.id.as_str()).collect();
        // Newest-first order: crit (ts=4) then hi (ts=3).
        assert_eq!(ids, vec!["crit", "hi"]);
        assert_eq!(parsed.total, 2);
        assert!(!parsed.stale);
    }

    #[tokio::test]
    async fn handler_honours_since_ms_query_param() {
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: vec![
                article("old", Some(Severity::Critical), 100),
                article("new", Some(Severity::Critical), 500),
            ],
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{GET_BREAKING_PATH}?sinceMs=200"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GetBreakingResponse = serde_json::from_slice(&body).unwrap();
        let ids: Vec<&str> = parsed.articles.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["new"]);
        assert_eq!(parsed.total, 1);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = serde_json::json!({ "articles": "not-an-array" });
        let env = Envelope::new(bad);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GET_BREAKING_PATH)
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
            parsed.pointer("/error/code").and_then(serde_json::Value::as_str),
            Some("cache_shape"),
        );
    }

    #[tokio::test]
    async fn handler_clamps_limit_via_query_param() {
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: (0..10)
                .map(|i| article(&format!("a{i}"), Some(Severity::Critical), i as i64))
                .collect(),
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{GET_BREAKING_PATH}?limit=2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GetBreakingResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.articles.len(), 2);
        assert_eq!(parsed.total, 10);
    }

    #[tokio::test]
    async fn handler_marks_stale_response() {
        // Drive the stale branch by writing a row with a 0-ms TTL
        // so the cache layer surfaces it as `Stale` on read.
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: vec![article("hi", Some(Severity::High), 1)],
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 0).await.unwrap();
        // Wait a tick so the row's ttl_expires_at is now in the past.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GET_BREAKING_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Even when stale we still serve 200 (banner just renders
        // a "showing cached snapshot" footer client-side).
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: GetBreakingResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
        assert_eq!(parsed.articles.len(), 1);
    }
}

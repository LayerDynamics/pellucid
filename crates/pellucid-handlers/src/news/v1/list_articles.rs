//! `GET /api/news/v1/list-articles` handler.
//!
//! Pure cache reader — returns the article list a news seeder
//! has written to `news:articles:list:v1`. The seeder writes a
//! `SeedEnvelope<{ articles: [NewsArticle] }>`; this handler
//! unwraps the envelope and returns the inner list.
//!
//! ## Why a cache reader (not a live fetcher)
//!
//! Unlike the aviation handler (which calls aviationstack live on
//! cache miss because the upstream supports per-flight queries),
//! news data flows from the relay's RSS / GDELT / Telegram seed
//! pipeline. The handler must NEVER call the relay synchronously
//! per the H3 fix (SPEC-001 §24): edge handlers read from the
//! shared SQLite cache; only the relay populates it.
//!
//! ## M4 outage path
//!
//! When the cache key has no fresh, no stale, AND no
//! negative-sentinel value, the handler returns
//! `503 + Retry-After + bootstrap_upstream_empty` per SPEC-001
//! §24's M4 fix — same shape the bootstrap handler uses so the
//! webview's outage banner sizes the same retry policy.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key the handler reads. The cache-key linter
/// (`tools/check-cache-keys.ts`, T2.10) scans for this literal so
/// the keys.rs FAST_KEYS list and the handler stay in sync.
pub const CACHE_KEY: &str = "news:articles:list:v1";

/// SPEC-001 §11.2 default Retry-After for the M4 outage path. Same
/// 30 s the bootstrap handler uses so the webview's outage banner
/// sizes its countdown identically across endpoints.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Hard cap on `?limit=`. Larger values are clamped silently — the
/// underlying cache row only ever holds the latest N articles
/// the seeder published, so over-asking just hits the cap.
pub const MAX_LIMIT: usize = 200;

/// One news article in the response. Mirrors `NewsCardItem` in
/// `webview/src/panels/news/NewsCard.tsx` so the panel can
/// consume the response without re-shaping.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NewsArticle {
    /// Stable identifier. Seeders generate this from the upstream
    /// row id so dedup across re-publishes is deterministic.
    pub id: String,
    /// Headline. Required.
    pub title: String,
    /// Source attribution (publisher / feed name / agency).
    pub source: String,
    /// Wall-clock ms when the article was published upstream.
    #[serde(rename = "publishedAtMs")]
    pub published_at_ms: i64,
    /// Click-through URL. `None` when the upstream doesn't expose
    /// a stable article URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Short snippet — typically the first ~200 chars of the
    /// article body or an upstream-provided summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Severity tag. The seeder maps upstream signals (RSS keyword
    /// scan, GDELT goldstein-scale bucket, Telegram keyword
    /// classifier) to one of the four levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
}

/// Severity scale matching `webview/src/panels/news/SignalSeverityBadge.tsx`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Neutral / FYI.
    Info,
    /// Elevated attention.
    Warn,
    /// Actionable.
    High,
    /// Immediate.
    Critical,
}

/// The payload shape the seeder writes inside the envelope's
/// `data` field. Wrapping the list in a `{ "articles": [...] }`
/// object leaves room for adjacent metadata (e.g. cursor, total
/// count) without re-versioning the cache key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListArticlesPayload {
    /// The article list. Sorted descending by `published_at_ms`
    /// when the seeder writes the envelope; the handler trusts
    /// upstream ordering and does not re-sort.
    pub articles: Vec<NewsArticle>,
}

/// Wire-format response envelope returned to the webview.
///
/// `Deserialize` is included so integration tests can decode the
/// HTTP body back into this struct without re-typing each field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListArticlesResponse {
    /// Article list — clamped to `min(?limit, MAX_LIMIT)`.
    pub articles: Vec<NewsArticle>,
    /// Total number of articles in the cache slot before the
    /// `?limit` clamp. The webview uses this to render a
    /// "showing 50 of 200" hint.
    pub total: usize,
    /// Whether the response was synthesised from a stale cache
    /// row. The webview shows a subtle "as of …" footer when set.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Deserialize, Default)]
pub struct ListArticlesQuery {
    /// Cap on the number of articles returned. Defaults to 50,
    /// clamped to [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
    /// Optional severity floor. When set, articles below the
    /// requested severity are filtered out before the limit clamp.
    #[serde(default)]
    pub severity: Option<Severity>,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failed to read the row.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but the body did not deserialise into
    /// [`ListArticlesPayload`]. Shape drift between seeder + handler.
    #[error("cache shape: {0}")]
    Shape(String),
    /// M4 outage path: no fresh, no stale, no negative-sentinel
    /// value for [`CACHE_KEY`].
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value emitted with the 503.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    /// Stable error code — the webview branches on this.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status this error maps to.
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

/// Default `?limit` when the client doesn't ask for a specific page
/// size. 50 keeps the wire response under ~25 KiB and matches the
/// NewsPanel's initial render budget.
pub const DEFAULT_LIMIT: usize = 50;

/// Apply `?limit` + `?severity` filters to a raw payload. Pure —
/// extracted so unit tests can exercise the filter boundaries
/// without standing up the cache layer.
#[must_use]
pub fn apply_filters(
    payload: ListArticlesPayload,
    q: &ListArticlesQuery,
) -> (Vec<NewsArticle>, usize) {
    let total = payload.articles.len();
    let mut filtered: Vec<NewsArticle> = if let Some(min) = q.severity.clone() {
        payload
            .articles
            .into_iter()
            .filter(|a| {
                a.severity
                    .as_ref()
                    .map(|s| severity_rank(s) >= severity_rank(&min))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        payload.articles
    };
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
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
    Query(q): Query<ListArticlesQuery>,
) -> Result<Json<ListArticlesResponse>, HandlerError> {
    let raw: CacheHit<Value> = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => {
            // No signal at all — M4 outage path. Negative sentinel
            // for "no articles" is also surfaced as outage here
            // because the news domain has no meaningful "deliberately
            // empty" steady-state — an empty news feed means the
            // upstream pipeline has failed.
            return Err(HandlerError::Outage {
                retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
            });
        }
    };

    let inner = unwrap_envelope_data(value);
    let payload: ListArticlesPayload =
        serde_json::from_value(inner).map_err(|e| HandlerError::Shape(e.to_string()))?;
    let (articles, total) = apply_filters(payload, &q);
    Ok(Json(ListArticlesResponse {
        articles,
        total,
        stale,
    }))
}

/// Same envelope-unwrap helper used by the bootstrap handler:
/// when the cache value is `{ "_seed": ..., "data": ... }` peel
/// the wrapper; otherwise pass through verbatim. Lets seeders
/// that wrote raw payloads coexist with envelope-wrapped writers.
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
    use crate::news::v1::LIST_ARTICLES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use axum::response::IntoResponse;
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    fn sample_articles() -> Vec<NewsArticle> {
        vec![
            NewsArticle {
                id: "a1".into(),
                title: "Suez convoy resumes".into(),
                source: "Reuters".into(),
                published_at_ms: 1_700_000_000_000,
                url: Some("https://example.com/a1".into()),
                summary: Some("First convoy after 14-day closure.".into()),
                severity: Some(Severity::High),
            },
            NewsArticle {
                id: "a2".into(),
                title: "BoJ holds rates".into(),
                source: "Nikkei".into(),
                published_at_ms: 1_700_000_100_000,
                url: None,
                summary: None,
                severity: Some(Severity::Info),
            },
            NewsArticle {
                id: "a3".into(),
                title: "Wildfire alert".into(),
                source: "AP".into(),
                published_at_ms: 1_700_000_200_000,
                url: Some("https://example.com/a3".into()),
                summary: Some("Mandatory evacuation".into()),
                severity: Some(Severity::Critical),
            },
        ]
    }

    fn router_with_cache() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            LIST_ARTICLES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            LIST_ARTICLES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn apply_filters_clamps_to_max_limit() {
        let payload = ListArticlesPayload {
            articles: (0..(MAX_LIMIT + 50))
                .map(|i| NewsArticle {
                    id: format!("a{i}"),
                    title: "x".into(),
                    source: "s".into(),
                    published_at_ms: i as i64,
                    url: None,
                    summary: None,
                    severity: None,
                })
                .collect(),
        };
        let (out, total) = apply_filters(
            payload,
            &ListArticlesQuery {
                limit: Some(MAX_LIMIT * 10),
                severity: None,
            },
        );
        assert_eq!(out.len(), MAX_LIMIT);
        assert_eq!(total, MAX_LIMIT + 50);
    }

    #[test]
    fn apply_filters_default_limit_is_50() {
        let payload = ListArticlesPayload {
            articles: (0..120)
                .map(|i| NewsArticle {
                    id: format!("a{i}"),
                    title: "x".into(),
                    source: "s".into(),
                    published_at_ms: i as i64,
                    url: None,
                    summary: None,
                    severity: None,
                })
                .collect(),
        };
        let (out, total) = apply_filters(payload, &ListArticlesQuery::default());
        assert_eq!(out.len(), DEFAULT_LIMIT);
        assert_eq!(total, 120);
    }

    #[test]
    fn apply_filters_severity_floor_excludes_lower_levels() {
        let payload = ListArticlesPayload {
            articles: sample_articles(),
        };
        let (out, total) = apply_filters(
            payload,
            &ListArticlesQuery {
                limit: None,
                severity: Some(Severity::High),
            },
        );
        // High + Critical pass; Info excluded.
        assert_eq!(out.len(), 2);
        assert_eq!(total, 3);
        assert!(out
            .iter()
            .all(|a| matches!(a.severity, Some(Severity::High) | Some(Severity::Critical))));
    }

    #[test]
    fn apply_filters_severity_floor_excludes_articles_with_no_severity() {
        let mut articles = sample_articles();
        articles.push(NewsArticle {
            id: "a4".into(),
            title: "no-severity".into(),
            source: "x".into(),
            published_at_ms: 0,
            url: None,
            summary: None,
            severity: None,
        });
        let (out, _) = apply_filters(
            ListArticlesPayload { articles },
            &ListArticlesQuery {
                limit: None,
                severity: Some(Severity::Info),
            },
        );
        // The unsevered article is filtered out — no severity tag
        // means we cannot prove the article meets the floor.
        assert!(!out.iter().any(|a| a.id == "a4"));
    }

    #[test]
    fn handler_error_status_codes() {
        assert_eq!(HandlerError::Cache("x".into()).status(), Code::BAD_GATEWAY,);
        assert_eq!(HandlerError::Shape("x".into()).status(), Code::BAD_GATEWAY,);
        assert_eq!(
            HandlerError::Outage {
                retry_after_secs: 30,
            }
            .status(),
            Code::SERVICE_UNAVAILABLE,
        );
    }

    #[test]
    fn handler_error_codes() {
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
    async fn outage_response_carries_retry_after_header() {
        let resp = HandlerError::Outage {
            retry_after_secs: 30,
        }
        .into_response();
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

    #[test]
    fn cache_key_constant_is_stable() {
        // Cache-key linter pin: changing this string is a wire
        // break — bumps to v2 must rev FAST_KEYS too.
        assert_eq!(CACHE_KEY, "news:articles:list:v1");
    }

    #[tokio::test]
    async fn handler_returns_503_with_retry_after_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_ARTICLES_PATH)
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
            parsed
                .pointer("/error/code")
                .and_then(serde_json::Value::as_str),
            Some("bootstrap_upstream_empty"),
        );
    }

    #[tokio::test]
    async fn handler_returns_articles_when_cache_populated() {
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: sample_articles(),
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_ARTICLES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: ListArticlesResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.articles.len(), 3);
        assert_eq!(parsed.total, 3);
        assert!(!parsed.stale);
        assert_eq!(parsed.articles[0].id, "a1");
    }

    #[tokio::test]
    async fn handler_clamps_limit_via_query_param() {
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: sample_articles(),
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{LIST_ARTICLES_PATH}?limit=2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: ListArticlesResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.articles.len(), 2);
        assert_eq!(parsed.total, 3);
    }

    #[tokio::test]
    async fn handler_filters_by_severity_floor() {
        let (app, pool) = migrated_router().await;
        let payload = serde_json::to_value(ListArticlesPayload {
            articles: sample_articles(),
        })
        .unwrap();
        let env = Envelope::new(payload);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{LIST_ARTICLES_PATH}?severity=critical"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: ListArticlesResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.articles.len(), 1);
        assert_eq!(parsed.articles[0].id, "a3");
        assert_eq!(parsed.total, 3);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        // Wrong shape — articles field is wrong type.
        let bad = serde_json::json!({ "articles": "not-an-array" });
        let env = Envelope::new(bad);
        set_cached_json(&pool, CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_ARTICLES_PATH)
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
            parsed
                .pointer("/error/code")
                .and_then(serde_json::Value::as_str),
            Some("cache_shape"),
        );
    }

    #[tokio::test]
    async fn router_smoke_compiles() {
        // Just prove the v1::router function returns a Router that
        // mounts the route at LIST_ARTICLES_PATH.
        let (app, _pool) = router_with_cache();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_ARTICLES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // No cache → 502 (cache_failure: in-memory non-migrated pool
        // can't even read seed_meta) — we just prove the route IS
        // mounted by NOT getting 404.
        assert_ne!(resp.status(), Code::NOT_FOUND);
    }
}

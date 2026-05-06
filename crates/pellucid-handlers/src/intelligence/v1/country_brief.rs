//! `GET /api/intelligence/v1/country-brief` handler.
//!
//! Compact summary card variant of the country deep-dive — one
//! sentence-length headline + a tight bag of counts + the top-1
//! actor + the freshest article. The webview's
//! `CountryBriefPanel` (T4.1.8) renders this as a small card
//! suitable for a sidebar or grid cell, complementing the wide
//! `CountryDeepDivePanel`.
//!
//! Reads the same three FAST-tier slots as the deep-dive
//! handler so a single seeder cadence feeds both surfaces.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "country": "Iran",
//!   "region": "Middle East",
//!   "summary": "8 ACLED events, 1 GDELT incident, 1 Telegram mention.",
//!   "topActor": { "name": "IRGC", "events": 8 },
//!   "topArticle": {
//!     "url": "https://…",
//!     "title": "…",
//!     "domain": "…",
//!     "seenDate": "20260504T120000Z"
//!   },
//!   "totals": { "events": 8, "incidents": 1, "messages": 1 },
//!   "assembledAtMs": 1746360000000,
//!   "stale": false
//! }
//! ```
//!
//! `topActor` and `topArticle` are `null` when the matching
//! upstream slot has nothing for the country.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::intelligence::v1::country_deep_dive::{
    self, ActorRow, ArticleRow, CountryDeepDiveQuery, CountryDeepDiveResponse, CountryTotals,
};
use crate::state::AppState;

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Top-1 actor row in the brief.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopActor {
    /// Actor name.
    pub name: String,
    /// ACLED event count attributed to this country.
    pub events: u64,
}

/// Top-1 article in the brief.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TopArticle {
    /// Article URL.
    pub url: String,
    /// Headline.
    pub title: String,
    /// Source domain.
    pub domain: String,
    /// `YYYYMMDDTHHMMSSZ` upstream timestamp.
    #[serde(rename = "seenDate")]
    pub seen_date: String,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CountryBriefResponse {
    /// Echo of the requested country (canonicalised by the
    /// shared deep-dive helper).
    pub country: String,
    /// Coarse region label.
    pub region: String,
    /// One-sentence English summary the panel renders verbatim.
    pub summary: String,
    /// Highest-event actor for this country, or `null` when ACLED
    /// has nothing.
    #[serde(rename = "topActor")]
    pub top_actor: Option<TopActor>,
    /// Freshest GDELT article tagged to this country, or `null`
    /// when GDELT has nothing.
    #[serde(rename = "topArticle")]
    pub top_article: Option<TopArticle>,
    /// Aggregated totals — same shape as the deep-dive endpoint.
    pub totals: CountryTotals,
    /// Maximum of the three upstream `assembled_at_ms` stamps.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when ANY upstream cache slot was stale.
    pub stale: bool,
}

/// Required query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct CountryBriefQuery {
    /// Country name. Required.
    pub country: Option<String>,
}

/// Errors the handler can produce. Codes match the deep-dive
/// handler so the loader's discriminator is shared.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Missing or empty `?country=`.
    #[error("country query parameter is required")]
    MissingCountry,
    /// Cache layer failure (delegated through the deep-dive
    /// handler so both endpoints share the M4 wiring).
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// All three upstream slots are empty.
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
            Self::MissingCountry => "invalid_request",
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::MissingCountry => StatusCode::BAD_REQUEST,
            Self::Cache(_) | Self::Shape(_) => StatusCode::BAD_GATEWAY,
            Self::Outage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<country_deep_dive::HandlerError> for HandlerError {
    fn from(e: country_deep_dive::HandlerError) -> Self {
        match e {
            country_deep_dive::HandlerError::MissingCountry => Self::MissingCountry,
            country_deep_dive::HandlerError::Cache(m) => Self::Cache(m),
            country_deep_dive::HandlerError::Shape(m) => Self::Shape(m),
            country_deep_dive::HandlerError::Outage { retry_after_secs } => {
                Self::Outage { retry_after_secs }
            }
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

/// Render the one-sentence summary line. Pure — exported so
/// unit tests pin the wording.
#[must_use]
pub fn summarise(totals: &CountryTotals) -> String {
    fn plural<'a>(n: u64, singular: &'a str, plural: &'a str) -> &'a str {
        if n == 1 {
            singular
        } else {
            plural
        }
    }
    format!(
        "{} ACLED {}, {} GDELT {}, {} Telegram {}.",
        totals.events,
        plural(totals.events, "event", "events"),
        totals.incidents,
        plural(totals.incidents, "incident", "incidents"),
        totals.messages,
        plural(totals.messages, "mention", "mentions"),
    )
}

/// Pick the freshest article from a deep-dive payload. Pure.
/// Returns `None` when the article list is empty.
#[must_use]
pub fn pick_top_article(articles: &[ArticleRow]) -> Option<TopArticle> {
    articles
        .iter()
        .max_by(|a, b| a.seen_date.cmp(&b.seen_date))
        .map(|a| TopArticle {
            url: a.url.clone(),
            title: a.title.clone(),
            domain: a.domain.clone(),
            seen_date: a.seen_date.clone(),
        })
}

/// Pick the highest-event actor from a deep-dive payload. Pure.
#[must_use]
pub fn pick_top_actor(actors: &[ActorRow]) -> Option<TopActor> {
    actors.first().map(|a| TopActor {
        name: a.name.clone(),
        events: a.events,
    })
}

/// Project a [`CountryDeepDiveResponse`] into the brief. Pure.
#[must_use]
pub fn project(deep: CountryDeepDiveResponse) -> CountryBriefResponse {
    let summary = summarise(&deep.totals);
    let top_actor = pick_top_actor(&deep.actors);
    let top_article = pick_top_article(&deep.articles);
    CountryBriefResponse {
        country: deep.country,
        region: deep.region,
        summary,
        top_actor,
        top_article,
        totals: deep.totals,
        assembled_at_ms: deep.assembled_at_ms,
        stale: deep.stale,
    }
}

/// Axum handler. Delegates to the deep-dive handler with caps
/// trimmed for the brief, then projects the response.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<CountryBriefQuery>,
) -> Result<Json<CountryBriefResponse>, HandlerError> {
    let raw = q.country.unwrap_or_default();
    if raw.trim().is_empty() {
        return Err(HandlerError::MissingCountry);
    }
    // Reuse the deep-dive composer with tight caps so the brief
    // doesn't pay for rows we drop in the projection.
    let dive_query = CountryDeepDiveQuery {
        country: Some(raw),
        actors: Some(1),
        limit: Some(5),
    };
    let deep_resp = country_deep_dive::handler(State(state), Query(dive_query))
        .await
        .map_err(HandlerError::from)?;
    let deep = deep_resp.0;
    Ok(Json(project(deep)))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::intelligence::v1::COUNTRY_BRIEF_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use serde_json::Value;
    use tower::ServiceExt;

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            COUNTRY_BRIEF_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    fn acled_value() -> Value {
        serde_json::json!({
            "rows": [
                {
                    "actor": "IRGC",
                    "event_count": 12,
                    "total_fatalities": 5,
                    "country_breakdown": [["Iran", 8]],
                },
            ],
            "total_events": 12,
            "date_range": ["2026-04-28", "2026-05-05"],
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    fn gdelt_value() -> Value {
        serde_json::json!({
            "rows": [
                {
                    "url": "https://a/old", "title": "old",
                    "seen_date": "20260504T120000Z",
                    "social_image": "", "domain": "a.com",
                    "language": "English", "source_country": "Iran",
                },
                {
                    "url": "https://a/new", "title": "freshest",
                    "seen_date": "20260504T230000Z",
                    "social_image": "", "domain": "b.com",
                    "language": "English", "source_country": "Iran",
                },
            ],
            "query": "x", "timespan": "24h",
            "assembled_at_ms": 1_700_000_001_000_i64,
        })
    }

    #[test]
    fn summarise_uses_singular_plural_correctly() {
        assert_eq!(
            summarise(&CountryTotals {
                events: 1,
                incidents: 1,
                messages: 1
            }),
            "1 ACLED event, 1 GDELT incident, 1 Telegram mention.",
        );
        assert_eq!(
            summarise(&CountryTotals {
                events: 0,
                incidents: 2,
                messages: 3
            }),
            "0 ACLED events, 2 GDELT incidents, 3 Telegram mentions.",
        );
    }

    #[test]
    fn pick_top_actor_returns_first_when_present() {
        let rows = vec![
            ActorRow {
                name: "A".into(),
                events: 5,
                total_fatalities: 0,
            },
            ActorRow {
                name: "B".into(),
                events: 3,
                total_fatalities: 0,
            },
        ];
        let top = pick_top_actor(&rows).unwrap();
        assert_eq!(top.name, "A");
        assert_eq!(top.events, 5);
    }

    #[test]
    fn pick_top_actor_returns_none_when_empty() {
        assert!(pick_top_actor(&[]).is_none());
    }

    #[test]
    fn pick_top_article_picks_max_seen_date() {
        let rows = vec![
            ArticleRow {
                url: "u1".into(),
                title: "old".into(),
                domain: "a".into(),
                language: "en".into(),
                seen_date: "20260504T120000Z".into(),
            },
            ArticleRow {
                url: "u2".into(),
                title: "new".into(),
                domain: "b".into(),
                language: "en".into(),
                seen_date: "20260504T230000Z".into(),
            },
        ];
        let top = pick_top_article(&rows).unwrap();
        assert_eq!(top.url, "u2");
        assert_eq!(top.title, "new");
    }

    #[test]
    fn pick_top_article_returns_none_when_empty() {
        assert!(pick_top_article(&[]).is_none());
    }

    #[test]
    fn project_threads_summary_top_actor_top_article_through() {
        let deep = CountryDeepDiveResponse {
            country: "Iran".into(),
            region: "Middle East".into(),
            actors: vec![ActorRow {
                name: "IRGC".into(),
                events: 8,
                total_fatalities: 5,
            }],
            articles: vec![ArticleRow {
                url: "u1".into(),
                title: "t".into(),
                domain: "d".into(),
                language: "en".into(),
                seen_date: "20260504T120000Z".into(),
            }],
            telegram: vec![],
            totals: CountryTotals {
                events: 8,
                incidents: 1,
                messages: 0,
            },
            assembled_at_ms: 5,
            stale: false,
        };
        let brief = project(deep);
        assert_eq!(brief.country, "Iran");
        assert_eq!(brief.region, "Middle East");
        assert!(brief.summary.contains("8 ACLED"));
        assert!(brief.summary.contains("1 GDELT"));
        assert_eq!(brief.top_actor.unwrap().name, "IRGC");
        assert_eq!(brief.top_article.unwrap().url, "u1");
        assert_eq!(brief.totals.events, 8);
        assert_eq!(brief.assembled_at_ms, 5);
    }

    #[tokio::test]
    async fn handler_returns_400_when_country_is_missing() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(COUNTRY_BRIEF_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_returns_503_when_all_caches_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_BRIEF_PATH}?country=Iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_renders_brief_payload_for_one_country() {
        let (app, pool) = migrated_router().await;
        set_cached_json(
            &pool,
            country_deep_dive::ACLED_CACHE_KEY,
            &Envelope::new(acled_value()),
            60_000,
        )
        .await
        .unwrap();
        set_cached_json(
            &pool,
            country_deep_dive::GDELT_CACHE_KEY,
            &Envelope::new(gdelt_value()),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_BRIEF_PATH}?country=iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CountryBriefResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.country, "Iran");
        assert_eq!(parsed.region, "Middle East");
        assert!(parsed.summary.contains("8 ACLED"));
        let actor = parsed.top_actor.expect("top actor");
        assert_eq!(actor.name, "IRGC");
        assert_eq!(actor.events, 8);
        let article = parsed.top_article.expect("top article");
        // Should pick the freshest seen_date (20260504T230000Z = "new").
        assert_eq!(article.url, "https://a/new");
        assert_eq!(article.title, "freshest");
    }

    #[tokio::test]
    async fn handler_serves_when_only_acled_present() {
        let (app, pool) = migrated_router().await;
        set_cached_json(
            &pool,
            country_deep_dive::ACLED_CACHE_KEY,
            &Envelope::new(acled_value()),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_BRIEF_PATH}?country=Iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CountryBriefResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.top_actor.is_some());
        assert!(parsed.top_article.is_none());
    }
}

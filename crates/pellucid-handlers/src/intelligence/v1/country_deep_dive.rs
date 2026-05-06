//! `GET /api/intelligence/v1/country-deep-dive` handler.
//!
//! Country-scoped composer that folds three FAST-tier cache
//! slots into one deep-dive payload the webview's
//! `CountryDeepDivePanel` (T4.1.7) renders:
//!
//! - `conflict:events-24h:v1` (ACLED actor snapshot) — surfaces
//!   actor names + per-country event counts.
//! - `conflict:incident-feed:v1` (GDELT) — surfaces recent
//!   articles whose `source_country` matches the request.
//! - `telegram:recent-feed:v1` — surfaces Telegram messages.
//!   Telegram messages have no country tag at the wire level,
//!   so the handler scans the message text for the country
//!   name (case-insensitive substring) — coarse, but enough
//!   for the panel's "matching chatter" affordance.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "country": "Iran",
//!   "region": "Middle East",
//!   "actors": [
//!     { "name": "IRGC", "events": 12, "totalFatalities": 0 }
//!   ],
//!   "articles": [
//!     { "url": "…", "title": "…", "domain": "…", "language": "…" }
//!   ],
//!   "telegram": [
//!     { "channel": "…", "dataPost": "…", "url": "…",
//!       "datetime": "…", "text": "…", "views": "…" }
//!   ],
//!   "totals": { "events": 12, "incidents": 8, "messages": 4 },
//!   "assembledAtMs": 1746360000000,
//!   "stale": false
//! }
//! ```

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::intelligence::v1::regional::region_for_country;
use crate::state::AppState;

/// Cache keys read.
pub const ACLED_CACHE_KEY: &str = "conflict:events-24h:v1";
/// GDELT incident-feed cache key.
pub const GDELT_CACHE_KEY: &str = "conflict:incident-feed:v1";
/// Telegram recent-feed cache key.
pub const TELEGRAM_CACHE_KEY: &str = "telegram:recent-feed:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Default top-N actor cap.
pub const DEFAULT_TOP_ACTORS: usize = 10;

/// Default article + telegram-row cap each.
pub const DEFAULT_FEED_LIMIT: usize = 20;

/// Hard cap to keep wire bytes bounded.
pub const MAX_FEED_LIMIT: usize = 100;

/// One actor row scoped to a country.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActorRow {
    /// Actor name as ACLED reported.
    pub name: String,
    /// Events the actor appears in for this country.
    pub events: u64,
    /// Total reported fatalities across those events (sum
    /// across the actor's full breakdown, not country-scoped —
    /// ACLED's `total_fatalities` is per-actor).
    #[serde(rename = "totalFatalities")]
    pub total_fatalities: i64,
}

/// One GDELT article in the response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ArticleRow {
    pub url: String,
    pub title: String,
    pub domain: String,
    pub language: String,
    /// `YYYYMMDDTHHMMSSZ` upstream timestamp.
    #[serde(rename = "seenDate", alias = "seen_date")]
    pub seen_date: String,
}

/// One Telegram message in the response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TelegramRow {
    pub channel: String,
    #[serde(rename = "dataPost", alias = "data_post")]
    pub data_post: String,
    pub url: String,
    pub datetime: String,
    pub text: String,
    pub views: String,
}

/// Aggregated totals for the country.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CountryTotals {
    /// Sum of ACLED event counts attributed to this country.
    pub events: u64,
    /// GDELT articles tagged with this `source_country`.
    pub incidents: u64,
    /// Telegram messages whose body mentions the country name.
    pub messages: u64,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CountryDeepDiveResponse {
    /// Echo of the requested country (canonicalised — see
    /// [`canonicalize_country`]).
    pub country: String,
    /// Coarse region label (`region_for_country()`).
    pub region: String,
    /// Top-N actors operating in this country.
    pub actors: Vec<ActorRow>,
    /// Recent GDELT articles tagged with this country.
    pub articles: Vec<ArticleRow>,
    /// Recent Telegram messages mentioning the country.
    pub telegram: Vec<TelegramRow>,
    /// Aggregated totals.
    pub totals: CountryTotals,
    /// Maximum of the three upstream `assembled_at_ms`
    /// timestamps.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when ANY of the three upstream slots was stale.
    pub stale: bool,
}

/// Required query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct CountryDeepDiveQuery {
    /// Country name. Required — the handler 400s without it.
    pub country: Option<String>,
    /// Optional cap on actors returned (default
    /// [`DEFAULT_TOP_ACTORS`]).
    #[serde(default)]
    pub actors: Option<usize>,
    /// Optional cap on each feed (articles + telegram) — default
    /// [`DEFAULT_FEED_LIMIT`], hard-capped at [`MAX_FEED_LIMIT`].
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Internal — payload shapes for each cache slot.
#[derive(Debug, Deserialize)]
struct AcledActorRowRaw {
    actor: String,
    event_count: u64,
    total_fatalities: i64,
    country_breakdown: Vec<(String, u64)>,
}

#[derive(Debug, Deserialize)]
struct AcledPayload {
    rows: Vec<AcledActorRowRaw>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct GdeltArticleRaw {
    url: String,
    title: String,
    domain: String,
    language: String,
    source_country: String,
    seen_date: String,
}

#[derive(Debug, Deserialize)]
struct GdeltPayload {
    rows: Vec<GdeltArticleRaw>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramMessageRaw {
    channel: String,
    #[serde(alias = "dataPost")]
    data_post: String,
    url: String,
    datetime: String,
    text: String,
    views: String,
}

#[derive(Debug, Deserialize)]
struct TelegramPayload {
    rows: Vec<TelegramMessageRaw>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Missing or empty `?country=` query parameter.
    #[error("country query parameter is required")]
    MissingCountry,
    /// Cache layer failure.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// All three upstream cache slots are empty.
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

/// Trim + lowercase a country string for matching, then
/// re-cap the first letter of each word so the response carries
/// a presentable display form. Pure.
#[must_use]
pub fn canonicalize_country(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    let head: String = first.to_uppercase().collect();
                    let tail: String = chars.as_str().to_ascii_lowercase();
                    head + &tail
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Case-insensitive country match. The handler folds both sides
/// to lowercase + trim before comparing.
#[must_use]
pub fn matches_country(candidate: &str, target: &str) -> bool {
    candidate.trim().eq_ignore_ascii_case(target.trim())
}

/// Case-insensitive substring search used for the Telegram
/// "matching chatter" pass.
#[must_use]
pub fn text_mentions_country(text: &str, country: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&country.trim().to_ascii_lowercase())
}

/// Compose the deep-dive payload from raw cache reads. Pure —
/// extracted so unit tests pin every aggregation boundary.
#[must_use]
pub fn compose(
    country: &str,
    actors_cap: usize,
    feed_cap: usize,
    acled: Option<AcledRollupOwned>,
    gdelt: Option<GdeltRollupOwned>,
    telegram: Option<TelegramRollupOwned>,
) -> CountryDeepDiveResponse {
    let canonical = canonicalize_country(country);
    let region = region_for_country(&canonical).to_string();

    let mut actors: Vec<ActorRow> = Vec::new();
    let mut event_total: u64 = 0;
    if let Some(a) = acled.as_ref() {
        for row in &a.rows {
            for (c, n) in &row.country_breakdown {
                if matches_country(c, &canonical) {
                    actors.push(ActorRow {
                        name: row.actor.clone(),
                        events: *n,
                        total_fatalities: row.total_fatalities,
                    });
                    event_total = event_total.saturating_add(*n);
                }
            }
        }
        actors.sort_by(|a, b| b.events.cmp(&a.events).then_with(|| a.name.cmp(&b.name)));
        if actors.len() > actors_cap {
            actors.truncate(actors_cap);
        }
    }

    let mut articles: Vec<ArticleRow> = Vec::new();
    let mut incident_total: u64 = 0;
    if let Some(g) = gdelt.as_ref() {
        for row in &g.rows {
            if matches_country(&row.source_country, &canonical) {
                if articles.len() < feed_cap {
                    articles.push(ArticleRow {
                        url: row.url.clone(),
                        title: row.title.clone(),
                        domain: row.domain.clone(),
                        language: row.language.clone(),
                        seen_date: row.seen_date.clone(),
                    });
                }
                incident_total = incident_total.saturating_add(1);
            }
        }
    }

    let mut telegram_rows: Vec<TelegramRow> = Vec::new();
    let mut message_total: u64 = 0;
    if let Some(t) = telegram.as_ref() {
        for row in &t.rows {
            if text_mentions_country(&row.text, &canonical) {
                if telegram_rows.len() < feed_cap {
                    telegram_rows.push(TelegramRow {
                        channel: row.channel.clone(),
                        data_post: row.data_post.clone(),
                        url: row.url.clone(),
                        datetime: row.datetime.clone(),
                        text: row.text.clone(),
                        views: row.views.clone(),
                    });
                }
                message_total = message_total.saturating_add(1);
            }
        }
    }

    let assembled_at_ms = [
        acled.as_ref().map_or(0, |a| a.assembled_at_ms),
        gdelt.as_ref().map_or(0, |g| g.assembled_at_ms),
        telegram.as_ref().map_or(0, |t| t.assembled_at_ms),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    let stale = [
        acled.as_ref().is_some_and(|a| a.stale),
        gdelt.as_ref().is_some_and(|g| g.stale),
        telegram.as_ref().is_some_and(|t| t.stale),
    ]
    .into_iter()
    .any(|b| b);

    CountryDeepDiveResponse {
        country: canonical,
        region,
        actors,
        articles,
        telegram: telegram_rows,
        totals: CountryTotals {
            events: event_total,
            incidents: incident_total,
            messages: message_total,
        },
        assembled_at_ms,
        stale,
    }
}

/// Public ACLED rollup — exposed so tests can build inputs
/// without re-typing the seeder field names.
#[derive(Clone, Debug)]
pub struct AcledRollupOwned {
    /// Actor rows.
    pub rows: Vec<AcledActorOwned>,
    /// Wall-clock ms.
    pub assembled_at_ms: i64,
    /// Stale flag.
    pub stale: bool,
}

/// Public ACLED actor row.
#[derive(Clone, Debug)]
pub struct AcledActorOwned {
    /// Actor name.
    pub actor: String,
    /// Total events (across all countries).
    #[allow(dead_code)]
    pub event_count: u64,
    /// Total fatalities (across all countries).
    pub total_fatalities: i64,
    /// Country → event-count breakdown.
    pub country_breakdown: Vec<(String, u64)>,
}

/// Public GDELT rollup.
#[derive(Clone, Debug)]
pub struct GdeltRollupOwned {
    /// Article rows.
    pub rows: Vec<GdeltArticleOwned>,
    /// Wall-clock ms.
    pub assembled_at_ms: i64,
    /// Stale flag.
    pub stale: bool,
}

/// Public GDELT article row.
#[derive(Clone, Debug)]
pub struct GdeltArticleOwned {
    /// URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Domain.
    pub domain: String,
    /// Language.
    pub language: String,
    /// Source country.
    pub source_country: String,
    /// `YYYYMMDDTHHMMSSZ` timestamp.
    pub seen_date: String,
}

/// Public Telegram rollup.
#[derive(Clone, Debug)]
pub struct TelegramRollupOwned {
    /// Message rows.
    pub rows: Vec<TelegramMessageOwned>,
    /// Wall-clock ms.
    pub assembled_at_ms: i64,
    /// Stale flag.
    pub stale: bool,
}

/// Public Telegram message row.
#[derive(Clone, Debug)]
pub struct TelegramMessageOwned {
    /// Channel slug.
    pub channel: String,
    /// `<channel>/<id>`.
    pub data_post: String,
    /// Permalink.
    pub url: String,
    /// ISO-8601 timestamp.
    pub datetime: String,
    /// Plain-text body.
    pub text: String,
    /// View counter.
    pub views: String,
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<CountryDeepDiveQuery>,
) -> Result<Json<CountryDeepDiveResponse>, HandlerError> {
    let raw_country = q.country.unwrap_or_default();
    if raw_country.trim().is_empty() {
        return Err(HandlerError::MissingCountry);
    }
    let actors_cap = q.actors.unwrap_or(DEFAULT_TOP_ACTORS).clamp(1, 100);
    let feed_cap = q
        .limit
        .unwrap_or(DEFAULT_FEED_LIMIT)
        .clamp(1, MAX_FEED_LIMIT);

    let acled_raw = get_cached_json::<Value>(&state.pool, ACLED_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let gdelt_raw = get_cached_json::<Value>(&state.pool, GDELT_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let telegram_raw = get_cached_json::<Value>(&state.pool, TELEGRAM_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (acled_p, acled_stale) = decode_optional::<AcledPayload>(acled_raw)?;
    let (gdelt_p, gdelt_stale) = decode_optional::<GdeltPayload>(gdelt_raw)?;
    let (telegram_p, telegram_stale) = decode_optional::<TelegramPayload>(telegram_raw)?;

    if acled_p.is_none() && gdelt_p.is_none() && telegram_p.is_none() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    let acled_owned = acled_p.map(|p| AcledRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: acled_stale,
        rows: p
            .rows
            .into_iter()
            .map(|r| AcledActorOwned {
                actor: r.actor,
                event_count: r.event_count,
                total_fatalities: r.total_fatalities,
                country_breakdown: r.country_breakdown,
            })
            .collect(),
    });
    let gdelt_owned = gdelt_p.map(|p| GdeltRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: gdelt_stale,
        rows: p
            .rows
            .into_iter()
            .map(|r| GdeltArticleOwned {
                url: r.url,
                title: r.title,
                domain: r.domain,
                language: r.language,
                source_country: r.source_country,
                seen_date: r.seen_date,
            })
            .collect(),
    });
    let telegram_owned = telegram_p.map(|p| TelegramRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: telegram_stale,
        rows: p
            .rows
            .into_iter()
            .map(|r| TelegramMessageOwned {
                channel: r.channel,
                data_post: r.data_post,
                url: r.url,
                datetime: r.datetime,
                text: r.text,
                views: r.views,
            })
            .collect(),
    });

    Ok(Json(compose(
        &raw_country,
        actors_cap,
        feed_cap,
        acled_owned,
        gdelt_owned,
        telegram_owned,
    )))
}

fn decode_optional<T>(raw: CacheHit<Value>) -> Result<(Option<T>, bool), HandlerError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => return Ok((None, false)),
    };
    let inner = unwrap_envelope_data(value);
    let parsed: T =
        serde_json::from_value(inner).map_err(|e| HandlerError::Shape(e.to_string()))?;
    Ok((Some(parsed), stale))
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
    use crate::intelligence::v1::COUNTRY_DEEP_DIVE_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            COUNTRY_DEEP_DIVE_PATH,
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
                    "country_breakdown": [["Iran", 8], ["Iraq", 4]],
                },
                {
                    "actor": "IDF",
                    "event_count": 7,
                    "total_fatalities": 0,
                    "country_breakdown": [["Israel", 7]],
                },
            ],
            "total_events": 19,
            "date_range": ["2026-04-28", "2026-05-05"],
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    fn gdelt_value() -> Value {
        serde_json::json!({
            "rows": [
                {
                    "url": "https://a/1", "title": "t1",
                    "seen_date": "20260504T120000Z",
                    "social_image": "", "domain": "a.com",
                    "language": "English", "source_country": "Iran",
                },
                {
                    "url": "https://a/2", "title": "t2",
                    "seen_date": "20260504T130000Z",
                    "social_image": "", "domain": "b.com",
                    "language": "Persian", "source_country": "iran",
                },
                {
                    "url": "https://a/3", "title": "t3",
                    "seen_date": "20260504T140000Z",
                    "social_image": "", "domain": "c.com",
                    "language": "Hebrew", "source_country": "Israel",
                },
            ],
            "query": "x",
            "timespan": "24h",
            "assembled_at_ms": 1_700_000_001_000_i64,
        })
    }

    fn telegram_value() -> Value {
        serde_json::json!({
            "rows": [
                {
                    "channel": "rt_intl_news",
                    "data_post": "rt_intl_news/1",
                    "url": "https://t.me/rt_intl_news/1",
                    "datetime": "2026-05-04T12:00:00Z",
                    "text": "Iran update — drones launched",
                    "views": "1.2K",
                },
                {
                    "channel": "isw_warstudies",
                    "data_post": "isw_warstudies/2",
                    "url": "https://t.me/isw_warstudies/2",
                    "datetime": "2026-05-04T13:00:00Z",
                    "text": "Ukraine front developments today",
                    "views": "3K",
                },
            ],
            "channels": ["rt_intl_news", "isw_warstudies"],
            "assembled_at_ms": 1_700_000_002_000_i64,
        })
    }

    #[test]
    fn canonicalize_country_titlecases_and_trims() {
        assert_eq!(canonicalize_country("  iran  "), "Iran");
        assert_eq!(
            canonicalize_country("united arab emirates"),
            "United Arab Emirates"
        );
        assert_eq!(canonicalize_country(""), "");
    }

    #[test]
    fn matches_country_is_case_insensitive() {
        assert!(matches_country("IRAN", "iran"));
        assert!(matches_country(" Iran ", "Iran"));
        assert!(!matches_country("Iraq", "Iran"));
    }

    #[test]
    fn text_mentions_country_case_insensitive_substring() {
        assert!(text_mentions_country("the IRAN-backed militia", "Iran",));
        assert!(text_mentions_country("nothing", "ot"));
        assert!(!text_mentions_country("ukraine front", "Iran"));
    }

    #[test]
    fn compose_aggregates_actors_filtered_to_country() {
        let acled = AcledRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![
                AcledActorOwned {
                    actor: "IRGC".into(),
                    event_count: 12,
                    total_fatalities: 5,
                    country_breakdown: vec![("Iran".into(), 8), ("Iraq".into(), 4)],
                },
                AcledActorOwned {
                    actor: "IDF".into(),
                    event_count: 7,
                    total_fatalities: 0,
                    country_breakdown: vec![("Israel".into(), 7)],
                },
            ],
        };
        let resp = compose("iran", 10, 20, Some(acled), None, None);
        assert_eq!(resp.country, "Iran");
        assert_eq!(resp.region, "Middle East");
        assert_eq!(resp.actors.len(), 1);
        assert_eq!(resp.actors[0].name, "IRGC");
        assert_eq!(resp.actors[0].events, 8);
        assert_eq!(resp.totals.events, 8);
    }

    #[test]
    fn compose_actors_capped_to_actors_cap() {
        let acled = AcledRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: (0..15)
                .map(|i| AcledActorOwned {
                    actor: format!("actor{i}"),
                    event_count: i + 1,
                    total_fatalities: 0,
                    country_breakdown: vec![("Iran".into(), i + 1)],
                })
                .collect(),
        };
        let resp = compose("Iran", 5, 20, Some(acled), None, None);
        assert_eq!(resp.actors.len(), 5);
        // Sorted desc — actor14 has the most events.
        assert_eq!(resp.actors[0].name, "actor14");
    }

    #[test]
    fn compose_articles_filtered_by_source_country_case_insensitive() {
        let gdelt = GdeltRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![
                GdeltArticleOwned {
                    url: "u1".into(),
                    title: "t1".into(),
                    domain: "d1".into(),
                    language: "English".into(),
                    source_country: "Iran".into(),
                    seen_date: "20260504T120000Z".into(),
                },
                GdeltArticleOwned {
                    url: "u2".into(),
                    title: "t2".into(),
                    domain: "d2".into(),
                    language: "Persian".into(),
                    source_country: "iran".into(),
                    seen_date: "20260504T130000Z".into(),
                },
                GdeltArticleOwned {
                    url: "u3".into(),
                    title: "t3".into(),
                    domain: "d3".into(),
                    language: "Hebrew".into(),
                    source_country: "Israel".into(),
                    seen_date: "20260504T140000Z".into(),
                },
            ],
        };
        let resp = compose("Iran", 10, 20, None, Some(gdelt), None);
        let urls: Vec<&str> = resp.articles.iter().map(|a| a.url.as_str()).collect();
        assert_eq!(urls, vec!["u1", "u2"]);
        assert_eq!(resp.totals.incidents, 2);
    }

    #[test]
    fn compose_telegram_filtered_by_text_substring() {
        let telegram = TelegramRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![
                TelegramMessageOwned {
                    channel: "a".into(),
                    data_post: "a/1".into(),
                    url: "u1".into(),
                    datetime: "x".into(),
                    text: "Iran update".into(),
                    views: "1K".into(),
                },
                TelegramMessageOwned {
                    channel: "b".into(),
                    data_post: "b/2".into(),
                    url: "u2".into(),
                    datetime: "x".into(),
                    text: "Ukraine front".into(),
                    views: "2K".into(),
                },
            ],
        };
        let resp = compose("Iran", 10, 20, None, None, Some(telegram));
        assert_eq!(resp.telegram.len(), 1);
        assert_eq!(resp.totals.messages, 1);
    }

    #[test]
    fn compose_marks_stale_when_any_upstream_is_stale() {
        let acled = AcledRollupOwned {
            assembled_at_ms: 5,
            stale: true,
            rows: vec![],
        };
        let resp = compose("Iran", 10, 20, Some(acled), None, None);
        assert!(resp.stale);
        assert_eq!(resp.assembled_at_ms, 5);
    }

    #[tokio::test]
    async fn handler_returns_400_when_country_is_missing() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(COUNTRY_DEEP_DIVE_PATH)
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
    async fn handler_returns_503_when_all_caches_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_DEEP_DIVE_PATH}?country=Iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_combines_three_caches_into_one_payload() {
        let (app, pool) = migrated_router().await;
        set_cached_json(
            &pool,
            ACLED_CACHE_KEY,
            &Envelope::new(acled_value()),
            60_000,
        )
        .await
        .unwrap();
        set_cached_json(
            &pool,
            GDELT_CACHE_KEY,
            &Envelope::new(gdelt_value()),
            60_000,
        )
        .await
        .unwrap();
        set_cached_json(
            &pool,
            TELEGRAM_CACHE_KEY,
            &Envelope::new(telegram_value()),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_DEEP_DIVE_PATH}?country=iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CountryDeepDiveResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.country, "Iran");
        assert_eq!(parsed.region, "Middle East");
        assert_eq!(parsed.actors.len(), 1);
        assert_eq!(parsed.articles.len(), 2);
        assert_eq!(parsed.telegram.len(), 1);
        assert_eq!(parsed.totals.events, 8);
        assert_eq!(parsed.totals.incidents, 2);
        assert_eq!(parsed.totals.messages, 1);
        assert_eq!(parsed.assembled_at_ms, 1_700_000_002_000);
    }

    #[tokio::test]
    async fn handler_serves_when_only_one_cache_slot_present() {
        let (app, pool) = migrated_router().await;
        set_cached_json(
            &pool,
            ACLED_CACHE_KEY,
            &Envelope::new(acled_value()),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_DEEP_DIVE_PATH}?country=Iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CountryDeepDiveResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.actors.len(), 1);
        assert_eq!(parsed.articles.len(), 0);
        assert_eq!(parsed.telegram.len(), 0);
    }

    #[tokio::test]
    async fn handler_clamps_actors_and_limit_query_params() {
        let (app, pool) = migrated_router().await;
        set_cached_json(
            &pool,
            ACLED_CACHE_KEY,
            &Envelope::new(acled_value()),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "{COUNTRY_DEEP_DIVE_PATH}?country=Iran&actors=0&limit=99999"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: CountryDeepDiveResponse = serde_json::from_slice(&body).unwrap();
        // actors=0 is clamped to >=1, but the country has only 1
        // matching actor so we still see exactly 1 row.
        assert_eq!(parsed.actors.len(), 1);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "rows": "not-an-array" }));
        set_cached_json(&pool, ACLED_CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{COUNTRY_DEEP_DIVE_PATH}?country=Iran"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}

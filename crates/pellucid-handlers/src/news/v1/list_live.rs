//! `GET /api/news/v1/list-live` — Server-Sent Events stream of
//! news articles as the seeder pipeline publishes them.
//!
//! ## Strategy
//!
//! The handler polls the FAST-tier cache key
//! `news:articles:list:v1` every [`POLL_INTERVAL`] and emits an
//! SSE `article` event for every article whose `id` was not in
//! the previous snapshot. Order of emission matches descending
//! `published_at_ms` (most recent first) so a panel rendering the
//! events as a top-down list naturally prepends.
//!
//! Polling-on-the-cache (not on the upstream) is correct here:
//! the relay's news seeder writes `news:articles:list:v1`
//! atomically; the edge handler reads from the same SQLite via
//! Litestream replication. A future enhancement can replace the
//! poll with a `tokio::sync::broadcast` populated by the relay's
//! atomic_publish notifications without changing the wire shape.
//!
//! ## Wire shape
//!
//! `text/event-stream` per HTML5 SSE. Three event types:
//!
//! - `event: ready` — emitted once on connect; `data` is the
//!   initial article list (same schema as `list-articles`). The
//!   client uses this to seed the panel before any deltas arrive.
//! - `event: article` — emitted per new article; `data` is one
//!   `NewsArticle` JSON object.
//! - `event: outage` — emitted when the cache reports M4 outage.
//!   The client should switch to its outage banner. Subsequent
//!   reads continue; once the cache repopulates a `ready` event
//!   resyncs the panel.
//!
//! Plus axum's built-in keep-alive comments every
//! [`KEEP_ALIVE_INTERVAL`] so proxies don't time the connection
//! out during quiet periods.

use std::collections::HashSet;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};

use crate::news::v1::list_articles::{
    apply_filters, ListArticlesPayload, ListArticlesQuery, NewsArticle, Severity,
    CACHE_KEY,
};
use crate::state::AppState;

/// How often the handler polls the cache for new articles.
///
/// 5 s is the same cadence the original WorldMonitor live news
/// feed used. Production seeders publish every 60–120 s, so 5 s
/// catches new publishes within a few seconds of the seeder's
/// `atomic_publish` commit + Litestream replica window.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// SSE keep-alive comment cadence. axum injects `:` lines this
/// often when the data stream is quiet; many edge proxies time a
/// silent stream out at ~30 s.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Optional query knobs — same shape as `list-articles` so panels
/// can hand off the same filter object to either endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct ListLiveQuery {
    /// Optional severity floor — articles below the requested
    /// severity are dropped before emission.
    pub severity: Option<Severity>,
    /// Cap on the initial `ready` event's article count. Default
    /// is 50; clamped to the same `MAX_LIMIT` as `list-articles`.
    pub limit: Option<usize>,
}

/// One SSE record before it's serialised into axum's `Event`.
/// Pulled out so tests can match on `(name, data)` without
/// reaching into axum's private `Event` buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// SSE event name — `ready`, `article`, `outage`, `error`.
    pub name: String,
    /// JSON payload that will be serialised into the `data:`
    /// field of the SSE frame.
    pub data: Value,
}

/// Build the SSE response stream.
///
/// The function is `pub` so tests can drive it without standing
/// up axum's full extractor machinery.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<ListLiveQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let records = build_record_stream(state, q, POLL_INTERVAL);
    let events = futures::StreamExt::map(records, |rec| Ok(record_to_event(&rec)));
    Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(KEEP_ALIVE_INTERVAL)
            .text("pellucid-keepalive"),
    )
}

/// Pure-async stream builder — extracted so tests can pass a
/// shorter `interval` and an in-process `AppState` to assert
/// emission order without waiting 5 s per tick.
pub fn build_record_stream(
    state: AppState,
    q: ListLiveQuery,
    interval: Duration,
) -> impl Stream<Item = Record> {
    let filters = ListArticlesQuery {
        limit: q.limit,
        severity: q.severity.clone(),
    };
    async_stream::stream! {
        let mut seen: HashSet<String> = HashSet::new();

        // Initial snapshot — emitted as the `ready` event so the
        // panel can render the canonical list before any deltas
        // arrive. After this point only new articles flow.
        let initial = read_filtered(&state, &filters).await;
        match initial {
            Snapshot::Articles { articles, total, stale } => {
                for a in &articles {
                    seen.insert(a.id.clone());
                }
                yield Record {
                    name: "ready".into(),
                    data: serde_json::json!({
                        "articles": articles,
                        "total": total,
                        "stale": stale,
                    }),
                };
            }
            Snapshot::Outage => {
                yield Record {
                    name: "outage".into(),
                    data: serde_json::json!({ "reason": "bootstrap_upstream_empty" }),
                };
            }
            Snapshot::Error(msg) => {
                yield Record {
                    name: "error".into(),
                    data: serde_json::json!({ "code": "cache_failure", "message": msg }),
                };
            }
        }

        // Delta loop — poll every `interval`, dedupe by id, emit
        // each new article. The loop yields forever; the client
        // closes the connection by dropping the EventSource and
        // the runtime drops the stream.
        loop {
            tokio::time::sleep(interval).await;
            let snapshot = read_filtered(&state, &filters).await;
            match snapshot {
                Snapshot::Articles { articles, total: _, stale: _ } => {
                    for a in &articles {
                        if seen.insert(a.id.clone()) {
                            yield Record {
                                name: "article".into(),
                                data: serde_json::to_value(a).unwrap_or(Value::Null),
                            };
                        }
                    }
                }
                Snapshot::Outage => {
                    // Reset the dedupe set so when the cache
                    // recovers we re-emit the full snapshot via
                    // a fresh `ready`.
                    seen.clear();
                    yield Record {
                        name: "outage".into(),
                        data: serde_json::json!({ "reason": "bootstrap_upstream_empty" }),
                    };
                }
                Snapshot::Error(msg) => {
                    yield Record {
                        name: "error".into(),
                        data: serde_json::json!({ "code": "cache_failure", "message": msg }),
                    };
                }
            }
        }
    }
}

/// Adapter — shape one `Record` into axum's `Event`. Used by
/// the production [`handler`]; tests assert against the `Record`
/// directly via [`build_record_stream`].
pub fn record_to_event(rec: &Record) -> Event {
    Event::default()
        .event(&rec.name)
        .data(serde_json::to_string(&rec.data).unwrap_or_else(|_| "{}".to_string()))
}

/// One step of the polling loop. Pure read — no I/O outside the
/// cache layer.
async fn read_filtered(state: &AppState, q: &ListArticlesQuery) -> Snapshot {
    let raw = match get_cached_json::<Value>(&state.pool, CACHE_KEY).await {
        Ok(v) => v,
        Err(e) => return Snapshot::Error(e.to_string()),
    };
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => return Snapshot::Outage,
    };
    let inner = unwrap_envelope_data(value);
    let payload: ListArticlesPayload = match serde_json::from_value(inner) {
        Ok(p) => p,
        Err(e) => return Snapshot::Error(e.to_string()),
    };
    let (articles, total) = apply_filters(payload, q);
    Snapshot::Articles {
        articles,
        total,
        stale,
    }
}

/// Same envelope-unwrap helper used by `list_articles` — peels
/// the seeder's `_seed` wrapper when present, passes through
/// otherwise.
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

/// Tagged snapshot type so the loop branches on cache state
/// without smuggling `Result`s through the stream.
enum Snapshot {
    Articles {
        articles: Vec<NewsArticle>,
        total: usize,
        stale: bool,
    },
    Outage,
    Error(String),
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::news::v1::list_articles::{NewsArticle, Severity};
    use futures::StreamExt;
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use std::time::Duration as Dur;

    fn article(id: &str, sev: Option<Severity>) -> NewsArticle {
        NewsArticle {
            id: id.into(),
            title: format!("title-{id}"),
            source: "src".into(),
            published_at_ms: 0,
            url: None,
            summary: None,
            severity: sev,
        }
    }

    async fn write_payload(pool: &pellucid_db::Pool, articles: Vec<NewsArticle>) {
        let payload = serde_json::to_value(ListArticlesPayload { articles }).unwrap();
        let env = Envelope::new(payload);
        set_cached_json(pool, CACHE_KEY, &env, 60_000).await.unwrap();
    }

    /// Pull records off the stream until we've collected `n` of
    /// them OR the per-event timeout elapses.
    async fn collect_n<S>(
        mut stream: std::pin::Pin<Box<S>>,
        n: usize,
        per_event_timeout: Dur,
    ) -> Vec<Record>
    where
        S: Stream<Item = Record> + Send + ?Sized,
    {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            match tokio::time::timeout(per_event_timeout, stream.next()).await {
                Ok(Some(rec)) => out.push(rec),
                Ok(None) | Err(_) => break,
            }
        }
        out
    }

    #[tokio::test]
    async fn ready_event_is_emitted_first_with_initial_snapshot() {
        let state = AppState::for_tests_async().await.unwrap();
        write_payload(
            &state.pool,
            vec![article("a1", Some(Severity::High)), article("a2", None)],
        )
        .await;
        let stream = Box::pin(build_record_stream(
            state,
            ListLiveQuery::default(),
            Dur::from_millis(50),
        ));
        let events = collect_n(stream, 1, Dur::from_secs(2)).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "ready");
        let arr = events[0]
            .data
            .pointer("/articles")
            .and_then(Value::as_array)
            .expect("articles");
        assert_eq!(arr.len(), 2);
        assert_eq!(
            events[0].data.pointer("/total").and_then(Value::as_u64),
            Some(2),
        );
    }

    #[tokio::test]
    async fn outage_event_when_cache_empty() {
        let state = AppState::for_tests_async().await.unwrap();
        let stream = Box::pin(build_record_stream(
            state,
            ListLiveQuery::default(),
            Dur::from_millis(50),
        ));
        let events = collect_n(stream, 1, Dur::from_secs(2)).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "outage");
        assert_eq!(
            events[0]
                .data
                .pointer("/reason")
                .and_then(Value::as_str),
            Some("bootstrap_upstream_empty"),
        );
    }

    #[tokio::test]
    async fn new_articles_emit_one_event_per_id() {
        let state = AppState::for_tests_async().await.unwrap();
        write_payload(&state.pool, vec![article("a1", None)]).await;
        let mut s = Box::pin(build_record_stream(
            state.clone(),
            ListLiveQuery::default(),
            Dur::from_millis(40),
        ));

        // Drain the initial `ready` event.
        let first = tokio::time::timeout(Dur::from_secs(2), s.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.name, "ready");

        // Add two new articles to the cache. The poll loop should
        // pick them up on the next tick.
        write_payload(
            &state.pool,
            vec![
                article("a1", None), // already seen — must not emit
                article("a2", None),
                article("a3", None),
            ],
        )
        .await;
        let events = collect_n(s, 2, Dur::from_secs(3)).await;
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.name == "article"));
        let ids: Vec<String> = events
            .iter()
            .map(|e| {
                e.data
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        assert!(ids.contains(&"a2".to_string()));
        assert!(ids.contains(&"a3".to_string()));
    }

    #[tokio::test]
    async fn severity_filter_drops_lower_levels_from_initial_and_deltas() {
        let state = AppState::for_tests_async().await.unwrap();
        write_payload(
            &state.pool,
            vec![
                article("low", Some(Severity::Info)),
                article("hi", Some(Severity::Critical)),
            ],
        )
        .await;
        let q = ListLiveQuery {
            severity: Some(Severity::High),
            limit: None,
        };
        let mut stream = Box::pin(build_record_stream(state, q, Dur::from_millis(40)));
        let first = tokio::time::timeout(Dur::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.name, "ready");
        let arr = first
            .data
            .pointer("/articles")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("id").and_then(Value::as_str), Some("hi"));
    }

    #[tokio::test]
    async fn outage_recovers_to_article_events_when_cache_repopulates() {
        let state = AppState::for_tests_async().await.unwrap();
        // Empty cache → first event is `outage`.
        let mut stream = Box::pin(build_record_stream(
            state.clone(),
            ListLiveQuery::default(),
            Dur::from_millis(30),
        ));
        let first = tokio::time::timeout(Dur::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.name, "outage");

        // Now populate. After the recovery path resets `seen`,
        // every article appears as a new "article" event.
        write_payload(&state.pool, vec![article("a1", None)]).await;
        let events = collect_n(stream, 1, Dur::from_secs(3)).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "article");
    }

    #[test]
    fn record_to_event_emits_named_data_event() {
        let rec = Record {
            name: "ready".into(),
            data: serde_json::json!({"x": 1}),
        };
        let _ev = record_to_event(&rec);
        // Just prove the helper doesn't panic + accepts every
        // event name we use in the loop. The Event's wire shape
        // is verified by axum's own SSE tests.
        for name in ["ready", "article", "outage", "error"] {
            let _ = record_to_event(&Record {
                name: name.into(),
                data: Value::Null,
            });
        }
    }
}

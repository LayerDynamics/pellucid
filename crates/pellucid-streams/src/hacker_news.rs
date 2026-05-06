//! Hacker News Firebase API client.
//!
//! Y Combinator publishes the official Hacker News data at:
//!
//! ```text
//! GET https://hacker-news.firebaseio.com/v0/topstories.json
//! GET https://hacker-news.firebaseio.com/v0/item/<id>.json
//! ```
//!
//! Free, no auth, no rate limit (per the official docs).
//! `topstories.json` returns the top ~500 story ids; each
//! `item/<id>.json` returns one story's metadata.

use std::time::Duration;

use futures::future::join_all;
use serde::Deserialize;

use crate::error::StreamsError;

/// Default base URL — HN Firebase production.
pub const DEFAULT_BASE_URL: &str = "https://hacker-news.firebaseio.com";

/// Default per-request timeout — 8 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct HackerNewsConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for HackerNewsConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable HN client.
#[derive(Clone, Debug)]
pub struct HackerNewsClient {
    http: reqwest::Client,
    config: HackerNewsConfig,
}

impl HackerNewsClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: HackerNewsConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = HackerNewsConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the top-N story ids and resolve them to typed
    /// [`HnStory`] rows in one fan-out.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx top-stories.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_top_stories(&self, limit: usize) -> Result<Vec<HnStory>, StreamsError> {
        let ids = self.fetch_top_ids().await?;
        let calls = ids
            .iter()
            .take(limit)
            .map(|id| async move { self.fetch_item(*id).await.ok().flatten() });
        Ok(join_all(calls).await.into_iter().flatten().collect())
    }

    async fn fetch_top_ids(&self) -> Result<Vec<u64>, StreamsError> {
        let url = format!("{}/v0/topstories.json", self.config.base_url);
        let resp = self
            .http
            .get(&url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: Vec<u64> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body)
    }

    async fn fetch_item(&self, id: u64) -> Result<Option<HnStory>, StreamsError> {
        let url = format!("{}/v0/item/{}.json", self.config.base_url, id);
        let resp = self
            .http
            .get(&url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: Option<RawItem> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.map(HnStory::from_raw))
    }
}

/// One Hacker News story row.
#[derive(Clone, Debug, PartialEq)]
pub struct HnStory {
    /// HN item id.
    pub id: u64,
    /// Submitting user.
    pub by: String,
    /// Story title.
    pub title: String,
    /// External URL (when present; empty for Ask HN / Show HN
    /// text posts).
    pub url: String,
    /// Score (upvotes).
    pub score: i64,
    /// Comment count.
    pub descendants: i64,
    /// Wall-clock seconds when submitted.
    pub time_unix: i64,
    /// HN item type (`story | job | poll | …`).
    pub item_type: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawItem {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    by: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    descendants: i64,
    #[serde(default)]
    time: i64,
    #[serde(default, rename = "type")]
    item_type: String,
}

impl HnStory {
    fn from_raw(raw: RawItem) -> Self {
        Self {
            id: raw.id,
            by: raw.by,
            title: raw.title,
            url: raw.url,
            score: raw.score,
            descendants: raw.descendants,
            time_unix: raw.time,
            item_type: raw.item_type,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn item_body(id: u64, title: &str, score: i64) -> serde_json::Value {
        serde_json::json!({
            "id":          id,
            "by":          "alice",
            "title":       title,
            "url":         format!("https://example.com/{id}"),
            "score":       score,
            "descendants": 42,
            "time":        1_714_060_800_i64,
            "type":        "story"
        })
    }

    fn client_pointing_at(server: &MockServer) -> HackerNewsClient {
        HackerNewsClient::new(
            HackerNewsConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_top_stories_returns_resolved_items() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/topstories.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([1001, 1002, 1003])),
            )
            .mount(&server)
            .await;
        for (id, score) in [(1001_u64, 500_i64), (1002, 300), (1003, 100)] {
            Mock::given(method("GET"))
                .and(path(format!("/v0/item/{id}.json")))
                .respond_with(ResponseTemplate::new(200).set_body_json(item_body(
                    id,
                    &format!("Story {id}"),
                    score,
                )))
                .mount(&server)
                .await;
        }
        let client = client_pointing_at(&server);
        let stories = client.fetch_top_stories(3).await.unwrap();
        assert_eq!(stories.len(), 3);
        assert_eq!(stories[0].id, 1001);
        assert_eq!(stories[0].score, 500);
        assert_eq!(stories[0].item_type, "story");
    }

    #[tokio::test]
    async fn fetch_top_stories_truncates_to_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/topstories.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([1, 2, 3, 4, 5])),
            )
            .mount(&server)
            .await;
        for id in 1_u64..=2 {
            Mock::given(method("GET"))
                .and(path(format!("/v0/item/{id}.json")))
                .respond_with(ResponseTemplate::new(200).set_body_json(item_body(id, "x", 1)))
                .mount(&server)
                .await;
        }
        let client = client_pointing_at(&server);
        let stories = client.fetch_top_stories(2).await.unwrap();
        assert_eq!(stories.len(), 2);
    }

    #[tokio::test]
    async fn fetch_top_stories_skips_null_items() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/topstories.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([1, 2])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v0/item/1.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(item_body(1, "ok", 1)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v0/item/2.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::Value::Null))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let stories = client.fetch_top_stories(2).await.unwrap();
        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].id, 1);
    }

    #[tokio::test]
    async fn fetch_top_stories_top_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/topstories.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_top_stories(10).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_top_stories_unreachable_yields_io() {
        let client = HackerNewsClient::new(
            HackerNewsConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_top_stories(10).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = HackerNewsConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}

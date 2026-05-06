//! Metaculus question / forecast API client.
//!
//! Metaculus exposes a REST API at:
//!
//! ```text
//! GET https://www.metaculus.com/api2/questions/?status=open&order_by=-activity&limit=20
//! ```
//!
//! Free, no auth required for read access. Returns
//! crowd-forecasts on a wide variety of geopolitical /
//! economic / scientific questions.
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "results": [
//!     {
//!       "id":              12345,
//!       "title":           "Will X happen by Y?",
//!       "url":             "/questions/12345/...",
//!       "page_url":        "/questions/12345/...",
//!       "status":          "open",
//!       "resolve_time":    "2026-12-31T00:00:00Z",
//!       "activity":        42.5,
//!       "community_prediction": {
//!         "full":    { "q1": 0.30, "q2": 0.45, "q3": 0.55 },
//!         "history": []
//!       }
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — Metaculus production.
pub const DEFAULT_BASE_URL: &str = "https://www.metaculus.com";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct MetaculusConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for MetaculusConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable Metaculus client.
#[derive(Clone, Debug)]
pub struct MetaculusClient {
    http: reqwest::Client,
    config: MetaculusConfig,
}

impl MetaculusClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: MetaculusConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = MetaculusConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch up to `limit` open Metaculus questions, ordered
    /// by recent activity.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_active_questions(
        &self,
        limit: u32,
    ) -> Result<Vec<MetaculusQuestion>, StreamsError> {
        let url = self.build_url(limit)?;
        let resp = self
            .http
            .get(url)
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
        let body: QuestionsResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body
            .results
            .into_iter()
            .map(MetaculusQuestion::from_raw)
            .collect())
    }

    fn build_url(&self, limit: u32) -> Result<Url, StreamsError> {
        let raw = format!("{}/api2/questions/", self.config.base_url);
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("metaculus url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("status", "open");
            q.append_pair("order_by", "-activity");
            q.append_pair("limit", &limit.to_string());
        }
        Ok(url)
    }
}

/// One Metaculus question + community prediction.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaculusQuestion {
    /// Numeric Metaculus id.
    pub id: i64,
    /// Question title.
    pub title: String,
    /// Permalink path (relative to `metaculus.com`).
    pub page_url: String,
    /// `open | closed | resolved` status.
    pub status: String,
    /// Resolution timestamp.
    pub resolve_time: String,
    /// Activity score (Metaculus's internal popularity metric).
    pub activity: f64,
    /// Median (q2) of the community prediction in `[0, 1]`.
    /// 0.0 when the question has no community forecast yet.
    pub community_median: f64,
    /// 25th percentile of community prediction.
    pub community_q1: f64,
    /// 75th percentile of community prediction.
    pub community_q3: f64,
}

#[derive(Debug, Deserialize)]
struct QuestionsResponse {
    #[serde(default)]
    results: Vec<RawQuestion>,
}

#[derive(Debug, Default, Deserialize)]
struct RawQuestion {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    page_url: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    resolve_time: String,
    #[serde(default)]
    activity: f64,
    #[serde(default)]
    community_prediction: Value,
}

impl MetaculusQuestion {
    fn from_raw(raw: RawQuestion) -> Self {
        let (q1, q2, q3) = parse_quartiles(&raw.community_prediction);
        Self {
            id: raw.id,
            title: raw.title,
            page_url: raw.page_url,
            status: raw.status,
            resolve_time: raw.resolve_time,
            activity: raw.activity,
            community_median: q2,
            community_q1: q1,
            community_q3: q3,
        }
    }
}

fn parse_quartiles(v: &Value) -> (f64, f64, f64) {
    let full = v.get("full").unwrap_or(&Value::Null);
    let q1 = full.get("q1").and_then(Value::as_f64).unwrap_or(0.0);
    let q2 = full.get("q2").and_then(Value::as_f64).unwrap_or(0.0);
    let q3 = full.get("q3").and_then(Value::as_f64).unwrap_or(0.0);
    (q1, q2, q3)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "results": [
                {
                    "id":           12345,
                    "title":        "Will X happen by Y?",
                    "page_url":     "/questions/12345/will-x/",
                    "status":       "open",
                    "resolve_time": "2026-12-31T00:00:00Z",
                    "activity":     42.5,
                    "community_prediction": {
                        "full":    { "q1": 0.30, "q2": 0.45, "q3": 0.55 },
                        "history": []
                    }
                },
                {
                    "id":           67890,
                    "title":        "Question without prediction",
                    "page_url":     "/questions/67890/",
                    "status":       "open",
                    "resolve_time": "2026-08-31T00:00:00Z",
                    "activity":     5.0
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> MetaculusClient {
        MetaculusClient::new(
            MetaculusConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_active_questions_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api2/questions/"))
            .and(query_param("status", "open"))
            .and(query_param("order_by", "-activity"))
            .and(query_param("limit", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let questions = client.fetch_active_questions(20).await.unwrap();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].id, 12345);
        assert!((questions[0].community_median - 0.45).abs() < 1e-9);
        assert!((questions[0].community_q1 - 0.30).abs() < 1e-9);
        assert!((questions[0].community_q3 - 0.55).abs() < 1e-9);
        assert!((questions[0].activity - 42.5).abs() < 1e-9);
        // Question without prediction defaults to zeros.
        assert!((questions[1].community_median - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn fetch_active_questions_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api2/questions/"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_active_questions(20).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_active_questions_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api2/questions/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_active_questions(20).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_active_questions_unreachable_yields_io() {
        let client = MetaculusClient::new(
            MetaculusConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_active_questions(20).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn parse_quartiles_returns_zeros_when_absent() {
        let (q1, q2, q3) = parse_quartiles(&Value::Null);
        assert!((q1 - 0.0).abs() < 1e-9);
        assert!((q2 - 0.0).abs() < 1e-9);
        assert!((q3 - 0.0).abs() < 1e-9);
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = MetaculusConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}

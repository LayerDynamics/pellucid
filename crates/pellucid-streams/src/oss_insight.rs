//! OSSInsight trending-repos client.
//!
//! GitHub deprecated their official trending API in 2018.
//! OSSInsight (run by PingCAP, backed by GitHub Archive data)
//! exposes a free public REST API:
//!
//! ```text
//! GET https://api.ossinsight.io/v1/trends/repos/?language=&period=past_24_hours
//! ```
//!
//! Free, no auth, modest rate limit.
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "data": {
//!     "rows": [
//!       {
//!         "repo_id":           "12345",
//!         "repo_name":         "owner/repo",
//!         "stars":             "8500",
//!         "forks":             "1200",
//!         "pushes":            "42",
//!         "total_score":       "0.85",
//!         "primary_language":  "Rust",
//!         "description":       "..."
//!       }
//!     ]
//!   }
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — OSSInsight production.
pub const DEFAULT_BASE_URL: &str = "https://api.ossinsight.io";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Trending period — maps to OSSInsight's `period` query parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrendingPeriod {
    /// `past_24_hours`.
    Past24Hours,
    /// `past_week`.
    PastWeek,
    /// `past_month`.
    PastMonth,
}

impl TrendingPeriod {
    /// Slug used in the `period` query parameter.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Past24Hours => "past_24_hours",
            Self::PastWeek => "past_week",
            Self::PastMonth => "past_month",
        }
    }
}

/// Configuration.
#[derive(Clone, Debug)]
pub struct OssInsightConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for OssInsightConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable OSSInsight client.
#[derive(Clone, Debug)]
pub struct OssInsightClient {
    http: reqwest::Client,
    config: OssInsightConfig,
}

impl OssInsightClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: OssInsightConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = OssInsightConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch trending repos for `period`. Pass `Some(lang)`
    /// to filter (e.g. `"Rust"`); `None` returns all
    /// languages.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_trending(
        &self,
        period: TrendingPeriod,
        language: Option<&str>,
    ) -> Result<Vec<TrendingRepo>, StreamsError> {
        let url = self.build_url(period, language)?;
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
        let body: RawResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.data.rows.into_iter().map(TrendingRepo::from_raw).collect())
    }

    fn build_url(
        &self,
        period: TrendingPeriod,
        language: Option<&str>,
    ) -> Result<Url, StreamsError> {
        let raw = format!("{}/v1/trends/repos/", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("ossinsight url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("period", period.slug());
            if let Some(lang) = language {
                q.append_pair("language", lang);
            }
        }
        Ok(url)
    }
}

/// One trending repo row.
#[derive(Clone, Debug, PartialEq)]
pub struct TrendingRepo {
    /// `owner/name` repo path.
    pub repo_name: String,
    /// Star count.
    pub stars: i64,
    /// Fork count.
    pub forks: i64,
    /// Push count over the period.
    pub pushes: i64,
    /// Composite trending score (`[0, 1]`).
    pub total_score: f64,
    /// Primary language (e.g. `"Rust"`).
    pub primary_language: String,
    /// Repo description.
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    data: RawData,
}

#[derive(Debug, Default, Deserialize)]
struct RawData {
    #[serde(default)]
    rows: Vec<RawRepo>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRepo {
    #[serde(default)]
    repo_name: String,
    #[serde(default)]
    stars: String,
    #[serde(default)]
    forks: String,
    #[serde(default)]
    pushes: String,
    #[serde(default)]
    total_score: String,
    #[serde(default)]
    primary_language: String,
    #[serde(default)]
    description: String,
}

impl TrendingRepo {
    fn from_raw(raw: RawRepo) -> Self {
        Self {
            repo_name: raw.repo_name,
            stars: raw.stars.trim().parse::<i64>().unwrap_or(0),
            forks: raw.forks.trim().parse::<i64>().unwrap_or(0),
            pushes: raw.pushes.trim().parse::<i64>().unwrap_or(0),
            total_score: raw.total_score.trim().parse::<f64>().unwrap_or(0.0),
            primary_language: raw.primary_language,
            description: raw.description,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "rows": [
                    {
                        "repo_name":        "owner/great-repo",
                        "stars":            "8500",
                        "forks":            "1200",
                        "pushes":           "42",
                        "total_score":      "0.85",
                        "primary_language": "Rust",
                        "description":      "A great repo"
                    },
                    {
                        "repo_name":        "owner/other-repo",
                        "stars":            "5000",
                        "forks":            "800",
                        "pushes":           "12",
                        "total_score":      "0.72",
                        "primary_language": "Go",
                        "description":      "Another"
                    }
                ]
            }
        })
    }

    fn client_pointing_at(server: &MockServer) -> OssInsightClient {
        OssInsightClient::new(
            OssInsightConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_trending_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/trends/repos/"))
            .and(query_param("period", "past_24_hours"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let repos = client
            .fetch_trending(TrendingPeriod::Past24Hours, None)
            .await
            .unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].repo_name, "owner/great-repo");
        assert_eq!(repos[0].stars, 8500);
        assert!((repos[0].total_score - 0.85).abs() < 1e-9);
        assert_eq!(repos[0].primary_language, "Rust");
    }

    #[tokio::test]
    async fn fetch_trending_passes_language_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/trends/repos/"))
            .and(query_param("language", "Rust"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let _ = client
            .fetch_trending(TrendingPeriod::PastWeek, Some("Rust"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn fetch_trending_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/trends/repos/"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_trending(TrendingPeriod::Past24Hours, None)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_trending_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/trends/repos/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_trending(TrendingPeriod::Past24Hours, None)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_trending_unreachable_yields_io() {
        let client = OssInsightClient::new(
            OssInsightConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_trending(TrendingPeriod::Past24Hours, None)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn period_slug_round_trips() {
        assert_eq!(TrendingPeriod::Past24Hours.slug(), "past_24_hours");
        assert_eq!(TrendingPeriod::PastWeek.slug(), "past_week");
        assert_eq!(TrendingPeriod::PastMonth.slug(), "past_month");
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = OssInsightConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
    }
}

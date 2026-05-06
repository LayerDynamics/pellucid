//! GDELT 2.0 DOC API client.
//!
//! GDELT publishes a JSON API at:
//!
//! ```text
//! GET https://api.gdeltproject.org/api/v2/doc/doc
//!     ?query=ukraine+protest
//!     &mode=ArtList
//!     &format=json
//!     &maxrecords=75
//!     &timespan=24h
//!     &sort=DateDesc
//! ```
//!
//! No auth required. The DOC 2.0 API surfaces individual
//! news articles GDELT has indexed in the last `timespan`
//! window, matching the supplied query (which uses GDELT's
//! own `field:value` operator vocabulary — `sourcecountry`,
//! `theme`, `actor1code`, etc.).
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "articles": [
//!     {
//!       "url":           "https://example.com/article",
//!       "url_mobile":    "",
//!       "title":         "Protest in Tehran...",
//!       "seendate":      "20260504T120000Z",
//!       "socialimage":   "https://...",
//!       "domain":        "example.com",
//!       "language":      "English",
//!       "sourcecountry": "Iran"
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — GDELT 2.0 production.
pub const DEFAULT_BASE_URL: &str = "https://api.gdeltproject.org";

/// Default per-request timeout — 15 s. GDELT can be slow
/// during peak news cycles.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct GdeltConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for GdeltConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable GDELT 2.0 DOC client.
#[derive(Clone, Debug)]
pub struct GdeltClient {
    http: reqwest::Client,
    config: GdeltConfig,
}

impl GdeltClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: GdeltConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = GdeltConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Search articles. `query` is a GDELT DOC query string
    /// (e.g. `"protest sourcecountry:IR"`), `timespan` matches
    /// GDELT's own format (`"24h"`, `"7d"`, `"1mo"`).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn search_articles(
        &self,
        query: &str,
        timespan: &str,
        max_records: u32,
    ) -> Result<Vec<GdeltArticle>, StreamsError> {
        let url = self.build_doc_url(query, timespan, max_records)?;
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
        let body: DocResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body
            .articles
            .into_iter()
            .map(GdeltArticle::from_raw)
            .collect())
    }

    fn build_doc_url(
        &self,
        query: &str,
        timespan: &str,
        max_records: u32,
    ) -> Result<Url, StreamsError> {
        let raw = format!("{}/api/v2/doc/doc", self.config.base_url);
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("gdelt url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("query", query);
            q.append_pair("mode", "ArtList");
            q.append_pair("format", "json");
            q.append_pair("maxrecords", &max_records.to_string());
            q.append_pair("timespan", timespan);
            q.append_pair("sort", "DateDesc");
        }
        Ok(url)
    }
}

/// One article row.
#[derive(Clone, Debug, PartialEq)]
pub struct GdeltArticle {
    /// Article URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// `YYYYMMDDTHHMMSSZ` timestamp the article was first
    /// indexed.
    pub seen_date: String,
    /// Social-share image URL (when present).
    pub social_image: String,
    /// Source domain.
    pub domain: String,
    /// Reported language.
    pub language: String,
    /// Source country.
    pub source_country: String,
}

#[derive(Debug, Deserialize)]
struct DocResponse {
    #[serde(default)]
    articles: Vec<RawArticle>,
}

#[derive(Debug, Default, Deserialize)]
struct RawArticle {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    seendate: String,
    #[serde(default)]
    socialimage: String,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    sourcecountry: String,
}

impl GdeltArticle {
    fn from_raw(raw: RawArticle) -> Self {
        Self {
            url: raw.url,
            title: raw.title,
            seen_date: raw.seendate,
            social_image: raw.socialimage,
            domain: raw.domain,
            language: raw.language,
            source_country: raw.sourcecountry,
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
            "articles": [
                {
                    "url":           "https://example.com/iran-protest",
                    "url_mobile":    "",
                    "title":         "Protest in Tehran tests new policy",
                    "seendate":      "20260504T120000Z",
                    "socialimage":   "https://example.com/img.png",
                    "domain":        "example.com",
                    "language":      "English",
                    "sourcecountry": "Iran"
                },
                {
                    "url":           "https://other.com/clash",
                    "title":         "Clashes near border",
                    "seendate":      "20260504T080000Z",
                    "domain":        "other.com",
                    "language":      "English",
                    "sourcecountry": "Iraq"
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> GdeltClient {
        GdeltClient::new(
            GdeltConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn search_articles_returns_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/doc/doc"))
            .and(query_param("query", "protest sourcecountry:IR"))
            .and(query_param("timespan", "24h"))
            .and(query_param("maxrecords", "75"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let articles = client
            .search_articles("protest sourcecountry:IR", "24h", 75)
            .await
            .unwrap();
        assert_eq!(articles.len(), 2);
        assert_eq!(articles[0].url, "https://example.com/iran-protest");
        assert_eq!(articles[0].source_country, "Iran");
        assert_eq!(articles[0].seen_date, "20260504T120000Z");
        assert_eq!(articles[1].source_country, "Iraq");
        // Missing socialimage on second article defaults to empty.
        assert_eq!(articles[1].social_image, "");
    }

    #[tokio::test]
    async fn search_articles_empty_articles_array_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/doc/doc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "articles": []
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let articles = client.search_articles("nothing", "24h", 75).await.unwrap();
        assert!(articles.is_empty());
    }

    #[tokio::test]
    async fn search_articles_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/doc/doc"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.search_articles("x", "24h", 75).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn search_articles_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/doc/doc"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.search_articles("x", "24h", 75).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn search_articles_unreachable_yields_io() {
        let client = GdeltClient::new(
            GdeltConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.search_articles("x", "24h", 75).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = GdeltConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}

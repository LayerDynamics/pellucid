//! HuggingFace Hub model-listing API client.
//!
//! Endpoint:
//! ```text
//! GET https://huggingface.co/api/models?sort=downloads&direction=-1&limit=20
//! ```
//!
//! Free, no auth. Returns model metadata including download
//! counts, like counts, last-modified timestamp, and the
//! library/pipeline tags.
//!
//! Response (relevant subset):
//! ```json
//! [
//!   {
//!     "id":          "meta-llama/Llama-3.1-8B-Instruct",
//!     "downloads":   12345678,
//!     "likes":       4500,
//!     "lastModified":"2026-04-25T08:00:00.000Z",
//!     "pipeline_tag":"text-generation",
//!     "library_name":"transformers",
//!     "tags":        ["text-generation", "llama", "..."]
//!   }
//! ]
//! ```

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — HuggingFace Hub production.
pub const DEFAULT_BASE_URL: &str = "https://huggingface.co";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Sort order for listing models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HfSortBy {
    /// Sort by `downloads` descending.
    Downloads,
    /// Sort by `likes` descending.
    Likes,
    /// Sort by `lastModified` descending.
    Trending,
}

impl HfSortBy {
    /// HuggingFace `sort` query value.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Downloads => "downloads",
            Self::Likes => "likes",
            Self::Trending => "lastModified",
        }
    }
}

/// Configuration.
#[derive(Clone, Debug)]
pub struct HuggingFaceConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for HuggingFaceConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable HuggingFace client.
#[derive(Clone, Debug)]
pub struct HuggingFaceClient {
    http: reqwest::Client,
    config: HuggingFaceConfig,
}

impl HuggingFaceClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: HuggingFaceConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = HuggingFaceConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch up to `limit` models, sorted by `sort_by`
    /// descending.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_models(
        &self,
        sort_by: HfSortBy,
        limit: u32,
    ) -> Result<Vec<HfModel>, StreamsError> {
        let url = self.build_url(sort_by, limit)?;
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
        let body: Vec<RawModel> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.into_iter().map(HfModel::from_raw).collect())
    }

    fn build_url(&self, sort_by: HfSortBy, limit: u32) -> Result<Url, StreamsError> {
        let raw = format!("{}/api/models", self.config.base_url);
        let mut url = Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("hf url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("sort", sort_by.slug());
            q.append_pair("direction", "-1");
            q.append_pair("limit", &limit.to_string());
        }
        Ok(url)
    }
}

/// One HuggingFace model row.
#[derive(Clone, Debug, PartialEq)]
pub struct HfModel {
    /// `<owner>/<name>` model id.
    pub id: String,
    /// All-time downloads.
    pub downloads: i64,
    /// Like count.
    pub likes: i64,
    /// ISO-8601 last-modified timestamp.
    pub last_modified: String,
    /// Pipeline tag (`text-generation`, `image-classification`, …).
    pub pipeline_tag: String,
    /// Library (`transformers`, `diffusers`, …).
    pub library_name: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    downloads: i64,
    #[serde(default)]
    likes: i64,
    #[serde(default, rename = "lastModified")]
    last_modified: String,
    #[serde(default)]
    pipeline_tag: String,
    #[serde(default)]
    library_name: String,
}

impl HfModel {
    fn from_raw(raw: RawModel) -> Self {
        Self {
            id: raw.id,
            downloads: raw.downloads,
            likes: raw.likes,
            last_modified: raw.last_modified,
            pipeline_tag: raw.pipeline_tag,
            library_name: raw.library_name,
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
        serde_json::json!([
            {
                "id":           "meta-llama/Llama-3.1-8B-Instruct",
                "downloads":    12_345_678_i64,
                "likes":        4500,
                "lastModified": "2026-04-25T08:00:00.000Z",
                "pipeline_tag": "text-generation",
                "library_name": "transformers"
            },
            {
                "id":           "stabilityai/stable-diffusion-3.5",
                "downloads":    8_000_000_i64,
                "likes":        2200,
                "lastModified": "2026-04-22T08:00:00.000Z",
                "pipeline_tag": "text-to-image",
                "library_name": "diffusers"
            }
        ])
    }

    fn client_pointing_at(server: &MockServer) -> HuggingFaceClient {
        HuggingFaceClient::new(
            HuggingFaceConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_models_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("sort", "downloads"))
            .and(query_param("direction", "-1"))
            .and(query_param("limit", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let models = client.fetch_models(HfSortBy::Downloads, 20).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "meta-llama/Llama-3.1-8B-Instruct");
        assert_eq!(models[0].downloads, 12_345_678);
        assert_eq!(models[0].pipeline_tag, "text-generation");
    }

    #[tokio::test]
    async fn fetch_models_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_models(HfSortBy::Downloads, 20)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_models_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_models(HfSortBy::Downloads, 20)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_models_unreachable_yields_io() {
        let client = HuggingFaceClient::new(
            HuggingFaceConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_models(HfSortBy::Downloads, 20)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn sort_slug_round_trips() {
        assert_eq!(HfSortBy::Downloads.slug(), "downloads");
        assert_eq!(HfSortBy::Likes.slug(), "likes");
        assert_eq!(HfSortBy::Trending.slug(), "lastModified");
    }
}

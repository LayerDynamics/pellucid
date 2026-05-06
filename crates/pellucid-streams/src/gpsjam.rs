//! gpsjam.org daily H3-hexagon GPS jamming data client.
//!
//! gpsjam.org publishes daily aggregated GPS-interference data
//! at a public CDN path:
//!
//! ```text
//! GET https://gpsjam.org/data/{YYYY}/{MM}/{DD}/h3_4.json
//! ```
//!
//! Response: a GeoJSON-style FeatureCollection where each
//! feature is one H3 level-4 hexagon with a `bad_pos_pct`
//! property in `[0.0, 1.0]` — the fraction of aircraft over
//! the cell that day that reported degraded GPS accuracy.
//!
//! Pellucid's airspace-restrictions panel renders the top-N
//! cells by `bad_pos_pct`. The seeder fetches the current day
//! (and falls back to the previous day during the early-UTC
//! window before today's file is published).

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — gpsjam.org public CDN.
pub const DEFAULT_BASE_URL: &str = "https://gpsjam.org";

/// Default per-request timeout — 15 s. The daily file is
/// 1-3 MB so we give it plenty of headroom on slow links.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct GpsjamConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for GpsjamConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable gpsjam.org client.
#[derive(Clone, Debug)]
pub struct GpsjamClient {
    http: reqwest::Client,
    config: GpsjamConfig,
}

impl GpsjamClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: GpsjamConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// Returns [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = GpsjamConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the daily H3 level-4 jamming hexagons for the
    /// supplied UTC date. `year/month/day` are 4-/2-/2-digit
    /// values per the upstream's path layout.
    ///
    /// Returns `Ok(None)` when the upstream returns 404 — the
    /// file may not yet exist for today before the daily
    /// rollover finishes (around 03:00 UTC). Callers should
    /// retry the previous day in that case.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses other
    ///   than 404.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_daily_h3(
        &self,
        year: u16,
        month: u8,
        day: u8,
    ) -> Result<Option<Vec<JammingCell>>, StreamsError> {
        let url = self.build_daily_url(year, month, day)?;
        let resp = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: FeatureCollection = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(Some(
            body.features
                .into_iter()
                .filter_map(JammingCell::from_feature)
                .collect(),
        ))
    }

    fn build_daily_url(&self, year: u16, month: u8, day: u8) -> Result<Url, StreamsError> {
        let raw = format!(
            "{}/data/{year:04}/{month:02}/{day:02}/h3_4.json",
            self.config.base_url
        );
        Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("gpsjam url: {e}")))
    }
}

/// One H3 level-4 cell with a degraded-position percentage.
#[derive(Clone, Debug, PartialEq)]
pub struct JammingCell {
    /// H3 cell identifier (15-character base-16 string).
    pub h3: String,
    /// Fraction of aircraft over the cell that day that
    /// reported degraded GPS accuracy. `[0.0, 1.0]`.
    pub bad_pos_pct: f64,
    /// Sample count (number of aircraft observations).
    pub samples: i64,
}

#[derive(Debug, Deserialize)]
struct FeatureCollection {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    #[serde(default)]
    properties: FeatureProps,
}

#[derive(Debug, Default, Deserialize)]
struct FeatureProps {
    #[serde(default)]
    h3: String,
    // The upstream reports `bad_pos_pct` as a fraction in [0,1].
    #[serde(default)]
    bad_pos_pct: f64,
    // Sample count — `n_samples` in the upstream.
    #[serde(default, rename = "n_samples")]
    samples: i64,
}

impl JammingCell {
    fn from_feature(f: Feature) -> Option<Self> {
        if f.properties.h3.is_empty() {
            return None;
        }
        Some(Self {
            h3: f.properties.h3,
            bad_pos_pct: f.properties.bad_pos_pct,
            samples: f.properties.samples,
        })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "geometry": { "type": "Polygon", "coordinates": [] },
                    "properties": {
                        "h3":          "84283483fffffff",
                        "bad_pos_pct": 0.42,
                        "n_samples":   123
                    }
                },
                {
                    "type": "Feature",
                    "geometry": { "type": "Polygon", "coordinates": [] },
                    "properties": {
                        "h3":          "84283487fffffff",
                        "bad_pos_pct": 0.05,
                        "n_samples":   45
                    }
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> GpsjamClient {
        GpsjamClient::new(
            GpsjamConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_daily_h3_returns_two_cells() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data/2026/05/04/h3_4.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let cells = client.fetch_daily_h3(2026, 5, 4).await.unwrap().unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].h3, "84283483fffffff");
        assert!((cells[0].bad_pos_pct - 0.42).abs() < 1e-9);
        assert_eq!(cells[0].samples, 123);
    }

    #[tokio::test]
    async fn fetch_daily_h3_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data/2026/05/04/h3_4.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let res = client.fetch_daily_h3(2026, 5, 4).await.unwrap();
        assert!(res.is_none(), "404 must be Ok(None) for retry-yesterday");
    }

    #[tokio::test]
    async fn fetch_daily_h3_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data/2026/05/04/h3_4.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_daily_h3(2026, 5, 4).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_daily_h3_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data/2026/05/04/h3_4.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_daily_h3(2026, 5, 4).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_daily_h3_drops_features_without_h3_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data/2026/05/04/h3_4.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "features": [
                    { "properties": { "h3": "ok", "bad_pos_pct": 0.1, "n_samples": 10 } },
                    { "properties": { "bad_pos_pct": 0.5 } }
                ]
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let cells = client.fetch_daily_h3(2026, 5, 4).await.unwrap().unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].h3, "ok");
    }

    #[tokio::test]
    async fn fetch_daily_h3_unreachable_yields_io() {
        let client = GpsjamClient::new(
            GpsjamConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_daily_h3(2026, 5, 4).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[tokio::test]
    async fn url_zero_pads_month_and_day() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data/2026/01/02/h3_4.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let cells = client.fetch_daily_h3(2026, 1, 2).await.unwrap().unwrap();
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn config_default_uses_production_base_url() {
        let cfg = GpsjamConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}

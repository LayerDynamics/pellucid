//! CoinGecko Public API v3 client — universal crypto price source.
//!
//! Endpoint shape:
//! ```text
//! GET https://api.coingecko.com/api/v3/simple/price
//!     ?ids=bitcoin,ethereum,solana
//!     &vs_currencies=usd
//!     &include_24hr_change=true
//!     &include_market_cap=true
//!     &include_last_updated_at=true
//! ```
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "bitcoin":  { "usd": 67234.12, "usd_market_cap": 1.32e12,
//!                 "usd_24h_change": 1.84, "last_updated_at": 1714060800 },
//!   "ethereum": { "usd": 3450.55,  "usd_market_cap": 4.13e11,
//!                 "usd_24h_change": -0.42, "last_updated_at": 1714060800 }
//! }
//! ```
//!
//! The free public API tier:
//! - No key required.
//! - 30 requests/minute soft limit (Demo plan key gives 30
//!   req/min too — both share the same bucket).
//! - 401/429 responses are surfaced as [`StreamsError::Status`]
//!   so the caller can back off.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — `https://api.coingecko.com/api/v3`.
pub const DEFAULT_BASE_URL: &str = "https://api.coingecko.com/api/v3";

/// Default per-request timeout — 6 s. CoinGecko's `simple/price`
/// usually responds in < 300 ms; 6 s tolerates the occasional
/// 1-2 s spike under load.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);

/// Default user-agent. CoinGecko accepts unmarked clients but
/// some Cloudflare paths between us and them treat the default
/// `reqwest/x.y` UA poorly; sending a clear app-name UA avoids
/// the worst-case 403s.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0";

/// Configuration for the CoinGecko client.
#[derive(Clone, Debug)]
pub struct CoinGeckoConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
    /// Optional Demo plan API key — sent as
    /// `x-cg-demo-api-key` when present. Production deployments
    /// can run without it; setting one bumps the rate limit
    /// floor and avoids shared-IP throttles.
    pub demo_api_key: Option<String>,
}

impl Default for CoinGeckoConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
            demo_api_key: None,
        }
    }
}

/// Pluggable CoinGecko v3 client.
#[derive(Clone, Debug)]
pub struct CoinGeckoClient {
    http: reqwest::Client,
    config: CoinGeckoConfig,
}

impl CoinGeckoClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: CoinGeckoConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor: production base URL with a
    /// freshly-built `reqwest::Client`.
    ///
    /// # Errors
    /// Returns [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = CoinGeckoConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch USD-denominated quotes for `ids` (e.g.
    /// `["bitcoin", "ethereum", "solana"]`). Returns one
    /// [`CryptoQuote`] per requested id that the upstream
    /// returned a row for; missing ids are silently dropped
    /// (CoinGecko's `simple/price` already discards unknown
    /// ids, so the caller can compare lengths to detect drift).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] on transport failures.
    /// - [`StreamsError::Status`] on non-2xx (notably 429).
    /// - [`StreamsError::Parse`] on body shape mismatch.
    pub async fn fetch_simple_price(&self, ids: &[&str]) -> Result<Vec<CryptoQuote>, StreamsError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.build_simple_price_url(ids)?;
        let mut req = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json");
        if let Some(key) = self.config.demo_api_key.as_deref() {
            req = req.header("x-cg-demo-api-key", key);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        let map = body
            .as_object()
            .ok_or_else(|| StreamsError::Parse("expected top-level object".into()))?;

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(row) = map.get(*id) {
                out.push(parse_quote(id, row)?);
            }
        }
        Ok(out)
    }

    fn build_simple_price_url(&self, ids: &[&str]) -> Result<Url, StreamsError> {
        let raw = format!("{}/simple/price", self.config.base_url);
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("coingecko url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("ids", &ids.join(","));
            q.append_pair("vs_currencies", "usd");
            q.append_pair("include_24hr_change", "true");
            q.append_pair("include_market_cap", "true");
            q.append_pair("include_last_updated_at", "true");
        }
        Ok(url)
    }
}

/// One CoinGecko price row.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CryptoQuote {
    /// CoinGecko id (e.g. `"bitcoin"`).
    pub id: String,
    /// Price in USD.
    pub usd: f64,
    /// 24-hour percent change vs USD. CoinGecko returns this
    /// signed (negative for losses).
    pub usd_24h_change: f64,
    /// Market capitalisation in USD. 0.0 when CoinGecko omits it
    /// (some long-tail tokens have no MC).
    pub usd_market_cap: f64,
    /// Wall-clock seconds when CoinGecko stamped this row.
    pub last_updated_at: i64,
}

fn parse_quote(id: &str, row: &Value) -> Result<CryptoQuote, StreamsError> {
    let obj = row
        .as_object()
        .ok_or_else(|| StreamsError::Parse(format!("{id}: expected object")))?;
    let usd = obj
        .get("usd")
        .and_then(Value::as_f64)
        .ok_or_else(|| StreamsError::Parse(format!("{id}: missing usd")))?;
    Ok(CryptoQuote {
        id: id.to_string(),
        usd,
        usd_24h_change: obj
            .get("usd_24h_change")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        usd_market_cap: obj
            .get("usd_market_cap")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        last_updated_at: obj
            .get("last_updated_at")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "bitcoin":  { "usd": 67234.12, "usd_market_cap": 1.32e12,
                          "usd_24h_change": 1.84, "last_updated_at": 1714060800 },
            "ethereum": { "usd": 3450.55,  "usd_market_cap": 4.13e11,
                          "usd_24h_change": -0.42, "last_updated_at": 1714060800 }
        })
    }

    fn client_pointing_at(server: &MockServer) -> CoinGeckoClient {
        CoinGeckoClient::new(
            CoinGeckoConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
                demo_api_key: None,
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_simple_price_maps_two_ids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .and(query_param("ids", "bitcoin,ethereum"))
            .and(query_param("vs_currencies", "usd"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let quotes = client
            .fetch_simple_price(&["bitcoin", "ethereum"])
            .await
            .unwrap();
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[0].id, "bitcoin");
        assert!((quotes[0].usd - 67234.12).abs() < 1e-6);
        assert!((quotes[0].usd_24h_change - 1.84).abs() < 1e-9);
        assert_eq!(quotes[0].last_updated_at, 1714060800);
        assert_eq!(quotes[1].id, "ethereum");
        assert!((quotes[1].usd - 3450.55).abs() < 1e-6);
    }

    #[tokio::test]
    async fn fetch_simple_price_drops_missing_ids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let quotes = client
            .fetch_simple_price(&["bitcoin", "ethereum", "doge-not-listed"])
            .await
            .unwrap();
        assert_eq!(quotes.len(), 2, "missing id silently dropped");
        let ids: Vec<_> = quotes.iter().map(|q| q.id.as_str()).collect();
        assert!(ids.contains(&"bitcoin"));
        assert!(ids.contains(&"ethereum"));
    }

    #[tokio::test]
    async fn fetch_simple_price_empty_ids_returns_empty_vec() {
        let server = MockServer::start().await;
        // Server should NEVER be called for an empty `ids`.
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        assert!(client.fetch_simple_price(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_simple_price_429_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_simple_price(&["bitcoin"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 429 }));
    }

    #[tokio::test]
    async fn fetch_simple_price_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json {{"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_simple_price(&["bitcoin"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_simple_price_missing_usd_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "bitcoin": { "eur": 60000.0 }
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_simple_price(&["bitcoin"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn demo_api_key_when_set_is_sent_as_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/price"))
            .and(wiremock::matchers::header(
                "x-cg-demo-api-key",
                "demo-key-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = CoinGeckoClient::new(
            CoinGeckoConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "x".into(),
                demo_api_key: Some("demo-key-1".into()),
            },
            reqwest::Client::new(),
        );
        let quotes = client.fetch_simple_price(&["bitcoin"]).await.unwrap();
        assert_eq!(quotes.len(), 1);
    }

    #[tokio::test]
    async fn fetch_simple_price_unreachable_yields_io() {
        let client = CoinGeckoClient::new(
            CoinGeckoConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
                demo_api_key: None,
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_simple_price(&["bitcoin"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)), "got {err:?}");
    }

    #[test]
    fn config_default_uses_production_base_url() {
        let cfg = CoinGeckoConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
        assert!(cfg.demo_api_key.is_none());
    }
}

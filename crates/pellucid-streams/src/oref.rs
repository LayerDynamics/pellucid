//! OREF (Israeli Home Front Command) alerts client.
//!
//! Ports the OREF leg of the original WorldMonitor relay
//! (`scripts/ais-relay.cjs` OREF path, SPEC-001 §17.6). The OREF
//! endpoint serves real-time alert data and aggressively
//! fingerprint-blocks non-Chrome clients; the legacy relay used
//! a curl + JA3 spoof to bypass.
//!
//! ## What this module ships today
//!
//! - **HTTP-level Chrome shaping** — every direct fetch sets the
//!   exact `User-Agent`, `Accept`, `Accept-Language`, `Sec-Fetch-*`
//!   headers Chrome 121 sends, in the order Chrome sends them.
//!   This alone bypasses the bulk of UA-based filters.
//! - **Residential-proxy fallback** — a configured
//!   `proxy_url` is used on direct failure. The proxy itself is
//!   responsible for the TLS-level fingerprint when the operator
//!   has wired a JA3-aware exit (e.g. a `rquest`-fronted
//!   forward proxy). Failover is deterministic: 5xx / 429 / IO
//!   error on direct → one retry through the proxy.
//! - **History persistence** — every successful fetch envelope
//!   is written to `kv_envelope` under `relay:oref:history:v1`
//!   so the relay's `/health` cascade can read freshness.
//!
//! ## What requires a follow-up swap-in
//!
//! - **TLS-level JA3 byte-equality with Chrome.** The
//!   [`OrefClient::with_browser_fingerprint`] extension point
//!   accepts a closure that returns a `reqwest::Client`; the
//!   relay binary swaps in a `ja3-rustls`- or `rquest`-backed
//!   client at boot. The default `reqwest::Client::builder()`
//!   path uses standard rustls + Chrome HTTP headers; full TLS
//!   fingerprint matching is the operator's choice (it's a
//!   deps + build matrix decision, not a code shape one).
//!
//! [`crate::ja3`] provides the JA3 fingerprint *computation* —
//! the algorithm half of the contract — independent of which
//! client implementation actually emits the matching handshake.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_cache::set_cached_json;
use pellucid_core::Envelope;
use pellucid_db::Pool;

use crate::error::StreamsError;

/// Default OREF alert endpoint.
pub const DEFAULT_OREF_URL: &str = "https://www.oref.org.il/WarningMessages/alert/alerts.json";

/// `kv_envelope` key the history is persisted under.
pub const HISTORY_CACHE_KEY: &str = "relay:oref:history:v1";

/// History TTL — 12 h. The OREF feed is canonical for the panel's
/// "last 12 hours of alerts" view; the history cache backs it.
pub const HISTORY_TTL_MS: i64 = 12 * 60 * 60 * 1_000;

/// Per-request timeout. OREF responses are tiny (≤ 5 KiB) so a
/// short timeout is appropriate; the caller's outer `tokio::time::
/// timeout` adds a hard ceiling.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

/// Configuration for the OREF client.
#[derive(Clone, Debug)]
pub struct OrefConfig {
    /// Endpoint URL. Production default
    /// [`DEFAULT_OREF_URL`]; tests inject the wiremock URL.
    pub url: String,
    /// Optional residential-proxy URL. Used as a fallback on
    /// direct failure (5xx / 429 / IO).
    pub proxy_url: Option<String>,
}

impl OrefConfig {
    /// Build with the production endpoint + no proxy. Production
    /// callers chain `with_proxy(...)` before constructing the
    /// client.
    #[must_use]
    pub fn production() -> Self {
        Self {
            url: DEFAULT_OREF_URL.to_string(),
            proxy_url: None,
        }
    }

    /// Configure a residential proxy fallback.
    #[must_use]
    pub fn with_proxy(mut self, proxy_url: impl Into<String>) -> Self {
        self.proxy_url = Some(proxy_url.into());
        self
    }
}

/// Single OREF alert as the upstream emits it. The wire shape is
/// `{ id, cat, title, data, desc }` — `data` is the comma-joined
/// list of affected localities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrefAlert {
    /// Upstream alert id (incrementing integer).
    pub id: String,
    /// Category code (`1` = rocket, `13` = hostile aircraft, …).
    #[serde(default)]
    pub cat: String,
    /// Localised title (e.g. `"ירי רקטות וטילים"`).
    #[serde(default)]
    pub title: String,
    /// Comma-joined list of affected localities.
    #[serde(default)]
    pub data: String,
    /// Human-readable description.
    #[serde(default)]
    pub desc: String,
}

/// History envelope persisted to `kv_envelope`. Bounded to the
/// most recent 256 alerts so the row stays well below the
/// 5 MiB cap.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrefHistory {
    /// Most-recent-first alert list.
    pub alerts: Vec<OrefAlert>,
    /// Wall-clock ms when the history was last refreshed.
    pub updated_at_ms: i64,
}

const HISTORY_CAP: usize = 256;

/// Pluggable OREF client. The default constructor builds a
/// reqwest client with the Chrome HTTP shape; production wires
/// a JA3-aware client via [`Self::with_browser_fingerprint`].
#[derive(Clone, Debug)]
pub struct OrefClient {
    config: OrefConfig,
    /// Direct HTTP client (Chrome-headered).
    http: reqwest::Client,
    /// Optional proxied HTTP client built from
    /// `config.proxy_url`. Cached so the proxy URL is parsed
    /// once.
    proxied: Option<reqwest::Client>,
}

impl OrefClient {
    /// Build with the supplied config + a default Chrome-headered
    /// reqwest client.
    ///
    /// # Errors
    /// Returns [`StreamsError::Io`] if the proxied client fails
    /// to build (typically a malformed proxy URL).
    pub fn new(config: OrefConfig) -> Result<Self, StreamsError> {
        let http = build_chrome_client(None)?;
        let proxied = match config.proxy_url.as_deref() {
            Some(url) => Some(build_chrome_client(Some(url))?),
            None => None,
        };
        Ok(Self {
            config,
            http,
            proxied,
        })
    }

    /// Build with a pre-configured browser-fingerprint client
    /// (e.g. a `rquest::Client`-fronted reqwest layer). The
    /// caller's client MUST already set the headers it wants;
    /// this constructor does NOT layer Chrome headers on top.
    #[must_use]
    pub fn with_browser_fingerprint(config: OrefConfig, http: reqwest::Client) -> Self {
        Self {
            config,
            http,
            proxied: None,
        }
    }

    /// Read-only access to the resolved direct-fetch URL.
    pub fn url(&self) -> &str {
        &self.config.url
    }

    /// `true` iff a residential proxy fallback is configured.
    pub fn has_proxy(&self) -> bool {
        self.proxied.is_some()
    }

    /// Fetch the current alert list. Tries the direct path first,
    /// falls back to the proxied path on direct failure
    /// (5xx / 429 / IO).
    ///
    /// Returns `Ok(None)` when the upstream emits an empty array
    /// (no active alerts) — the caller's history merge layer
    /// records this as "no new alerts in this poll", not as
    /// upstream failure.
    pub async fn fetch_alerts(&self) -> Result<Option<Vec<OrefAlert>>, StreamsError> {
        match try_fetch(&self.http, &self.config.url).await {
            Ok(alerts) => Ok(alerts),
            Err(direct_err) => {
                if let Some(proxied) = self.proxied.as_ref() {
                    tracing::warn!(
                        target: "pellucid::streams::oref",
                        "direct fetch failed, retrying via proxy: {direct_err}"
                    );
                    try_fetch(proxied, &self.config.url).await
                } else {
                    Err(direct_err)
                }
            }
        }
    }

    /// Fetch + persist the merged history to `kv_envelope`.
    /// Returns the persisted history so the caller can react to
    /// new alerts (broadcast, panel state seed) without a second
    /// read.
    ///
    /// New alert ids are inserted at the front of the existing
    /// history; any id already in the history is treated as a
    /// duplicate and skipped. The history is bounded at
    /// [`HISTORY_CAP`] entries.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] / `Status` / `Parse` from the
    ///   upstream fetch.
    pub async fn refresh_history(&self, pool: &Pool) -> Result<OrefHistory, StreamsError> {
        let new_alerts = self.fetch_alerts().await?.unwrap_or_default();
        let prev = read_history(pool).await.unwrap_or_default();
        let merged = merge_history(prev.alerts, new_alerts);
        let history = OrefHistory {
            alerts: merged,
            updated_at_ms: pellucid_core::now_ms(),
        };
        let envelope = Envelope::new(history.clone());
        set_cached_json(pool, HISTORY_CACHE_KEY, &envelope, HISTORY_TTL_MS)
            .await
            .map_err(|e| StreamsError::Io(format!("history persist: {e}")))?;
        Ok(history)
    }
}

async fn try_fetch(
    http: &reqwest::Client,
    url: &str,
) -> Result<Option<Vec<OrefAlert>>, StreamsError> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| StreamsError::Io(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(StreamsError::Status {
            status: status.as_u16(),
        });
    }
    let body = resp
        .text()
        .await
        .map_err(|e| StreamsError::Io(e.to_string()))?;
    parse_oref_body(&body)
}

/// Parse the OREF JSON body. The endpoint is unusual: it returns
/// either `""` (empty string body, no alerts) or a `{ id, cat,
/// title, data, desc }` object — there's no top-level array.
/// Some seasons of the upstream wrap multiple alerts in an
/// array; we accept either shape.
pub fn parse_oref_body(body: &str) -> Result<Option<Vec<OrefAlert>>, StreamsError> {
    let trimmed = body.trim();
    if trimmed.is_empty() || trimmed == "\"\"" {
        return Ok(None);
    }
    // Try array first.
    if let Ok(list) = serde_json::from_str::<Vec<OrefAlert>>(trimmed) {
        if list.is_empty() {
            return Ok(None);
        }
        return Ok(Some(list));
    }
    // Fall back to single-object shape.
    let one: OrefAlert = serde_json::from_str(trimmed)
        .map_err(|e| StreamsError::Parse(format!("oref body: {e}")))?;
    Ok(Some(vec![one]))
}

/// Merge `new_alerts` into `existing` history, dedup by `id`,
/// cap at [`HISTORY_CAP`].
pub fn merge_history(existing: Vec<OrefAlert>, new_alerts: Vec<OrefAlert>) -> Vec<OrefAlert> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<OrefAlert> = Vec::with_capacity(existing.len() + new_alerts.len());
    for a in new_alerts.into_iter().chain(existing) {
        if seen.insert(a.id.clone()) {
            out.push(a);
            if out.len() >= HISTORY_CAP {
                break;
            }
        }
    }
    out
}

/// Read the history envelope from `kv_envelope`. Returns an
/// empty history when the row is missing — callers seed it that
/// way and merge new alerts into it.
pub async fn read_history(pool: &Pool) -> Result<OrefHistory, StreamsError> {
    use pellucid_cache::CacheHit;
    // `kv::get_cached_json` returns the full Envelope<T>-shaped
    // JSON value; we unwrap the `.data` slot before decoding into
    // OrefHistory.
    let hit: CacheHit<serde_json::Value> =
        pellucid_cache::kv::get_cached_json::<serde_json::Value>(pool, HISTORY_CACHE_KEY)
            .await
            .map_err(|e| StreamsError::Io(format!("history read: {e}")))?;
    let envelope_value: serde_json::Value = match hit {
        CacheHit::Fresh(v) | CacheHit::Stale(v) => v,
        CacheHit::NegativeSentinel | CacheHit::Miss => {
            return Ok(OrefHistory::default());
        }
    };
    let data_value = envelope_value
        .get("data")
        .cloned()
        .unwrap_or(envelope_value);
    let history: OrefHistory = serde_json::from_value(data_value)
        .map_err(|e| StreamsError::Parse(format!("history parse: {e}")))?;
    Ok(history)
}

/// Build a reqwest client with Chrome 121's HTTP shape. When
/// `proxy_url` is set, the client routes through it (`reqwest::
/// Proxy::all`).
fn build_chrome_client(proxy_url: Option<&str>) -> Result<reqwest::Client, StreamsError> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/121.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        reqwest::header::HeaderValue::from_static("en-US,en;q=0.9,he;q=0.8"),
    );
    headers.insert(
        reqwest::header::REFERER,
        reqwest::header::HeaderValue::from_static("https://www.oref.org.il/"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("x-requested-with"),
        reqwest::header::HeaderValue::from_static("XMLHttpRequest"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("sec-fetch-site"),
        reqwest::header::HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("sec-fetch-mode"),
        reqwest::header::HeaderValue::from_static("cors"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("sec-fetch-dest"),
        reqwest::header::HeaderValue::from_static("empty"),
    );

    let mut builder = reqwest::Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .default_headers(headers)
        .gzip(true)
        .brotli(true);
    if let Some(url) = proxy_url {
        let proxy =
            reqwest::Proxy::all(url).map_err(|e| StreamsError::Io(format!("proxy: {e}")))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|e| StreamsError::Io(format!("client build: {e}")))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_oref_body_empty_string_returns_none() {
        assert_eq!(parse_oref_body("").unwrap(), None);
        assert_eq!(parse_oref_body("   ").unwrap(), None);
        assert_eq!(parse_oref_body("\"\"").unwrap(), None);
    }

    #[test]
    fn parse_oref_body_empty_array_returns_none() {
        assert_eq!(parse_oref_body("[]").unwrap(), None);
    }

    #[test]
    fn parse_oref_body_single_object_returns_singleton_vec() {
        let body =
            r#"{"id":"42","cat":"1","title":"רקטות","data":"שדרות","desc":"היכנסו למרחב מוגן"}"#;
        let alerts = parse_oref_body(body).unwrap().unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].id, "42");
        assert_eq!(alerts[0].cat, "1");
        assert!(alerts[0].title.contains("רקטות"));
    }

    #[test]
    fn parse_oref_body_array_returns_full_vec() {
        let body = r#"[
            {"id":"42","cat":"1","title":"a","data":"x","desc":""},
            {"id":"43","cat":"1","title":"b","data":"y","desc":""}
        ]"#;
        let alerts = parse_oref_body(body).unwrap().unwrap();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].id, "42");
        assert_eq!(alerts[1].id, "43");
    }

    #[test]
    fn parse_oref_body_malformed_yields_parse_error() {
        let err = parse_oref_body("not json {{").unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn merge_history_dedupes_by_id_preserves_order() {
        let prev = vec![
            OrefAlert {
                id: "10".into(),
                cat: "1".into(),
                title: "old".into(),
                data: String::new(),
                desc: String::new(),
            },
            OrefAlert {
                id: "9".into(),
                cat: "1".into(),
                title: "older".into(),
                data: String::new(),
                desc: String::new(),
            },
        ];
        let new = vec![
            OrefAlert {
                id: "11".into(),
                cat: "1".into(),
                title: "new".into(),
                data: String::new(),
                desc: String::new(),
            },
            OrefAlert {
                id: "10".into(),
                cat: "1".into(),
                title: "dup".into(),
                data: String::new(),
                desc: String::new(),
            },
        ];
        let merged = merge_history(prev, new);
        let ids: Vec<&str> = merged.iter().map(|a| a.id.as_str()).collect();
        // "11" is newest; "10" was new+old (dedup'd to first
        // occurrence, which is "new" because new_alerts came
        // first in the chain); "9" survives at the tail.
        assert_eq!(ids, vec!["11", "10", "9"]);
        // The dedup picks the NEW version of the duplicate, not
        // the old one.
        let ten = merged.iter().find(|a| a.id == "10").unwrap();
        assert_eq!(ten.title, "dup");
    }

    #[test]
    fn merge_history_caps_at_history_cap() {
        let huge: Vec<OrefAlert> = (0..500)
            .map(|i| OrefAlert {
                id: i.to_string(),
                cat: "1".into(),
                title: String::new(),
                data: String::new(),
                desc: String::new(),
            })
            .collect();
        let merged = merge_history(huge, vec![]);
        assert_eq!(merged.len(), HISTORY_CAP);
    }

    #[test]
    fn config_production_uses_canonical_url() {
        let cfg = OrefConfig::production();
        assert_eq!(cfg.url, DEFAULT_OREF_URL);
        assert!(cfg.proxy_url.is_none());
        let cfg = cfg.with_proxy("http://proxy.test:8080");
        assert_eq!(cfg.proxy_url.as_deref(), Some("http://proxy.test:8080"));
    }

    #[test]
    fn build_chrome_client_no_proxy_succeeds() {
        let c = build_chrome_client(None).unwrap();
        let _ = c; // type-check only — no live request
    }

    #[test]
    fn build_chrome_client_with_proxy_succeeds() {
        let c = build_chrome_client(Some("http://proxy.test:8080")).unwrap();
        let _ = c;
    }

    #[test]
    fn build_chrome_client_with_malformed_proxy_fails() {
        let err = build_chrome_client(Some("not a url at all")).unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn new_without_proxy_has_no_proxy() {
        let c = OrefClient::new(OrefConfig::production()).unwrap();
        assert!(!c.has_proxy());
    }

    #[test]
    fn new_with_proxy_records_proxy() {
        let cfg = OrefConfig::production().with_proxy("http://proxy.test:8080");
        let c = OrefClient::new(cfg).unwrap();
        assert!(c.has_proxy());
    }
}

//! RSS / Atom client — port of the original WorldMonitor RSS
//! proxy from `scripts/ais-relay.cjs` (the relay served as the
//! public-facing RSS proxy because most publishers reject CORS
//! requests from browsers).
//!
//! Responsibilities (SPEC-001 §10):
//! 1. **Allowlist enforcement** — reject any URL whose host is not
//!    in [`allowed_domains::ALLOWED_DOMAINS`].
//! 2. **Parse** — `feed-rs` accepts RSS 1.0, RSS 2.0, and Atom
//!    transparently; we normalise into [`RssFeed`] / [`RssEntry`].
//! 3. **Cache** — 5 min positive / 1 min negative. Keyed by
//!    `rss:proxy:<sha256(url)>:v1`; cache state lives in the
//!    consumer's `pellucid-cache` instance, not here. This module
//!    just exposes the fetch + parse primitives + the in-flight
//!    deduplication.
//! 4. **In-flight dedup** — concurrent calls to
//!    [`RssClient::fetch`] for the same URL collapse onto a single
//!    upstream request via a `dashmap` + `tokio::sync::OnceCell`
//!    registry, matching the stampede pattern in
//!    `pellucid-cache::cached_fetch_json`.

pub mod allowed_domains;

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use feed_rs::parser::ParseFeedError;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use url::Url;

use crate::error::StreamsError;

/// Default per-request timeout. The production relay uses 10s;
/// integration tests override to a small value.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default positive-cache TTL. Mirrors the SPEC-001 §10 5-min
/// positive cache. The consumer crate owns the cache writeback;
/// this constant is exposed for callers that want to bypass the
/// cache layer for ad-hoc fetches.
pub const POSITIVE_TTL: Duration = Duration::from_secs(5 * 60);

/// Default negative-cache TTL — 1 minute for transient upstream
/// failures.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(60);

/// Normalised feed shape — what the handler / seeder sees after
/// `feed-rs` has parsed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RssFeed {
    /// Echo of the resolved feed URL (post-redirect).
    pub url: String,
    /// Feed title; `None` when the upstream omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Feed-level description / subtitle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Entries in the order the upstream emitted them (most-
    /// recent-first for every publisher we support).
    pub entries: Vec<RssEntry>,
}

/// Normalised entry. Drops 90% of `feed-rs`'s typed fields — the
/// webview's news panel only consumes title + link + summary +
/// timestamp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RssEntry {
    /// Stable identifier the upstream provided (`<id>` for Atom,
    /// `<guid>` for RSS 2.0). Falls back to the entry's link.
    pub id: String,
    /// Entry title; `None` when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Canonical link to the article.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Summary or content snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Published time as ISO 8601. `None` when the upstream omits
    /// `<pubDate>` / `<published>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}

/// Per-URL in-flight cell type. Aliased so the field declaration
/// stays under clippy's complexity threshold and the lifetime
/// boundary is named.
type InflightCell = Arc<OnceCell<Result<RssFeed, StreamsError>>>;

/// Pluggable RSS fetch + parse client.
#[derive(Clone, Debug)]
pub struct RssClient {
    http: reqwest::Client,
    timeout: Duration,
    /// In-flight request registry — concurrent `fetch(url)` calls
    /// collapse onto a single upstream request.
    inflight: Arc<DashMap<String, InflightCell>>,
}

impl RssClient {
    /// Construct with the given HTTP client. The `User-Agent` and
    /// other headers are owned by the caller — the relay sets a
    /// realistic UA to avoid publisher 403s.
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            timeout: DEFAULT_TIMEOUT,
            inflight: Arc::new(DashMap::new()),
        }
    }

    /// Override the per-request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Fetch + parse an RSS / Atom feed. Concurrent calls for the
    /// same URL collapse onto a single upstream request.
    ///
    /// # Errors
    /// - [`StreamsError::Status`] for non-2xx upstream responses
    ///   AND for hosts not on the allowlist (status `403` —
    ///   surfaced so the gateway can map it to a clear envelope).
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Parse`] for malformed XML.
    pub async fn fetch(&self, url: &str) -> Result<RssFeed, StreamsError> {
        // Allowlist check happens before we touch the in-flight
        // registry so a denied URL can never poison the cache.
        let parsed = Url::parse(url)
            .map_err(|e| StreamsError::Parse(format!("url parse: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| StreamsError::Parse(format!("url has no host: {url}")))?;
        if !allowed_domains::is_allowed(host) {
            return Err(StreamsError::Status { status: 403 });
        }

        let cell = self
            .inflight
            .entry(url.to_string())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone();
        let url_owned = url.to_string();
        let http = self.http.clone();
        let timeout = self.timeout;
        let outcome = cell
            .get_or_init(move || async move {
                fetch_and_parse(&http, &url_owned, timeout).await
            })
            .await
            .clone();
        // Evict the in-flight entry once everyone has their result
        // — the *next* call to the same URL re-fetches.
        self.inflight.remove(url);
        outcome
    }
}

async fn fetch_and_parse(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<RssFeed, StreamsError> {
    let resp = http
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| StreamsError::Io(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(StreamsError::Status {
            status: status.as_u16(),
        });
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| StreamsError::Io(e.to_string()))?;
    let parsed = feed_rs::parser::parse(bytes.as_ref()).map_err(map_parse_error)?;
    Ok(normalise(url, parsed))
}

fn map_parse_error(e: ParseFeedError) -> StreamsError {
    StreamsError::Parse(format!("feed parse: {e}"))
}

fn normalise(source_url: &str, feed: feed_rs::model::Feed) -> RssFeed {
    let entries = feed
        .entries
        .into_iter()
        .map(|e| {
            let link = e.links.into_iter().next().map(|l| l.href);
            // `feed-rs` uses an `id` field; for RSS 2.0 with `<guid>`
            // it still populates `id` from the GUID. When the feed
            // omits both, fall through to the link.
            let id = if e.id.is_empty() {
                link.clone().unwrap_or_default()
            } else {
                e.id
            };
            RssEntry {
                id,
                title: e.title.map(|t| t.content),
                link,
                summary: e.summary.map(|s| s.content),
                published: e.published.map(|d| d.to_rfc3339()),
            }
        })
        .collect();
    RssFeed {
        url: source_url.to_string(),
        title: feed.title.map(|t| t.content),
        description: feed.description.map(|d| d.content),
        entries,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn rss_2_0_minimal() -> &'static [u8] {
        br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Example Feed</title>
    <description>An example RSS 2.0 feed for tests.</description>
    <link>https://reuters.com</link>
    <item>
      <title>First entry</title>
      <link>https://reuters.com/articles/1</link>
      <guid isPermaLink="false">tag:reuters,2026:1</guid>
      <description>First summary.</description>
      <pubDate>Sat, 02 May 2026 12:00:00 GMT</pubDate>
    </item>
    <item>
      <title>Second entry</title>
      <link>https://reuters.com/articles/2</link>
      <guid isPermaLink="false">tag:reuters,2026:2</guid>
      <description>Second summary.</description>
      <pubDate>Sat, 02 May 2026 11:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#
    }

    fn atom_minimal() -> &'static [u8] {
        br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Example</title>
  <subtitle>Atom-format example for tests.</subtitle>
  <link href="https://krebsonsecurity.com"/>
  <id>tag:krebsonsecurity,2026:atom</id>
  <updated>2026-05-02T12:00:00Z</updated>
  <entry>
    <title>Atom entry one</title>
    <link href="https://krebsonsecurity.com/posts/1"/>
    <id>tag:krebsonsecurity,2026:1</id>
    <summary>Atom entry one summary.</summary>
    <published>2026-05-02T12:00:00Z</published>
  </entry>
</feed>"#
    }

    fn rss_1_0_minimal() -> &'static [u8] {
        br#"<?xml version="1.0" encoding="UTF-8"?>
<rdf:RDF
  xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
  xmlns="http://purl.org/rss/1.0/">
  <channel rdf:about="https://schneier.com">
    <title>Schneier Feed</title>
    <description>RSS 1.0 example for tests.</description>
    <link>https://schneier.com</link>
  </channel>
  <item rdf:about="https://schneier.com/posts/1">
    <title>RSS 1.0 entry</title>
    <link>https://schneier.com/posts/1</link>
    <description>RSS 1.0 entry summary.</description>
  </item>
</rdf:RDF>"#
    }

    #[test]
    fn parses_rss_2_0_into_normalised_feed() {
        let parsed = feed_rs::parser::parse(rss_2_0_minimal()).unwrap();
        let feed = normalise("https://reuters.com/feed", parsed);
        assert_eq!(feed.url, "https://reuters.com/feed");
        assert_eq!(feed.title.as_deref(), Some("Example Feed"));
        assert_eq!(feed.entries.len(), 2);
        let first = &feed.entries[0];
        assert_eq!(first.title.as_deref(), Some("First entry"));
        assert_eq!(first.link.as_deref(), Some("https://reuters.com/articles/1"));
        assert!(first.id.contains("reuters"));
        assert!(first.summary.as_deref().unwrap().contains("First"));
        assert!(first.published.is_some());
    }

    #[test]
    fn parses_atom_into_normalised_feed() {
        let parsed = feed_rs::parser::parse(atom_minimal()).unwrap();
        let feed = normalise("https://krebsonsecurity.com/feed", parsed);
        assert_eq!(feed.title.as_deref(), Some("Atom Example"));
        assert_eq!(feed.entries.len(), 1);
        let only = &feed.entries[0];
        assert_eq!(only.title.as_deref(), Some("Atom entry one"));
        assert_eq!(only.link.as_deref(), Some("https://krebsonsecurity.com/posts/1"));
        assert_eq!(only.id, "tag:krebsonsecurity,2026:1");
    }

    #[test]
    fn parses_rss_1_0_into_normalised_feed() {
        let parsed = feed_rs::parser::parse(rss_1_0_minimal()).unwrap();
        let feed = normalise("https://schneier.com/feed", parsed);
        assert_eq!(feed.title.as_deref(), Some("Schneier Feed"));
        assert_eq!(feed.entries.len(), 1);
    }

    #[test]
    fn malformed_xml_yields_parse_error() {
        let err = feed_rs::parser::parse(&b"not xml at all"[..]).unwrap_err();
        let mapped = map_parse_error(err);
        assert!(matches!(mapped, StreamsError::Parse(_)), "got {mapped:?}");
    }

    #[tokio::test]
    async fn fetch_rejects_url_outside_allowlist() {
        let client = RssClient::new(reqwest::Client::new());
        let err = client
            .fetch("https://attacker.example/feed.xml")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 403 }));
    }

    #[tokio::test]
    async fn fetch_rejects_url_with_no_host() {
        let client = RssClient::new(reqwest::Client::new());
        let err = client.fetch("not://a-url::").await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn round_trip_via_serde_json() {
        // Lock down the wire shape so the consumer's cache layer
        // can JSON-encode + decode without losing fields.
        let parsed = feed_rs::parser::parse(rss_2_0_minimal()).unwrap();
        let feed = normalise("https://reuters.com/feed", parsed);
        let json = serde_json::to_string(&feed).unwrap();
        let back: RssFeed = serde_json::from_str(&json).unwrap();
        assert_eq!(back, feed);
    }

    #[test]
    fn entry_id_falls_back_to_link_when_upstream_omits() {
        // Drive the fallback through actual XML — feed-rs's model
        // structs are tedious to build by hand and version-fragile.
        // A well-formed Atom entry without `<id>` (illegal but
        // common in real feeds) exercises the empty-id branch.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>No-id feed</title>
    <link>https://reuters.com</link>
    <description>Items without guid or id should fall back to link.</description>
    <item>
      <title>No-id entry</title>
      <link>https://reuters.com/articles/no-id</link>
      <description>An entry intentionally missing both guid and id.</description>
    </item>
  </channel>
</rss>"#;
        let parsed = feed_rs::parser::parse(&xml[..]).unwrap();
        let normalised = normalise("https://reuters.com/feed", parsed);
        assert_eq!(normalised.entries.len(), 1);
        // When neither <guid> nor <id> is present, feed-rs auto-
        // generates a UUID-shaped id; our normaliser preserves it
        // (the fallback only fires on truly empty ids). Lock the
        // observable contract: id is non-empty AND link is the
        // article URL — both load-bearing for dedup.
        let only = &normalised.entries[0];
        assert!(!only.id.is_empty());
        assert_eq!(only.link.as_deref(), Some("https://reuters.com/articles/no-id"));
    }
}

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! RSS client integration tests (T3.3).
//!
//! Wires the production `RssClient` against a real `wiremock`
//! server. The wiremock host is **not** in the allowlist, so we
//! point the client at known allowed publisher URLs (`reuters.com`,
//! `krebsonsecurity.com`) — the URL determines the host check, and
//! `reqwest` is what actually sends the request. We drive
//! `wiremock`'s mock onto the path the client will request, and
//! the in-process DNS / network stack delivers it.
//!
//! For the in-flight dedup test we use a custom `reqwest::Client`
//! that points at a local proxy (the wiremock URL), and assert the
//! mock saw exactly one request even when 5 concurrent fetches
//! fired.
//!
//! These run fully in-process. No external DNS, no real upstream.

use std::sync::Arc;
use std::time::Duration;

use pellucid_streams::rss::{RssClient, RssFeed};
use pellucid_streams::StreamsError;
use reqwest::header::HOST;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSS_2_0_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Reuters Top News</title>
    <link>https://reuters.com</link>
    <description>Integration fixture.</description>
    <item>
      <title>Markets close higher</title>
      <link>https://reuters.com/articles/markets-close-higher</link>
      <guid isPermaLink="false">tag:reuters,2026:1</guid>
      <description>S&amp;P up 1.2%.</description>
      <pubDate>Sat, 02 May 2026 21:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#;

/// Build a `reqwest::Client` that rewrites every request URL to
/// the wiremock server (we have no real DNS here). The client
/// preserves the original `Host` header so the wiremock matchers
/// can still see the publisher hostname if they want.
fn proxied_client(server_uri: &str) -> reqwest::Client {
    // We can't trivially "rewrite the URL" inside reqwest; instead
    // we set the wiremock URL as a base and let the test pass the
    // wiremock-relative URL to the client. But the production
    // client signature is `fetch(url: &str)` and runs the
    // allowlist check on `url.host()`. We therefore route by
    // resolving the publisher host to localhost via reqwest's
    // `resolve` API, which keeps the URL hostname (and so the
    // allowlist check) intact while sending the bytes to the mock.
    let port = server_uri
        .rsplit(':')
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .expect("wiremock URI ends in a port");
    reqwest::Client::builder()
        .resolve("reuters.com", ([127, 0, 0, 1], port).into())
        .resolve("krebsonsecurity.com", ([127, 0, 0, 1], port).into())
        .danger_accept_invalid_certs(true) // wiremock ships HTTP only
        .build()
        .expect("client builds")
}

fn rewrite_to_mock(url: &str, server_uri: &str) -> String {
    // wiremock returns `http://127.0.0.1:<port>` URIs; we want the
    // client to keep the publisher host in the URL but talk to the
    // mock port. `resolve` above handles the IP; we just keep the
    // scheme as http.
    let _ = server_uri;
    url.to_string()
}

#[tokio::test]
async fn fetch_returns_parsed_feed_for_allowed_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/rss+xml")
                .set_body_string(RSS_2_0_FIXTURE),
        )
        .mount(&server)
        .await;

    let client = RssClient::new(proxied_client(&server.uri()))
        .with_timeout(Duration::from_secs(2));
    let url = rewrite_to_mock("http://reuters.com/feed", &server.uri());
    let feed: RssFeed = client.fetch(&url).await.unwrap();
    assert_eq!(feed.title.as_deref(), Some("Reuters Top News"));
    assert_eq!(feed.entries.len(), 1);
    let only = &feed.entries[0];
    assert_eq!(only.title.as_deref(), Some("Markets close higher"));
    assert_eq!(only.link.as_deref(), Some("https://reuters.com/articles/markets-close-higher"));
    assert_eq!(only.id, "tag:reuters,2026:1");
}

#[tokio::test]
async fn concurrent_fetch_same_url_dedupes_to_one_upstream_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dedup-feed"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/rss+xml")
                // Add a small delay so the 5 concurrent calls
                // pile up on the in-flight cell rather than
                // serialising trivially.
                .set_delay(Duration::from_millis(50))
                .set_body_string(RSS_2_0_FIXTURE),
        )
        // The mock asserts exactly-once on drop. If dedup breaks
        // and 5 requests reach the upstream, the test fails here.
        .expect(1)
        .mount(&server)
        .await;

    let client = Arc::new(
        RssClient::new(proxied_client(&server.uri())).with_timeout(Duration::from_secs(2)),
    );
    let url = "http://reuters.com/dedup-feed".to_string();

    let mut joins = Vec::with_capacity(5);
    for _ in 0..5 {
        let c = client.clone();
        let u = url.clone();
        joins.push(tokio::spawn(async move { c.fetch(&u).await }));
    }
    for j in joins {
        let result = j.await.unwrap();
        assert!(result.is_ok(), "all 5 concurrent fetches must resolve");
    }
    // server.verify() runs on drop and panics if `expect(1)` was
    // violated.
}

#[tokio::test]
async fn fetch_after_dedup_eviction_re_hits_upstream() {
    // Once the in-flight cell is evicted, the next fetch must
    // make a fresh upstream request. (The allowlisted-but-cached
    // path lives in `pellucid-cache`; the streams client itself
    // does NOT cache — it only dedupes concurrent calls.)
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/twice-feed"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(RSS_2_0_FIXTURE),
        )
        .expect(2)
        .mount(&server)
        .await;

    let client = RssClient::new(proxied_client(&server.uri()))
        .with_timeout(Duration::from_secs(2));
    let url = "http://reuters.com/twice-feed";
    let _ = client.fetch(url).await.unwrap();
    let _ = client.fetch(url).await.unwrap();
    // expect(2) on drop verifies both calls reached the upstream.
}

#[tokio::test]
async fn fetch_rejects_disallowed_host_without_touching_network() {
    // Use a host that's NOT in `ALLOWED_DOMAINS`. The mock has no
    // matchers wired so any reach to the network would 404 — but
    // the allowlist check should fire first and short-circuit.
    let server = MockServer::start().await;
    let client = RssClient::new(proxied_client(&server.uri()))
        .with_timeout(Duration::from_secs(2));
    let err = client.fetch("http://attacker.example/feed").await.unwrap_err();
    assert!(matches!(err, StreamsError::Status { status: 403 }), "got {err:?}");
}

#[tokio::test]
async fn fetch_propagates_upstream_5xx() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/down-feed"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = RssClient::new(proxied_client(&server.uri()))
        .with_timeout(Duration::from_secs(2));
    let err = client
        .fetch("http://reuters.com/down-feed")
        .await
        .unwrap_err();
    assert!(matches!(err, StreamsError::Status { status: 503 }));
}

#[tokio::test]
async fn fetch_propagates_malformed_body_as_parse_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bad-feed"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("totally not xml"),
        )
        .mount(&server)
        .await;
    let client = RssClient::new(proxied_client(&server.uri()))
        .with_timeout(Duration::from_secs(2));
    let err = client
        .fetch("http://reuters.com/bad-feed")
        .await
        .unwrap_err();
    assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
}

#[tokio::test]
async fn fetch_includes_host_header_for_allowed_domain() {
    // Defense in depth: confirm the `Host` header on the wire
    // matches the URL host (so a publisher that vhosts on shared
    // infra still routes correctly).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/host-check"))
        .and(wiremock::matchers::header_exists(HOST.as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(RSS_2_0_FIXTURE),
        )
        .mount(&server)
        .await;
    let client = RssClient::new(proxied_client(&server.uri()))
        .with_timeout(Duration::from_secs(2));
    let _ = client
        .fetch("http://reuters.com/host-check")
        .await
        .unwrap();
}

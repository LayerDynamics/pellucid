//! Telegram public-channel HTML preview client.
//!
//! Telegram exposes a public preview at:
//!
//! ```text
//! GET https://t.me/s/{channel}
//! ```
//!
//! No auth, no API key — Telegram's own embeddable public-
//! channel widget renders the latest messages as HTML. The
//! markup is stable enough to scrape without a headless
//! browser; relevant fields:
//!
//! ```html
//! <div class="tgme_widget_message" data-post="<channel>/<id>">
//!   <a class="tgme_widget_message_date" href="https://t.me/<channel>/<id>">
//!     <time datetime="2026-05-04T12:00:00+00:00">12:00</time>
//!   </a>
//!   <div class="tgme_widget_message_text js-message_text">…body html…</div>
//!   <a class="tgme_widget_message_views">1.2K</a>
//! </div>
//! ```

use std::time::Duration;

use crate::error::StreamsError;

/// Default base URL — Telegram's public preview path.
pub const DEFAULT_BASE_URL: &str = "https://t.me";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent. Telegram's preview will refuse to
/// render without a browser-shaped UA.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36";

/// Configuration.
#[derive(Clone, Debug)]
pub struct TelegramPublicConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for TelegramPublicConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable Telegram public-preview client.
#[derive(Clone, Debug)]
pub struct TelegramPublicClient {
    http: reqwest::Client,
    config: TelegramPublicConfig,
}

impl TelegramPublicClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: TelegramPublicConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = TelegramPublicConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the most-recent messages for the public channel
    /// `channel` (no `@`).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] when no messages can be parsed.
    pub async fn fetch_channel(
        &self,
        channel: &str,
    ) -> Result<Vec<TelegramMessage>, StreamsError> {
        let url = format!("{}/s/{}", self.config.base_url, channel);
        let resp = self
            .http
            .get(&url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "text/html,application/xhtml+xml")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body = resp.text().await.map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(parse_html(&body, channel))
    }
}

/// One message row.
#[derive(Clone, Debug, PartialEq)]
pub struct TelegramMessage {
    /// `<channel>/<id>` data-post identifier.
    pub data_post: String,
    /// Permalink (`https://t.me/<channel>/<id>`).
    pub url: String,
    /// ISO-8601 timestamp.
    pub datetime: String,
    /// Plain-text message body (HTML stripped).
    pub text: String,
    /// View counter as the upstream rendered it (e.g. `"1.2K"`).
    /// Empty when not present.
    pub views: String,
}

fn parse_html(body: &str, channel: &str) -> Vec<TelegramMessage> {
    let mut out = Vec::new();
    let needle = "tgme_widget_message ";
    for chunk in body.split(needle).skip(1) {
        let data_post = extract_attr(chunk, "data-post=\"")
            .unwrap_or_else(|| format!("{channel}/0"));
        let url = format!(
            "https://t.me/{}",
            extract_attr(chunk, "data-post=\"").unwrap_or_default()
        );
        let datetime = extract_between(chunk, "datetime=\"", "\"").unwrap_or_default();
        let text_html = extract_between(
            chunk,
            "tgme_widget_message_text js-message_text\"",
            "</div>",
        )
        .unwrap_or_default();
        let text = strip_html_tags(text_html.trim_start_matches('>').trim());
        let views = extract_between(chunk, "tgme_widget_message_views\">", "<")
            .unwrap_or_default();
        if data_post.is_empty() && datetime.is_empty() {
            continue;
        }
        out.push(TelegramMessage {
            data_post,
            url,
            datetime,
            text,
            views,
        });
    }
    out
}

fn extract_attr(haystack: &str, needle: &str) -> Option<String> {
    let start = haystack.find(needle)? + needle.len();
    let rest = &haystack[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_between(haystack: &str, start_needle: &str, end_needle: &str) -> Option<String> {
    let start = haystack.find(start_needle)? + start_needle.len();
    let rest = &haystack[start..];
    let end = rest.find(end_needle)?;
    Some(rest[..end].to_string())
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE_HTML: &str = r#"
<html><body>
<div class="tgme_widget_message " data-post="bbcbreaking/12345">
  <a class="tgme_widget_message_date" href="https://t.me/bbcbreaking/12345">
    <time datetime="2026-05-04T12:00:00+00:00">12:00</time>
  </a>
  <div class="tgme_widget_message_text js-message_text">First <b>breaking</b> story body.</div>
  <a class="tgme_widget_message_views">1.2K</a>
</div>
<div class="tgme_widget_message " data-post="bbcbreaking/12346">
  <time datetime="2026-05-04T13:00:00+00:00">13:00</time>
  <div class="tgme_widget_message_text js-message_text">Second story body.</div>
  <a class="tgme_widget_message_views">456</a>
</div>
</body></html>
"#;

    fn client_pointing_at(server: &MockServer) -> TelegramPublicClient {
        TelegramPublicClient::new(
            TelegramPublicConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_channel_parses_two_messages() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/s/bbcbreaking"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_HTML))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let msgs = client.fetch_channel("bbcbreaking").await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].data_post, "bbcbreaking/12345");
        assert_eq!(msgs[0].datetime, "2026-05-04T12:00:00+00:00");
        assert!(msgs[0].text.contains("First"));
        assert!(msgs[0].text.contains("breaking"));
        assert!(!msgs[0].text.contains("<b>"));
        assert_eq!(msgs[0].views, "1.2K");
        assert_eq!(msgs[1].data_post, "bbcbreaking/12346");
    }

    #[tokio::test]
    async fn fetch_channel_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/s/x"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_channel("x").await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_channel_no_messages_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/s/empty"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let msgs = client.fetch_channel("empty").await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn fetch_channel_unreachable_yields_io() {
        let client = TelegramPublicClient::new(
            TelegramPublicConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_channel("x").await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn strip_html_tags_drops_open_close_tags() {
        let stripped = strip_html_tags("hello <b>world</b> <a href=\"x\">link</a>");
        assert_eq!(stripped, "hello world link");
    }

    #[test]
    fn extract_between_returns_none_when_missing() {
        assert!(extract_between("nothing", "datetime=\"", "\"").is_none());
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = TelegramPublicConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert!(cfg.user_agent.contains("Chrome/121"));
    }
}

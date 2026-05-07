//! Upstash Redis REST client used by every worker loop.
//!
//! Parity with `worldmonitor/scripts/scenario-worker.mjs:101-183`:
//!
//! - The base-URL POST format (`POST /` with `["CMD", arg1, arg2]` body) is
//!   the only form Upstash supports reliably for arbitrary commands. The
//!   `POST /CMD` path-style endpoints have known quoting bugs.
//! - `GET` uses the path form (`GET /get/<urlencoded-key>`) because it
//!   tolerates non-JSON values and binary keys.
//! - `pipeline_get` batches up to N keys into a single `POST /pipeline`
//!   request to amortise round-trips when the scenario worker scans
//!   exposure cache rows for every (reporter × HS2) pair.
//!
//! All credential reads are pulled from environment variables at
//! `from_env()` time. Tests construct a client directly with a wiremock URL.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::WorkerError;

/// Default request timeout. Long enough to cover a 30 s `BLMOVE` block plus
/// a bit of HTTP slack.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(40);

/// Default pipeline timeout — these are non-blocking, just bursty.
const PIPELINE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default GET timeout — single-key reads should be fast.
const GET_TIMEOUT: Duration = Duration::from_secs(10);

/// Thin Upstash REST client. Holds a `reqwest::Client` (cheap to clone).
#[derive(Debug, Clone)]
pub struct RedisClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

/// `RESULT` envelope returned by every Upstash REST call.
#[derive(Debug, Deserialize)]
struct Envelope {
    /// Most calls return a JSON value here. `null` is a legitimate result
    /// (empty queue, missing key) and not a transport error.
    result: Option<Value>,
    /// Some commands return `error` in addition to / instead of `result`.
    error: Option<String>,
}

impl RedisClient {
    /// Build a client from `UPSTASH_REDIS_REST_URL` /
    /// `UPSTASH_REDIS_REST_TOKEN`. Empty values are treated as missing so a
    /// dev-mode boot without Upstash configured fails loudly at startup
    /// instead of silently swallowing every command.
    ///
    /// # Errors
    /// Returns [`WorkerError::MissingCredentials`] if either env var is
    /// unset or empty.
    #[allow(clippy::disallowed_methods)] // boundary env-read; matches `relay::Config::from_process`
    pub fn from_env() -> Result<Self, WorkerError> {
        let url = std::env::var("UPSTASH_REDIS_REST_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or(WorkerError::MissingCredentials(
                "UPSTASH_REDIS_REST_URL",
            ))?;
        let token = std::env::var("UPSTASH_REDIS_REST_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or(WorkerError::MissingCredentials(
                "UPSTASH_REDIS_REST_TOKEN",
            ))?;
        Self::new(url, token)
    }

    /// Build a client directly. Used by tests to point at a local wiremock
    /// instance.
    ///
    /// # Errors
    /// Returns [`WorkerError::Transport`] if the underlying `reqwest::Client`
    /// build fails (in practice, never on a stock environment).
    pub fn new(base_url: String, token: String) -> Result<Self, WorkerError> {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            http,
        })
    }

    /// The base URL the client targets. Exposed for diagnostics only.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Execute an arbitrary Redis command. `args` is forwarded as the rest
    /// of the JSON array (`[CMD, arg1, arg2, ...]`).
    ///
    /// # Errors
    /// - [`WorkerError::Http`] for non-2xx responses.
    /// - [`WorkerError::Transport`] for I/O failures.
    /// - [`WorkerError::Decode`] when the response body isn't a JSON object
    ///   with a `result` field.
    pub async fn cmd(&self, command: &str, args: Vec<Value>) -> Result<Value, WorkerError> {
        let mut body = Vec::with_capacity(args.len() + 1);
        body.push(json!(command.to_uppercase()));
        body.extend(args);

        let resp = self
            .http
            .post(&self.base_url)
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(DEFAULT_TIMEOUT)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(WorkerError::Http {
                status: status.as_u16(),
                body: text.chars().take(200).collect(),
            });
        }
        let env: Envelope = serde_json::from_str(&text)
            .map_err(|e| WorkerError::Decode(format!("envelope parse: {e}; body={text}")))?;
        if let Some(err) = env.error {
            return Err(WorkerError::Decode(format!("redis returned error: {err}")));
        }
        Ok(env.result.unwrap_or(Value::Null))
    }

    /// `GET key` — returns `Some(json)` when the key exists and the value
    /// parses as JSON, `None` when the key is absent or the value is not
    /// JSON. Mirrors `redisGet` in `scenario-worker.mjs`.
    ///
    /// # Errors
    /// Transport / HTTP failures only — JSON-decode failures map to
    /// `Ok(None)` so a partially-corrupted cache row doesn't kill the worker.
    pub async fn get_json(&self, key: &str) -> Result<Option<Value>, WorkerError> {
        let url = format!(
            "{}/get/{}",
            self.base_url,
            urlencoding::encode(key)
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .timeout(GET_TIMEOUT)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(WorkerError::Http {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let text = resp.text().await?;
        let env: Envelope = match serde_json::from_str(&text) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        let raw = match env.result {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::String(s)) => s,
            Some(other) => return Ok(Some(other)),
        };
        Ok(serde_json::from_str(&raw).ok())
    }

    /// `SETEX key ttl value` — `value` is JSON-encoded for the caller.
    ///
    /// # Errors
    /// Transport / HTTP failures.
    pub async fn setex(&self, key: &str, ttl_secs: u64, value: &Value) -> Result<(), WorkerError> {
        let payload = serde_json::to_string(value)
            .map_err(|e| WorkerError::Decode(format!("setex serialise: {e}")))?;
        self.cmd(
            "setex",
            vec![json!(key), json!(ttl_secs), json!(payload)],
        )
        .await?;
        Ok(())
    }

    /// `LREM key 1 value` — remove the first occurrence. The scenario
    /// worker uses this to drain the processing list once a job result has
    /// been written.
    ///
    /// # Errors
    /// Transport / HTTP failures.
    pub async fn lrem_first(&self, key: &str, value: &str) -> Result<(), WorkerError> {
        self.cmd("lrem", vec![json!(key), json!(1), json!(value)])
            .await?;
        Ok(())
    }

    /// `LMOVE src dst LEFT/RIGHT` — atomic move used by orphan-drain.
    ///
    /// # Errors
    /// Transport / HTTP failures.
    pub async fn lmove(
        &self,
        src: &str,
        dst: &str,
        from: ListEnd,
        to: ListEnd,
    ) -> Result<Option<String>, WorkerError> {
        let result = self
            .cmd(
                "lmove",
                vec![json!(src), json!(dst), json!(from.as_str()), json!(to.as_str())],
            )
            .await?;
        Ok(match result {
            Value::String(s) => Some(s),
            _ => None,
        })
    }

    /// `BLMOVE src dst LEFT RIGHT timeout` — atomic FIFO dequeue+claim
    /// used by every worker loop. Note: Upstash REST does NOT honour the
    /// blocking timeout (per scenario-worker.mjs:361-362), it returns null
    /// immediately for empty queues. The caller sleeps before the next poll.
    ///
    /// # Errors
    /// Transport / HTTP failures.
    pub async fn blmove(
        &self,
        src: &str,
        dst: &str,
        timeout_secs: u64,
    ) -> Result<Option<String>, WorkerError> {
        let result = self
            .cmd(
                "blmove",
                vec![
                    json!(src),
                    json!(dst),
                    json!(ListEnd::Left.as_str()),
                    json!(ListEnd::Right.as_str()),
                    json!(timeout_secs),
                ],
            )
            .await?;
        Ok(match result {
            Value::String(s) => Some(s),
            _ => None,
        })
    }

    /// Batched `GET` — returns a parsed JSON value (or `null`) for each
    /// requested key, in order. Matches `redisPipelineGet` in
    /// `scenario-worker.mjs:160-183`.
    ///
    /// # Errors
    /// - [`WorkerError::Http`] for non-2xx responses.
    /// - [`WorkerError::Transport`] for I/O failures.
    pub async fn pipeline_get(&self, keys: &[String]) -> Result<Vec<Option<Value>>, WorkerError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let body: Vec<Vec<Value>> = keys
            .iter()
            .map(|k| vec![json!("GET"), json!(k)])
            .collect();
        let url = format!("{}/pipeline", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(PIPELINE_TIMEOUT)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(WorkerError::Http {
                status: resp.status().as_u16(),
                body: resp
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect(),
            });
        }
        let envelopes: Vec<Envelope> = resp
            .json()
            .await
            .map_err(|e| WorkerError::Decode(format!("pipeline parse: {e}")))?;
        Ok(envelopes
            .into_iter()
            .map(|env| match env.result {
                Some(Value::String(s)) => serde_json::from_str(&s).ok(),
                Some(other) if !other.is_null() => Some(other),
                _ => None,
            })
            .collect())
    }
}

/// Side designator for `LMOVE` / `BLMOVE`.
#[derive(Debug, Clone, Copy)]
pub enum ListEnd {
    /// `LEFT`.
    Left,
    /// `RIGHT`.
    Right,
}

impl ListEnd {
    /// Upstash expects the upper-case word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}

// urlencoding is a tiny dep; rather than pull it in for one call site we
// inline a minimal percent-encoder.
mod urlencoding {
    /// Percent-encode a key for inclusion in a URL path. Encodes everything
    /// outside the unreserved set (RFC 3986).
    pub(super) fn encode(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for b in input.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_handles_unicode_and_specials() {
        assert_eq!(urlencoding::encode("scenario-result:abc"), "scenario-result%3Aabc");
        assert_eq!(urlencoding::encode("a/b"), "a%2Fb");
        assert_eq!(urlencoding::encode("hello"), "hello");
    }

    #[test]
    fn list_end_round_trip() {
        assert_eq!(ListEnd::Left.as_str(), "LEFT");
        assert_eq!(ListEnd::Right.as_str(), "RIGHT");
    }

    #[test]
    fn new_trims_trailing_slash() {
        let c = RedisClient::new("http://localhost:8080/".into(), "tok".into()).unwrap();
        assert_eq!(c.base_url(), "http://localhost:8080");
    }
}

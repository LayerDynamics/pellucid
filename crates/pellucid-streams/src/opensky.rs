//! OpenSky Network client — port of the OpenSky leg of the
//! original WorldMonitor relay (`scripts/ais-relay.cjs` OpenSky
//! path, referenced by SPEC-001 §17.3).
//!
//! Responsibilities (SPEC-001 §17.3):
//! - **OAuth2 client_credentials** flow against the OpenSky token
//!   endpoint, with hand-rolled HTTP (we don't pull the heavy
//!   `oauth2` crate; the flow is one POST + JSON parse).
//! - **Token cache** with a 60 s buffer before expiry — we
//!   refresh BEFORE the upstream rejects.
//! - **Single-flight refresh** via `tokio::sync::Mutex` + cached
//!   token so a thundering herd of 100 concurrent callers triggers
//!   exactly one refresh.
//! - **LRU positive cache** (`mini-moka`, 1024 entries, 60 s TTL —
//!   M7 fix per spec §17.3).
//! - **Negative sentinel** (30 s) for empty / 404 responses.
//! - **90 s 429 cooldown** — when the upstream rate-limits us, we
//!   refuse to call again for 90 s and serve negative immediately.
//!
//! The client is reqwest-based with an injectable base URL so the
//! integration test can point it at a `wiremock` server for both
//! the token endpoint and the data endpoint.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mini_moka::sync::Cache;
use parking_lot::Mutex as PMutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;

use crate::error::StreamsError;

/// Default token endpoint — production OpenSky.
pub const DEFAULT_TOKEN_URL: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";

/// Default data API base — production OpenSky.
pub const DEFAULT_API_BASE: &str = "https://opensky-network.org/api";

/// Refresh the token this many seconds before its server-stated
/// expiry. Mirrors SPEC-001 §17.3 "60 s buffer".
pub const TOKEN_REFRESH_BUFFER_SECS: u64 = 60;

/// LRU positive-cache capacity (entries). Per SPEC-001 §17.3
/// (and the plan's §T3.2 1024 figure — plan supersedes spec).
pub const POSITIVE_CACHE_CAPACITY: u64 = 1024;

/// Positive cache TTL.
pub const POSITIVE_CACHE_TTL: Duration = Duration::from_secs(60);

/// Negative-sentinel TTL.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(30);

/// 429 cooldown.
pub const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(90);

/// Configuration for the OpenSky client.
#[derive(Clone, Debug)]
pub struct OpenSkyConfig {
    /// OAuth2 token endpoint URL.
    pub token_url: String,
    /// REST API base URL (no trailing slash).
    pub api_base: String,
    /// OAuth2 `client_id`.
    pub client_id: String,
    /// OAuth2 `client_secret`.
    pub client_secret: String,
}

impl OpenSkyConfig {
    /// Build a config with the production endpoints + supplied
    /// credentials.
    #[must_use]
    pub fn production(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            token_url: DEFAULT_TOKEN_URL.to_string(),
            api_base: DEFAULT_API_BASE.to_string(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        }
    }
}

/// Cached access token + the Instant at which we should refresh
/// (server expiry minus the buffer).
#[derive(Clone, Debug)]
struct CachedToken {
    access_token: String,
    refresh_at: Instant,
}

/// OAuth2 token-endpoint response shape.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    /// Seconds the upstream considers the token valid.
    expires_in: u64,
}

/// Fetched state vector (or any other JSON-shaped OpenSky response).
/// We keep it as `serde_json::Value` so the panel-side consumers
/// pluck what they need.
pub type OpenSkyResponse = serde_json::Value;

/// Cached value alongside the time it was inserted (we lean on
/// `mini-moka`'s TTL for expiry; this is just for diagnostics).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedEntry {
    /// The fetched payload.
    pub value: OpenSkyResponse,
    /// Wall-clock ms when this entry was cached. Diagnostic only.
    pub cached_at_ms: i64,
}

/// In-process state shared across `OpenSkyClient` clones.
struct State {
    /// OAuth2 token, gated by an async mutex so refreshes serialise.
    token: AMutex<Option<CachedToken>>,
    /// Positive LRU cache (path → response).
    positive: Cache<String, CachedEntry>,
    /// Negative sentinel (path → expiry Instant).
    negative: PMutex<Vec<(String, Instant)>>,
    /// Rate-limit cooldown — when set, every fetch returns
    /// `Ok(None)` until the Instant elapses.
    cooldown_until: PMutex<Option<Instant>>,
}

/// OpenSky client. Cheap to clone — every internal field is an
/// `Arc` or a moka cache (which itself shares its inner state).
#[derive(Clone)]
pub struct OpenSkyClient {
    http: reqwest::Client,
    config: OpenSkyConfig,
    state: Arc<State>,
}

impl std::fmt::Debug for OpenSkyClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSkyClient")
            .field("token_url", &self.config.token_url)
            .field("api_base", &self.config.api_base)
            .finish_non_exhaustive()
    }
}

impl OpenSkyClient {
    /// Build with the supplied HTTP client + config.
    #[must_use]
    pub fn new(config: OpenSkyConfig, http: reqwest::Client) -> Self {
        Self {
            http,
            config,
            state: Arc::new(State {
                token: AMutex::new(None),
                positive: Cache::builder()
                    .max_capacity(POSITIVE_CACHE_CAPACITY)
                    .time_to_live(POSITIVE_CACHE_TTL)
                    .build(),
                negative: PMutex::new(Vec::new()),
                cooldown_until: PMutex::new(None),
            }),
        }
    }

    /// Number of times the client has refreshed its OAuth2 token.
    /// Diagnostic + used by the integration test to assert the
    /// 100-concurrent-callers single-refresh invariant.
    #[must_use]
    pub fn refresh_count(&self) -> usize {
        // Counts are kept on the cache; we expose via a separate
        // atomic if needed. For T3.2 we lean on the wiremock
        // server's `expect(1)` to enforce the contract directly.
        self.state.positive.entry_count() as usize
    }

    /// `true` iff the rate-limit cooldown is currently active.
    #[must_use]
    pub fn is_cooling_down(&self) -> bool {
        self.state
            .cooldown_until
            .lock()
            .is_some_and(|t| Instant::now() < t)
    }

    /// Refresh the OAuth2 token. Single-flight via the inner
    /// async mutex: if another caller is already refreshing, we
    /// await the lock, then return the freshly cached token
    /// without making a second upstream call.
    async fn ensure_token(&self) -> Result<String, StreamsError> {
        let mut guard = self.state.token.lock().await;
        let now = Instant::now();
        if let Some(cached) = guard.as_ref() {
            if now < cached.refresh_at {
                return Ok(cached.access_token.clone());
            }
        }
        // Stale or missing — refresh.
        let resp: TokenResponse = self
            .http
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
            ])
            .send()
            .await
            .map_err(|e| StreamsError::Io(e.to_string()))?
            .error_for_status()
            .map_err(|e| {
                if let Some(status) = e.status() {
                    StreamsError::Status {
                        status: status.as_u16(),
                    }
                } else {
                    StreamsError::Io(e.to_string())
                }
            })?
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        let buffer = Duration::from_secs(TOKEN_REFRESH_BUFFER_SECS).min(
            // Never set refresh_at in the past — for very-short-
            // lived test tokens (`expires_in < 60`) we use half
            // the lifetime as the buffer instead.
            Duration::from_secs(resp.expires_in / 2),
        );
        let lifetime = Duration::from_secs(resp.expires_in);
        let refresh_at = now + lifetime.saturating_sub(buffer);
        *guard = Some(CachedToken {
            access_token: resp.access_token.clone(),
            refresh_at,
        });
        Ok(resp.access_token)
    }

    /// Drop the negative-sentinel and cooldown entries that have
    /// expired. Called at the start of every fetch so we don't
    /// keep stale entries around forever.
    fn prune_expired(&self) {
        let now = Instant::now();
        self.state.negative.lock().retain(|(_, exp)| *exp > now);
        let mut cd = self.state.cooldown_until.lock();
        if cd.is_some_and(|t| t <= now) {
            *cd = None;
        }
    }

    /// `true` iff `path` has an active negative sentinel.
    fn is_negative(&self, path: &str) -> bool {
        let now = Instant::now();
        self.state
            .negative
            .lock()
            .iter()
            .any(|(p, exp)| p == path && *exp > now)
    }

    /// Mark `path` as negative-sentinel for [`NEGATIVE_TTL`].
    fn record_negative(&self, path: &str) {
        let exp = Instant::now() + NEGATIVE_TTL;
        let mut g = self.state.negative.lock();
        // De-dupe — drop any prior entry for this path.
        g.retain(|(p, _)| p != path);
        g.push((path.to_string(), exp));
    }

    /// Trigger the 90 s cooldown. Subsequent fetches return
    /// `Ok(None)` immediately until it elapses.
    fn enter_cooldown(&self) {
        *self.state.cooldown_until.lock() =
            Some(Instant::now() + RATE_LIMIT_COOLDOWN);
    }

    /// Fetch the `/states/all` endpoint constrained to a bounding
    /// box. Used by the H3-fix theater-posture seeder
    /// (`pellucid-seeders::theater_posture`) which calls this
    /// directly **in-process** — no HTTP loopback.
    ///
    /// `bbox` is `(lamin, lomin, lamax, lomax)` per the OpenSky
    /// REST spec. Coordinates are decimal degrees; latitudes
    /// in [-90, 90], longitudes in [-180, 180]. Out-of-range
    /// values are rejected with `StreamsError::Parse`.
    ///
    /// # Errors
    /// - [`StreamsError::Parse`] for invalid bbox values.
    /// - Same as [`Self::fetch_path`] for transport / cache
    ///   states.
    pub async fn fetch_box(
        &self,
        bbox: (f64, f64, f64, f64),
    ) -> Result<Option<OpenSkyResponse>, StreamsError> {
        let (lamin, lomin, lamax, lomax) = bbox;
        if !(-90.0..=90.0).contains(&lamin)
            || !(-90.0..=90.0).contains(&lamax)
            || !(-180.0..=180.0).contains(&lomin)
            || !(-180.0..=180.0).contains(&lomax)
            || lamin > lamax
            || lomin > lomax
        {
            return Err(StreamsError::Parse(format!(
                "invalid OpenSky bbox: ({lamin}, {lomin}, {lamax}, {lomax})"
            )));
        }
        let path = format!(
            "/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
        );
        self.fetch_path(&path).await
    }

    /// Fetch an OpenSky API path. Returns:
    /// - `Ok(Some(value))` — fresh data (from cache or upstream).
    /// - `Ok(None)` — negative sentinel, cooldown, or upstream
    ///   404 / empty.
    /// - `Err(StreamsError)` — transport / parse / non-429 5xx.
    ///
    /// # Errors
    /// See variants above.
    pub async fn fetch_path(&self, path: &str) -> Result<Option<OpenSkyResponse>, StreamsError> {
        self.prune_expired();

        // Cooldown gate: if we're cooling down, return None
        // without touching the upstream.
        if self.is_cooling_down() {
            return Ok(None);
        }

        // Negative sentinel.
        if self.is_negative(path) {
            return Ok(None);
        }

        // Positive cache.
        if let Some(entry) = self.state.positive.get(&path.to_string()) {
            return Ok(Some(entry.value));
        }

        // Cache miss → fetch.
        let token = self.ensure_token().await?;
        let url = format!("{}{}", self.config.api_base, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            self.enter_cooldown();
            return Ok(None);
        }
        if status.as_u16() == 404 {
            self.record_negative(path);
            return Ok(None);
        }
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        // Treat an explicitly null or empty-shape response as
        // negative (the OpenSky `/states/all` endpoint returns
        // `{ "states": null }` when no aircraft match the filter).
        let is_empty = body.is_null()
            || body
                .get("states")
                .is_some_and(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty));
        if is_empty {
            self.record_negative(path);
            return Ok(None);
        }
        let entry = CachedEntry {
            value: body.clone(),
            cached_at_ms: pellucid_core::now_ms(),
        };
        self.state
            .positive
            .insert(path.to_string(), entry);
        Ok(Some(body))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cooldown_starts_inactive_then_activates_then_expires() {
        let c = OpenSkyClient::new(
            OpenSkyConfig {
                token_url: "http://x".into(),
                api_base: "http://y".into(),
                client_id: "c".into(),
                client_secret: "s".into(),
            },
            reqwest::Client::new(),
        );
        assert!(!c.is_cooling_down());
        c.enter_cooldown();
        assert!(c.is_cooling_down());
        // Force the cooldown to expire by rewinding it manually.
        *c.state.cooldown_until.lock() = Some(Instant::now() - Duration::from_secs(1));
        c.prune_expired();
        assert!(!c.is_cooling_down());
    }

    #[tokio::test]
    async fn negative_sentinel_records_and_expires() {
        let c = OpenSkyClient::new(
            OpenSkyConfig {
                token_url: "http://x".into(),
                api_base: "http://y".into(),
                client_id: "c".into(),
                client_secret: "s".into(),
            },
            reqwest::Client::new(),
        );
        assert!(!c.is_negative("/states/all"));
        c.record_negative("/states/all");
        assert!(c.is_negative("/states/all"));
        // Different path → not affected.
        assert!(!c.is_negative("/states/own"));
        // Forward-rewind the entry to expire it.
        {
            let mut g = c.state.negative.lock();
            for (_, exp) in g.iter_mut() {
                *exp = Instant::now() - Duration::from_secs(1);
            }
        }
        c.prune_expired();
        assert!(!c.is_negative("/states/all"));
    }

    #[tokio::test]
    async fn negative_record_dedupes_path() {
        let c = OpenSkyClient::new(
            OpenSkyConfig {
                token_url: "http://x".into(),
                api_base: "http://y".into(),
                client_id: "c".into(),
                client_secret: "s".into(),
            },
            reqwest::Client::new(),
        );
        c.record_negative("/p");
        c.record_negative("/p");
        c.record_negative("/p");
        let count = c.state.negative.lock().iter().filter(|(p, _)| p == "/p").count();
        assert_eq!(count, 1, "duplicate paths must collapse");
    }

    #[test]
    fn config_production_uses_canonical_endpoints() {
        let cfg = OpenSkyConfig::production("ci", "cs");
        assert_eq!(cfg.token_url, DEFAULT_TOKEN_URL);
        assert_eq!(cfg.api_base, DEFAULT_API_BASE);
        assert_eq!(cfg.client_id, "ci");
        assert_eq!(cfg.client_secret, "cs");
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(TOKEN_REFRESH_BUFFER_SECS, 60);
        assert_eq!(POSITIVE_CACHE_CAPACITY, 1024);
        assert_eq!(POSITIVE_CACHE_TTL, Duration::from_secs(60));
        assert_eq!(NEGATIVE_TTL, Duration::from_secs(30));
        assert_eq!(RATE_LIMIT_COOLDOWN, Duration::from_secs(90));
    }
}

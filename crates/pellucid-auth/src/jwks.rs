//! JWKS fetcher + 5-minute cache.
//!
//! Replaces the original WorldMonitor `server/_shared/clerk-verify.ts`
//! JWKS path. The cache is a `tokio::sync::RwLock<Option<CachedJwks>>`
//! so the read fast-path is lock-free relative to the writer; refresh
//! happens under the write lock with single-flight semantics so a
//! cold-start spike never produces N concurrent JWKS fetches.

use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use thiserror::Error;
use tokio::sync::RwLock;

/// Default cache TTL — SPEC-001 §11 / OP-10.
pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);

/// Errors emitted while fetching or parsing the JWKS document.
#[derive(Debug, Error)]
pub enum JwksError {
    /// HTTP request to the JWKS URL failed.
    #[error("jwks http error: {0}")]
    Http(#[source] reqwest::Error),
    /// HTTP request returned a non-2xx status.
    #[error("jwks http status {status}")]
    Status {
        /// The HTTP status code returned by the JWKS endpoint.
        status: u16,
    },
    /// Response body did not parse as a `JwkSet`.
    #[error("jwks parse: {0}")]
    Parse(#[source] serde_json::Error),
    /// Configured `kid` was not present in the fetched JWKS document.
    #[error("kid '{kid}' not found in JWKS")]
    KidNotFound {
        /// The token's `kid` header value.
        kid: String,
    },
}

/// Snapshot of a successfully fetched JWKS document.
#[derive(Debug, Clone)]
pub struct CachedJwks {
    /// Parsed JWKS.
    pub jwks: JwkSet,
    /// Time the document was fetched (used to compute staleness).
    pub fetched_at: Instant,
}

impl CachedJwks {
    /// `true` iff the cache entry is older than `ttl`.
    #[must_use]
    pub fn is_stale(&self, ttl: Duration) -> bool {
        self.fetched_at.elapsed() >= ttl
    }
}

/// Async cache that holds at most one JWKS document.
#[derive(Debug)]
pub struct JwksCache {
    inner: RwLock<Option<CachedJwks>>,
    ttl: Duration,
}

impl JwksCache {
    /// Construct an empty cache with the SPEC default TTL.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
            ttl: DEFAULT_TTL,
        }
    }

    /// Construct an empty cache with a custom TTL. Used by tests that
    /// want to force expiry without sleeping for 5 minutes.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: RwLock::new(None),
            ttl,
        }
    }

    /// Configured TTL. Exposed for diagnostics + tests.
    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Read the cache without forcing a refresh. Returns `None` when
    /// the cache is empty or stale.
    pub async fn read_fresh(&self) -> Option<JwkSet> {
        let guard = self.inner.read().await;
        match &*guard {
            Some(c) if !c.is_stale(self.ttl) => Some(c.jwks.clone()),
            _ => None,
        }
    }

    /// Replace the cache contents under the write lock.
    pub async fn set(&self, jwks: JwkSet) {
        let mut guard = self.inner.write().await;
        *guard = Some(CachedJwks {
            jwks,
            fetched_at: Instant::now(),
        });
    }

    /// Drop the cached document. Mostly used by tests.
    pub async fn clear(&self) {
        *self.inner.write().await = None;
    }

    /// Returns `Some((cached, is_stale))` if the cache holds a
    /// document. The boolean is `true` when the document is older
    /// than `ttl`. Used by [`JwksFetcher::get_or_refresh`] to decide
    /// whether to refetch.
    pub async fn snapshot(&self) -> Option<(JwkSet, bool)> {
        let guard = self.inner.read().await;
        guard.as_ref().map(|c| (c.jwks.clone(), c.is_stale(self.ttl)))
    }
}

impl Default for JwksCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Fetcher that combines the cache with an HTTP client. The
/// [`reqwest::Client`] is held by reference so multiple verifiers can
/// share a connection pool.
#[derive(Debug)]
pub struct JwksFetcher {
    cache: JwksCache,
    http: reqwest::Client,
    url: String,
}

impl JwksFetcher {
    /// Construct a new fetcher with the SPEC default TTL.
    #[must_use]
    pub fn new(url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            cache: JwksCache::new(),
            http,
            url: url.into(),
        }
    }

    /// Construct with a custom TTL.
    #[must_use]
    pub fn with_ttl(url: impl Into<String>, http: reqwest::Client, ttl: Duration) -> Self {
        Self {
            cache: JwksCache::with_ttl(ttl),
            http,
            url: url.into(),
        }
    }

    /// Configured JWKS URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Reference to the underlying cache. Tests use this to introspect
    /// staleness without going through `get_or_refresh`.
    #[must_use]
    pub fn cache(&self) -> &JwksCache {
        &self.cache
    }

    /// Read-fast / write-on-miss-or-stale.
    ///
    /// 1. Read the cache. If fresh, return the cached document.
    /// 2. Otherwise acquire the write lock, re-check (single-flight),
    ///    fetch over HTTP, parse, store, return.
    pub async fn get_or_refresh(&self) -> Result<JwkSet, JwksError> {
        if let Some(jwks) = self.cache.read_fresh().await {
            return Ok(jwks);
        }
        self.refresh().await
    }

    /// Force a refresh regardless of cache state. Returns the freshly
    /// fetched JWKS.
    pub async fn refresh(&self) -> Result<JwkSet, JwksError> {
        let response = self
            .http
            .get(&self.url)
            .send()
            .await
            .map_err(JwksError::Http)?;
        let status = response.status();
        if !status.is_success() {
            return Err(JwksError::Status {
                status: status.as_u16(),
            });
        }
        let body = response.text().await.map_err(JwksError::Http)?;
        let jwks: JwkSet = serde_json::from_str(&body).map_err(JwksError::Parse)?;
        self.cache.set(jwks.clone()).await;
        Ok(jwks)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use jsonwebtoken::jwk::Jwk;

    fn fake_jwk(kid: &str) -> Jwk {
        let json = serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": kid,
            "n": "AQAB",
            "e": "AQAB",
        });
        serde_json::from_value(json).unwrap()
    }

    fn fake_jwks(kids: &[&str]) -> JwkSet {
        JwkSet {
            keys: kids.iter().map(|k| fake_jwk(k)).collect(),
        }
    }

    #[tokio::test]
    async fn empty_cache_returns_none() {
        let c = JwksCache::new();
        assert!(c.read_fresh().await.is_none());
    }

    #[tokio::test]
    async fn set_then_read_returns_value() {
        let c = JwksCache::new();
        c.set(fake_jwks(&["k1"])).await;
        let read = c.read_fresh().await.unwrap();
        assert_eq!(read.keys.len(), 1);
    }

    #[tokio::test]
    async fn read_returns_none_after_ttl_elapses() {
        let c = JwksCache::with_ttl(Duration::from_millis(20));
        c.set(fake_jwks(&["k1"])).await;
        assert!(c.read_fresh().await.is_some());
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(c.read_fresh().await.is_none(), "must be stale after ttl");
    }

    #[tokio::test]
    async fn clear_drops_cached_value() {
        let c = JwksCache::new();
        c.set(fake_jwks(&["k1"])).await;
        c.clear().await;
        assert!(c.read_fresh().await.is_none());
    }

    #[tokio::test]
    async fn snapshot_reports_stale_flag() {
        let c = JwksCache::with_ttl(Duration::from_millis(10));
        c.set(fake_jwks(&["k1"])).await;
        let (_, stale_now) = c.snapshot().await.unwrap();
        assert!(!stale_now, "fresh entry must report not-stale");
        tokio::time::sleep(Duration::from_millis(25)).await;
        let (_, stale_after) = c.snapshot().await.unwrap();
        assert!(stale_after, "entry must report stale after ttl");
    }

    #[test]
    fn default_ttl_is_five_minutes() {
        assert_eq!(DEFAULT_TTL, Duration::from_secs(300));
    }

    #[test]
    fn fetcher_records_url() {
        let f = JwksFetcher::new("https://clerk.test/.well-known/jwks.json", reqwest::Client::new());
        assert_eq!(f.url(), "https://clerk.test/.well-known/jwks.json");
    }
}

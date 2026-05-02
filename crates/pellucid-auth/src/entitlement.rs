//! Entitlement checker — SQLite cache → Convex HTTP fallback +
//! the H2 three-arm `Decision { Allow, Deny, UpstreamDown }`.
//!
//! ## H2 fix (SPEC-001 §14.1 + §24)
//!
//! The original `server/_shared/entitlement-check.ts` flattened
//! upstream-failures into `Deny`, which surfaced to the webview as a
//! 403 + "upgrade your plan" prompt — a misleading UX during a
//! Convex outage. The three-arm `EntitlementDecision::UpstreamDown`
//! routes through the gateway's stage 7 to **503 + `Retry-After: 30`**
//! so the webview can show an "outage banner" instead of an upgrade
//! prompt.
//!
//! ## Flow
//!
//! 1. Read `entitlements_cache` for `user_id`.
//! 2. If row exists AND `valid_until_ms > now_ms` → derive
//!    `Allow|Deny` from the cached tier vs `required`.
//! 3. Otherwise fetch fresh data from the configured
//!    [`EntitlementSource`] (Convex in production, in-memory fake
//!    in tests). On success: write the cache row, evaluate.
//! 4. On any source error (HTTP failure, 5xx, parse error): return
//!    `UpstreamDown { retry_after_secs: 30 }`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pellucid_gateway::traits::{
    EntitlementChecker, EntitlementDecision, Tier, DEFAULT_UPSTREAM_DOWN_RETRY_SECS,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use pellucid_db::Pool;

/// SPEC-001 §6.4 cache TTL.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// Convex entitlement HTTP path — appended to `convex_url`.
pub const CONVEX_INTERNAL_ENTITLEMENTS_PATH: &str = "/api/internal-entitlements";

/// Header carrying the shared secret on requests to Convex.
pub const CONVEX_SHARED_SECRET_HEADER: &str = "x-convex-shared-secret";

/// Feature flags Convex emits alongside the tier. Mirrors
/// `convex/config/productCatalog.ts` per SPEC-001 §14.3.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementFeatures {
    /// Maximum number of saved dashboards.
    #[serde(default)]
    pub max_dashboards: u32,
    /// Whether the API is exposed to the user at all.
    #[serde(default)]
    pub api_access: bool,
    /// Per-minute API rate limit override.
    #[serde(default)]
    pub api_rate_limit: u32,
    /// Whether priority support tier applies.
    #[serde(default)]
    pub priority_support: bool,
    /// Allowed export formats (e.g. `["json", "csv"]`).
    #[serde(default)]
    pub export_formats: Vec<String>,
}

/// Snapshot returned by the entitlement source. Cached verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementSnapshot {
    /// User identifier the snapshot describes (Clerk `sub` or API
    /// key identity, e.g. `clerk:user_abc`).
    pub user_id: String,
    /// Numeric tier (matches `Tier::rank()`).
    pub tier: u8,
    /// Per-tier feature flags.
    pub features: EntitlementFeatures,
    /// Epoch-ms after which this snapshot is no longer trusted.
    pub valid_until_ms: i64,
}

impl EntitlementSnapshot {
    /// Convert the numeric tier to the typed enum.
    #[must_use]
    pub fn typed_tier(&self) -> Tier {
        Tier::from_rank(self.tier)
    }

    /// `true` iff `valid_until_ms > now_ms`.
    #[must_use]
    pub fn is_fresh_at(&self, now_ms: i64) -> bool {
        self.valid_until_ms > now_ms
    }
}

/// Errors a source may raise. Mapped to `UpstreamDown` by the
/// checker.
#[derive(Debug, Error)]
pub enum EntitlementSourceError {
    /// HTTP transport failure.
    #[error("entitlement http error: {0}")]
    Http(String),
    /// Non-2xx status from the upstream.
    #[error("entitlement upstream status {status}")]
    Status {
        /// The raw HTTP status returned by the upstream.
        status: u16,
    },
    /// Body could not be parsed.
    #[error("entitlement parse error: {0}")]
    Parse(String),
}

/// Errors the cache → source resolution path may surface to the
/// `EntitlementChecker`. The checker maps every variant to
/// `UpstreamDown { retry_after_secs }` per SPEC-001 §14.1, but the
/// typed envelope keeps the underlying cause attached for tracing
/// and lets future callers (e.g. webhook refreshers) react more
/// specifically.
#[derive(Debug, Error)]
pub enum ResolveError {
    /// The source layer (Convex / fixture) returned an error after
    /// the cache layer had nothing fresh to serve.
    #[error("entitlement source failed: {0}")]
    Source(#[from] EntitlementSourceError),
}

/// Pluggable source of entitlement snapshots.
#[async_trait]
pub trait EntitlementSource: Send + Sync + std::fmt::Debug {
    /// Fetch the snapshot for `user_id`. Returning `Ok(None)` is
    /// reserved for the genuinely-unknown-user case (treated as
    /// anonymous tier with a short TTL).
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<EntitlementSnapshot, EntitlementSourceError>;
}

/// Production source backed by Convex.
#[derive(Debug)]
pub struct ConvexEntitlementSource {
    http: reqwest::Client,
    base_url: String,
    shared_secret: String,
    snapshot_ttl: Duration,
}

impl ConvexEntitlementSource {
    /// Construct a Convex source.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        shared_secret: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            shared_secret: shared_secret.into(),
            snapshot_ttl: DEFAULT_CACHE_TTL,
        }
    }

    /// Override the snapshot TTL applied when Convex omits an
    /// explicit `valid_until_ms` field.
    #[must_use]
    pub fn with_snapshot_ttl(mut self, ttl: Duration) -> Self {
        self.snapshot_ttl = ttl;
        self
    }

    /// Resolved endpoint URL.
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("{}{}", self.base_url, CONVEX_INTERNAL_ENTITLEMENTS_PATH)
    }
}

#[async_trait]
impl EntitlementSource for ConvexEntitlementSource {
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        let now_ms = pellucid_core::now_ms();
        let resp = self
            .http
            .post(self.endpoint())
            .header(CONVEX_SHARED_SECRET_HEADER, &self.shared_secret)
            .json(&serde_json::json!({ "userId": user_id }))
            .send()
            .await
            .map_err(|err| EntitlementSourceError::Http(err.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(EntitlementSourceError::Status {
                status: status.as_u16(),
            });
        }
        let body = resp
            .text()
            .await
            .map_err(|err| EntitlementSourceError::Http(err.to_string()))?;

        // Convex's `internal-entitlements` returns the raw payload
        // directly. We accept either of two equivalent shapes:
        //
        //   { userId, tier, features, validUntilMs }
        //   { user_id, tier, features, valid_until_ms }
        //
        // and synthesise a `valid_until_ms` if the upstream omits it.
        let parsed: ConvexResponse = serde_json::from_str(&body).map_err(|err| {
            EntitlementSourceError::Parse(format!("{err}: {body}"))
        })?;
        let valid_until_ms = parsed
            .valid_until_ms
            .or(parsed.valid_until_ms_alt)
            .unwrap_or_else(|| {
                let ttl_ms = i64::try_from(self.snapshot_ttl.as_millis()).unwrap_or(900_000);
                now_ms + ttl_ms
            });
        let resolved_user_id = parsed
            .user_id
            .or(parsed.user_id_alt)
            .unwrap_or_else(|| user_id.to_string());
        Ok(EntitlementSnapshot {
            user_id: resolved_user_id,
            tier: parsed.tier,
            features: parsed.features.unwrap_or_default(),
            valid_until_ms,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ConvexResponse {
    #[serde(rename = "userId", default)]
    user_id: Option<String>,
    #[serde(rename = "user_id", default)]
    user_id_alt: Option<String>,
    tier: u8,
    #[serde(default)]
    features: Option<EntitlementFeatures>,
    #[serde(rename = "validUntilMs", default)]
    valid_until_ms: Option<i64>,
    #[serde(rename = "valid_until_ms", default)]
    valid_until_ms_alt: Option<i64>,
}

/// In-memory test source returning a pre-seeded snapshot. Used by
/// unit tests to drive every code path deterministically. Fields are
/// private — interact via [`Self::seed`], [`Self::install_error`],
/// [`Self::clear_error`], and [`Self::fetch_count`].
#[derive(Debug, Default)]
pub struct StaticEntitlementSource {
    snapshots: parking_lot::RwLock<std::collections::HashMap<String, EntitlementSnapshot>>,
    error: parking_lot::RwLock<Option<EntitlementSourceError>>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl StaticEntitlementSource {
    /// Construct an empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed `user_id` with `snapshot`.
    pub fn seed(&self, user_id: &str, snapshot: EntitlementSnapshot) {
        self.snapshots.write().insert(user_id.to_string(), snapshot);
    }

    /// Force the next `fetch` call to return `error`. The error
    /// stays installed until [`Self::clear_error`] is called.
    pub fn install_error(&self, error: EntitlementSourceError) {
        *self.error.write() = Some(error);
    }

    /// Drop any installed error.
    pub fn clear_error(&self) {
        *self.error.write() = None;
    }

    /// Number of times `fetch` has been invoked.
    pub fn fetch_count(&self) -> usize {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl EntitlementSource for StaticEntitlementSource {
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(err) = self.error.read().as_ref() {
            return Err(match err {
                EntitlementSourceError::Http(s) => EntitlementSourceError::Http(s.clone()),
                EntitlementSourceError::Status { status } => {
                    EntitlementSourceError::Status { status: *status }
                }
                EntitlementSourceError::Parse(s) => EntitlementSourceError::Parse(s.clone()),
            });
        }
        match self.snapshots.read().get(user_id) {
            Some(s) => Ok(s.clone()),
            None => Err(EntitlementSourceError::Status { status: 404 }),
        }
    }
}

/// Errors raised by the cache layer (sqlx wrapper).
#[derive(Debug, Error)]
pub enum CacheError {
    /// SQLite failure.
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// `features_json` did not parse.
    #[error("features parse: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Newtype around [`Pool`] that owns the entitlements_cache table
/// surface. Cheap to clone (the pool is itself an `Arc`-backed
/// handle).
#[derive(Clone, Debug)]
pub struct EntitlementCache {
    pool: Pool,
}

impl EntitlementCache {
    /// Wrap a pool.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Reference to the underlying pool. Reserved for diagnostics +
    /// migrations.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Read the cache row for `user_id` if present.
    pub async fn read(
        &self,
        user_id: &str,
    ) -> Result<Option<EntitlementSnapshot>, CacheError> {
        let row = sqlx::query(
            "SELECT user_id, tier, features_json, valid_until_ms \
             FROM entitlements_cache WHERE user_id = ?1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let user_id: String = sqlx::Row::get(&row, 0);
        let tier: i64 = sqlx::Row::get(&row, 1);
        let features_json: String = sqlx::Row::get(&row, 2);
        let valid_until_ms: i64 = sqlx::Row::get(&row, 3);
        let features: EntitlementFeatures = serde_json::from_str(&features_json)?;
        Ok(Some(EntitlementSnapshot {
            user_id,
            tier: u8::try_from(tier).unwrap_or(0),
            features,
            valid_until_ms,
        }))
    }

    /// Write (insert-or-replace) a cache row.
    pub async fn write(
        &self,
        snapshot: &EntitlementSnapshot,
    ) -> Result<(), CacheError> {
        let now_ms = pellucid_core::now_ms();
        let features_json = serde_json::to_string(&snapshot.features)?;
        sqlx::query(
            "INSERT OR REPLACE INTO entitlements_cache \
             (user_id, tier, features_json, valid_until_ms, cached_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&snapshot.user_id)
        .bind(i64::from(snapshot.tier))
        .bind(features_json)
        .bind(snapshot.valid_until_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Free-function read shim — kept so existing call sites keep
/// compiling. Prefer [`EntitlementCache::read`].
pub async fn read_cache(
    pool: &Pool,
    user_id: &str,
) -> Result<Option<EntitlementSnapshot>, CacheError> {
    EntitlementCache::new(pool.clone()).read(user_id).await
}

/// Free-function write shim — kept so existing call sites keep
/// compiling. Prefer [`EntitlementCache::write`].
pub async fn write_cache(
    pool: &Pool,
    snapshot: &EntitlementSnapshot,
) -> Result<(), CacheError> {
    EntitlementCache::new(pool.clone()).write(snapshot).await
}

/// The actual checker the gateway plugs in.
#[derive(Debug)]
pub struct ClerkEntitlementChecker {
    cache: EntitlementCache,
    source: Arc<dyn EntitlementSource>,
    upstream_down_retry_secs: u32,
}

impl ClerkEntitlementChecker {
    /// Construct a new checker with the SPEC default 30-second
    /// `Retry-After`. Internally wraps `pool` in an [`EntitlementCache`].
    #[must_use]
    pub fn new(pool: Pool, source: Arc<dyn EntitlementSource>) -> Self {
        Self::with_cache(EntitlementCache::new(pool), source)
    }

    /// Construct from an explicit cache. Reserved for callers that
    /// share an [`EntitlementCache`] instance with other components.
    #[must_use]
    pub fn with_cache(
        cache: EntitlementCache,
        source: Arc<dyn EntitlementSource>,
    ) -> Self {
        Self {
            cache,
            source,
            upstream_down_retry_secs: DEFAULT_UPSTREAM_DOWN_RETRY_SECS,
        }
    }

    /// Override the `Retry-After` value emitted on UpstreamDown.
    #[must_use]
    pub fn with_retry_after(mut self, secs: u32) -> Self {
        self.upstream_down_retry_secs = secs;
        self
    }

    /// Reference to the underlying source. Diagnostic only.
    #[must_use]
    pub fn source(&self) -> &Arc<dyn EntitlementSource> {
        &self.source
    }

    /// Reference to the underlying cache. Tests use this to assert on
    /// cache state without going through the checker.
    #[must_use]
    pub fn cache(&self) -> &EntitlementCache {
        &self.cache
    }

    /// Resolve a snapshot for `user_id`, hitting the cache first and
    /// the source on miss/expiry.
    ///
    /// Outcomes:
    /// - `Ok(Some(snapshot))` — fresh snapshot from cache or source.
    /// - `Ok(None)` — reserved for the source's
    ///   genuinely-unknown-user case (currently unreachable; the
    ///   trait doc on [`EntitlementSource::fetch`] holds the slot
    ///   for a future "user missing → anonymous tier with short TTL"
    ///   path).
    /// - `Err(ResolveError::Source(_))` — the source returned an
    ///   error after the cache layer had nothing fresh.
    ///
    /// Cache hard-failures are still warn-and-continued (treated as
    /// misses) so a corrupt SQLite row cannot DoS the gateway —
    /// only an actual source failure surfaces upward.
    pub async fn resolve(
        &self,
        user_id: &str,
    ) -> Result<Option<EntitlementSnapshot>, ResolveError> {
        let now_ms = pellucid_core::now_ms();
        // 1. Cache lookup.
        match self.cache.read(user_id).await {
            Ok(Some(snapshot)) if snapshot.is_fresh_at(now_ms) => {
                return Ok(Some(snapshot));
            }
            Ok(Some(_)) => {
                // Cache row exists but is stale. Treat as miss and
                // refresh from the source. If the source is down we
                // surface UpstreamDown rather than serving stale data.
            }
            Ok(None) => {
                // Cache miss.
            }
            Err(err) => {
                tracing::warn!(
                    target: "pellucid::auth",
                    "entitlements_cache read failed: {err}"
                );
            }
        }

        // 2. Source fetch.
        let snapshot = self.source.fetch(user_id).await?;
        if let Err(err) = self.cache.write(&snapshot).await {
            tracing::warn!(
                target: "pellucid::auth",
                "entitlements_cache write failed: {err}"
            );
        }
        Ok(Some(snapshot))
    }
}

#[async_trait]
impl EntitlementChecker for ClerkEntitlementChecker {
    async fn check(&self, user_id: &str, required: Tier) -> EntitlementDecision {
        let snapshot = match self.resolve(user_id).await {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) | Err(_) => {
                // Both genuinely-missing and source-failure currently
                // collapse to UpstreamDown. The webview's outage
                // banner is the closest UX to "we don't know your
                // entitlement right now" — better than a misleading
                // 403 + upgrade prompt (the H2 fix).
                return EntitlementDecision::UpstreamDown {
                    retry_after_secs: self.upstream_down_retry_secs,
                };
            }
        };
        let effective = snapshot.typed_tier();
        if effective.satisfies(required) {
            EntitlementDecision::Allow {
                effective_tier: effective,
            }
        } else {
            EntitlementDecision::Deny {
                effective_tier: effective,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    fn snapshot(user_id: &str, tier: u8, valid_until_ms: i64) -> EntitlementSnapshot {
        EntitlementSnapshot {
            user_id: user_id.into(),
            tier,
            features: EntitlementFeatures::default(),
            valid_until_ms,
        }
    }

    async fn pool() -> Pool {
        open_in_memory().await.unwrap()
    }

    fn now() -> i64 {
        pellucid_core::now_ms()
    }

    #[tokio::test]
    async fn cache_hit_with_sufficient_tier_allows() {
        let pool = pool().await;
        let user_id = "clerk:u1";
        write_cache(&pool, &snapshot(user_id, 3, now() + 60_000))
            .await
            .unwrap();
        let source = Arc::new(StaticEntitlementSource::new());
        let checker = ClerkEntitlementChecker::new(pool, source.clone());
        let d = checker.check(user_id, Tier::Tier1).await;
        assert!(
            matches!(d, EntitlementDecision::Allow { effective_tier: Tier::Tier2 }),
            "got {d:?}"
        );
        assert_eq!(source.fetch_count(), 0, "fresh cache must skip the source");
    }

    #[tokio::test]
    async fn cache_hit_with_insufficient_tier_denies() {
        let pool = pool().await;
        let user_id = "clerk:u1";
        write_cache(&pool, &snapshot(user_id, 1, now() + 60_000))
            .await
            .unwrap();
        let source = Arc::new(StaticEntitlementSource::new());
        let checker = ClerkEntitlementChecker::new(pool, source.clone());
        let d = checker.check(user_id, Tier::Tier2).await;
        assert!(
            matches!(d, EntitlementDecision::Deny { effective_tier: Tier::Free }),
            "got {d:?}"
        );
        assert_eq!(source.fetch_count(), 0);
    }

    #[tokio::test]
    async fn stale_cache_refetched_from_source_then_caches_again() {
        let pool = pool().await;
        let user_id = "clerk:u1";
        write_cache(&pool, &snapshot(user_id, 0, now() - 1_000))
            .await
            .unwrap();
        let source = Arc::new(StaticEntitlementSource::new());
        source.seed(user_id, snapshot(user_id, 3, now() + 60_000));
        let checker = ClerkEntitlementChecker::new(pool.clone(), source.clone());
        let d = checker.check(user_id, Tier::Tier1).await;
        assert!(matches!(
            d,
            EntitlementDecision::Allow { effective_tier: Tier::Tier2 }
        ));
        assert_eq!(source.fetch_count(), 1);
        // The new snapshot must now be in the cache; a second call
        // does not invoke the source again.
        let _ = checker.check(user_id, Tier::Tier1).await;
        assert_eq!(source.fetch_count(), 1, "second call must hit the cache");
    }

    #[tokio::test]
    async fn cache_miss_with_source_success_allows_and_writes_cache() {
        let pool = pool().await;
        let user_id = "clerk:u1";
        let source = Arc::new(StaticEntitlementSource::new());
        source.seed(user_id, snapshot(user_id, 2, now() + 60_000));
        let checker = ClerkEntitlementChecker::new(pool.clone(), source.clone());
        let d = checker.check(user_id, Tier::Free).await;
        assert!(matches!(
            d,
            EntitlementDecision::Allow { effective_tier: Tier::Tier1 }
        ));
        // Persisted in cache.
        let cached = read_cache(&pool, user_id).await.unwrap().unwrap();
        assert_eq!(cached.tier, 2);
    }

    #[tokio::test]
    async fn cache_miss_with_source_5xx_returns_upstream_down_h2() {
        let pool = pool().await;
        let source = Arc::new(StaticEntitlementSource::new());
        source.install_error(EntitlementSourceError::Status { status: 503 });
        let checker = ClerkEntitlementChecker::new(pool, source);
        let d = checker.check("clerk:u1", Tier::Tier2).await;
        match d {
            EntitlementDecision::UpstreamDown { retry_after_secs } => {
                assert_eq!(retry_after_secs, DEFAULT_UPSTREAM_DOWN_RETRY_SECS);
            }
            other => panic!("expected UpstreamDown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cache_miss_with_source_unreachable_returns_upstream_down() {
        let pool = pool().await;
        let source = Arc::new(StaticEntitlementSource::new());
        source.install_error(EntitlementSourceError::Http("connection refused".into()));
        let checker = ClerkEntitlementChecker::new(pool, source);
        let d = checker.check("clerk:u1", Tier::Tier2).await;
        assert!(matches!(d, EntitlementDecision::UpstreamDown { .. }));
    }

    #[tokio::test]
    async fn upstream_down_retry_after_overridable() {
        let pool = pool().await;
        let source = Arc::new(StaticEntitlementSource::new());
        source.install_error(EntitlementSourceError::Status { status: 504 });
        let checker = ClerkEntitlementChecker::new(pool, source).with_retry_after(7);
        let d = checker.check("clerk:u1", Tier::Tier2).await;
        match d {
            EntitlementDecision::UpstreamDown { retry_after_secs } => {
                assert_eq!(retry_after_secs, 7);
            }
            other => panic!("expected UpstreamDown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cache_round_trip_preserves_features() {
        let pool = pool().await;
        let user_id = "clerk:u-features";
        let snapshot = EntitlementSnapshot {
            user_id: user_id.into(),
            tier: 2,
            features: EntitlementFeatures {
                max_dashboards: 5,
                api_access: true,
                api_rate_limit: 600,
                priority_support: false,
                export_formats: vec!["json".into(), "csv".into()],
            },
            valid_until_ms: now() + 60_000,
        };
        write_cache(&pool, &snapshot).await.unwrap();
        let round = read_cache(&pool, user_id).await.unwrap().unwrap();
        assert_eq!(round, snapshot);
    }

    #[tokio::test]
    async fn read_cache_missing_user_returns_none() {
        let pool = pool().await;
        let result = read_cache(&pool, "no-such-user").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn typed_tier_clamps_unknown_high_rank_to_tier2() {
        let s = snapshot("u", 99, now() + 1_000);
        assert_eq!(s.typed_tier(), Tier::Tier2);
    }

    #[tokio::test]
    async fn convex_endpoint_is_built_from_base_plus_path() {
        let s = ConvexEntitlementSource::new(
            "https://convex.test",
            "secret",
            reqwest::Client::new(),
        );
        assert_eq!(s.endpoint(), "https://convex.test/api/internal-entitlements");
    }
}

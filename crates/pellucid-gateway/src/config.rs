//! Gateway configuration — what `build_router` accepts.
//!
//! Each section maps 1:1 onto a stage's needs:
//!
//! - [`OriginAllowList`] → stage 1 (origin allow-list).
//! - [`CorsConfig`] → stages 2 + 3 (CORS merge + preflight).
//! - [`RouteEntitlementRules`] → stages 4 + 7 (tier gate +
//!   entitlement check).
//! - [`RouteRateLimitRules`] → stages 8 + 9 (per-endpoint + global
//!   rate limits).
//! - [`RouteCacheRules`] → stage 14 (cache-control headers).

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{HeaderName, Method};
use parking_lot::RwLock;
use pellucid_cache::rate_limit::{BucketConfig, RateLimitConfig};

use crate::traits::{ApiKeyStore, ClerkVerifier, EntitlementChecker, Tier};

/// Allow-list of acceptable `Origin` header values. Stage 1 returns
/// 403 when a request's `Origin` is non-empty and *not* in this list.
/// Empty `Origin` (same-origin / direct hit) is always allowed since
/// browsers omit `Origin` for same-origin GETs and tools like curl
/// never send one.
#[derive(Clone, Debug, Default)]
pub struct OriginAllowList {
    /// Exact-match entries (e.g. `https://worldmonitor.app`).
    pub exact: Vec<String>,
    /// Wildcard suffix entries (e.g. `.worldmonitor.app` matches
    /// `https://staging.worldmonitor.app`).
    pub suffixes: Vec<String>,
    /// When `true`, any `Origin` matching `127.0.0.1` or `localhost`
    /// (with any port) is allowed. Used by the desktop sidecar where
    /// the webview origin is dynamic.
    pub allow_loopback: bool,
}

impl OriginAllowList {
    /// Construct a new allow-list with no entries (allow only empty
    /// `Origin`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an exact-match origin.
    pub fn allow_exact(&mut self, origin: impl Into<String>) -> &mut Self {
        self.exact.push(origin.into());
        self
    }

    /// Add a suffix-match origin (e.g. `.worldmonitor.app`).
    pub fn allow_suffix(&mut self, suffix: impl Into<String>) -> &mut Self {
        self.suffixes.push(suffix.into());
        self
    }

    /// Toggle loopback exemption.
    pub fn with_loopback(mut self, allow: bool) -> Self {
        self.allow_loopback = allow;
        self
    }

    /// `true` iff `origin` is permitted.
    #[must_use]
    pub fn permits(&self, origin: &str) -> bool {
        if origin.is_empty() {
            return true;
        }
        if self.exact.iter().any(|e| e == origin) {
            return true;
        }
        if self
            .suffixes
            .iter()
            .any(|s| origin.ends_with(s.as_str()))
        {
            return true;
        }
        if self.allow_loopback && is_loopback_origin(origin) {
            return true;
        }
        false
    }
}

fn is_loopback_origin(origin: &str) -> bool {
    let lower = origin.to_ascii_lowercase();
    let host = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .or_else(|| lower.strip_prefix("tauri://"))
        .unwrap_or(&lower);
    let host = host.split('/').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// CORS configuration consumed by stages 2 + 3.
#[derive(Clone, Debug)]
pub struct CorsConfig {
    /// Methods the API accepts.
    pub allowed_methods: Vec<Method>,
    /// Request headers permitted on cross-origin requests.
    pub allowed_headers: Vec<HeaderName>,
    /// `Access-Control-Max-Age` in seconds.
    pub max_age_secs: u32,
    /// Whether to set `Access-Control-Allow-Credentials: true`.
    pub allow_credentials: bool,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_methods: vec![
                Method::GET,
                Method::POST,
                Method::OPTIONS,
                Method::PUT,
                Method::DELETE,
            ],
            allowed_headers: vec![
                HeaderName::from_static("authorization"),
                HeaderName::from_static("content-type"),
                HeaderName::from_static("if-none-match"),
                HeaderName::from_static("x-api-key"),
                HeaderName::from_static("x-pellucid-tier"),
            ],
            max_age_secs: 86_400,
            allow_credentials: true,
        }
    }
}

/// Per-route entitlement rules: which tier each path requires, plus a
/// flag for "Clerk JWT required" so stage 5 can short-circuit on
/// non-tier-gated routes.
#[derive(Clone, Debug, Default)]
pub struct RouteEntitlementRules {
    /// `path` → required tier. Missing entries default to
    /// [`Tier::Anonymous`] (no auth needed).
    pub by_path: HashMap<String, Tier>,
}

impl RouteEntitlementRules {
    /// Construct an empty rule set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a route's required tier.
    pub fn require(&mut self, path: impl Into<String>, tier: Tier) -> &mut Self {
        self.by_path.insert(path.into(), tier);
        self
    }

    /// Required tier for `path`. Defaults to `Tier::Anonymous`.
    #[must_use]
    pub fn required_for(&self, path: &str) -> Tier {
        self.by_path
            .get(path)
            .copied()
            .unwrap_or(Tier::Anonymous)
    }
}

/// Per-route rate-limit overrides.
#[derive(Clone, Debug, Default)]
pub struct RouteRateLimitRules {
    /// Default applied when a path has no override.
    pub default: RateLimitConfig,
    /// Per-path overrides.
    pub by_path: HashMap<String, RateLimitConfig>,
}

impl RouteRateLimitRules {
    /// Construct rules with a custom default.
    #[must_use]
    pub fn with_default(default: RateLimitConfig) -> Self {
        Self {
            default,
            by_path: HashMap::new(),
        }
    }

    /// Override `path`'s endpoint bucket only.
    pub fn override_endpoint(&mut self, path: impl Into<String>, cfg: BucketConfig) -> &mut Self {
        let key = path.into();
        let mut existing = self
            .by_path
            .get(&key)
            .copied()
            .unwrap_or(self.default);
        existing.endpoint = cfg;
        self.by_path.insert(key, existing);
        self
    }

    /// Resolve the effective config for `path`.
    #[must_use]
    pub fn for_path(&self, path: &str) -> RateLimitConfig {
        self.by_path.get(path).copied().unwrap_or(self.default)
    }
}

/// Cache-Control policy applied by stage 14.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheControlPolicy {
    /// Cache-Control header value.
    pub header: String,
}

impl CacheControlPolicy {
    /// `Cache-Control: no-store` — used for tier-gated and authorised
    /// responses so intermediaries do not cache personalised content.
    #[must_use]
    pub fn no_store() -> Self {
        Self {
            header: "no-store".into(),
        }
    }

    /// `Cache-Control: public, max-age=<secs>` for anonymous routes.
    #[must_use]
    pub fn public(max_age_secs: u32) -> Self {
        Self {
            header: format!("public, max-age={max_age_secs}"),
        }
    }
}

impl Default for CacheControlPolicy {
    fn default() -> Self {
        Self::no_store()
    }
}

/// Per-route Cache-Control overrides.
#[derive(Clone, Debug, Default)]
pub struct RouteCacheRules {
    /// Default policy for unmapped paths.
    pub default: CacheControlPolicy,
    /// Per-path overrides.
    pub by_path: HashMap<String, CacheControlPolicy>,
}

impl RouteCacheRules {
    /// Construct with default `no-store`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a path's policy.
    pub fn set(&mut self, path: impl Into<String>, policy: CacheControlPolicy) -> &mut Self {
        self.by_path.insert(path.into(), policy);
        self
    }

    /// Resolve the policy for `path`.
    #[must_use]
    pub fn for_path(&self, path: &str) -> CacheControlPolicy {
        self.by_path.get(path).cloned().unwrap_or_else(|| self.default.clone())
    }
}

/// Full gateway configuration. Construct via [`GatewayConfig::builder`].
#[derive(Clone, Debug)]
pub struct GatewayConfig {
    /// Stage 1.
    pub origins: OriginAllowList,
    /// Stages 2 + 3.
    pub cors: CorsConfig,
    /// Stages 4 + 7.
    pub route_tiers: RouteEntitlementRules,
    /// Stages 8 + 9.
    pub rate_limits: RouteRateLimitRules,
    /// Stage 14.
    pub cache_rules: RouteCacheRules,
    /// Pluggable Clerk verifier (stage 5).
    pub clerk: Arc<dyn ClerkVerifier>,
    /// Pluggable API key store (stage 6).
    pub api_keys: Arc<dyn ApiKeyStore>,
    /// Pluggable entitlement checker (stage 7).
    pub entitlement: Arc<dyn EntitlementChecker>,
    /// Optional rate-limit pool. When `None`, stages 8 + 9 short-circuit
    /// to `Allow` so the gateway works in tests that have not opened
    /// a database. Production deployments always set this.
    pub rate_limit_pool: Option<Arc<RwLock<Option<pellucid_db::Pool>>>>,
}

impl GatewayConfig {
    /// Default config for tests + the M0 web shard. Allows all
    /// origins, no rate limiting, all entitlements granted.
    #[must_use]
    pub fn permissive_for_tests() -> Self {
        Self {
            origins: OriginAllowList::new().with_loopback(true),
            cors: CorsConfig::default(),
            route_tiers: RouteEntitlementRules::new(),
            rate_limits: RouteRateLimitRules::default(),
            cache_rules: RouteCacheRules::new(),
            clerk: Arc::new(crate::traits::NoopClerkVerifier),
            api_keys: Arc::new(crate::traits::NoopApiKeyStore),
            entitlement: Arc::new(crate::traits::AlwaysAllowEntitlement),
            rate_limit_pool: None,
        }
    }

    /// Builder constructor.
    #[must_use]
    pub fn builder() -> GatewayConfigBuilder {
        GatewayConfigBuilder::default()
    }
}

/// Builder for [`GatewayConfig`].
#[derive(Default)]
#[allow(missing_debug_implementations)] // contains Arc<dyn Trait> values that don't impl Debug uniformly
pub struct GatewayConfigBuilder {
    origins: Option<OriginAllowList>,
    cors: Option<CorsConfig>,
    route_tiers: Option<RouteEntitlementRules>,
    rate_limits: Option<RouteRateLimitRules>,
    cache_rules: Option<RouteCacheRules>,
    clerk: Option<Arc<dyn ClerkVerifier>>,
    api_keys: Option<Arc<dyn ApiKeyStore>>,
    entitlement: Option<Arc<dyn EntitlementChecker>>,
    rate_limit_pool: Option<Arc<RwLock<Option<pellucid_db::Pool>>>>,
}

impl GatewayConfigBuilder {
    /// Set the origin allow-list.
    #[must_use]
    pub fn origins(mut self, value: OriginAllowList) -> Self {
        self.origins = Some(value);
        self
    }

    /// Set the CORS configuration.
    #[must_use]
    pub fn cors(mut self, value: CorsConfig) -> Self {
        self.cors = Some(value);
        self
    }

    /// Set the route tier rules.
    #[must_use]
    pub fn route_tiers(mut self, value: RouteEntitlementRules) -> Self {
        self.route_tiers = Some(value);
        self
    }

    /// Set the rate-limit rules.
    #[must_use]
    pub fn rate_limits(mut self, value: RouteRateLimitRules) -> Self {
        self.rate_limits = Some(value);
        self
    }

    /// Set the cache rules.
    #[must_use]
    pub fn cache_rules(mut self, value: RouteCacheRules) -> Self {
        self.cache_rules = Some(value);
        self
    }

    /// Plug in the real Clerk verifier (T2.2).
    #[must_use]
    pub fn clerk(mut self, value: Arc<dyn ClerkVerifier>) -> Self {
        self.clerk = Some(value);
        self
    }

    /// Plug in the real API key store.
    #[must_use]
    pub fn api_keys(mut self, value: Arc<dyn ApiKeyStore>) -> Self {
        self.api_keys = Some(value);
        self
    }

    /// Plug in the real entitlement checker (T2.3).
    #[must_use]
    pub fn entitlement(mut self, value: Arc<dyn EntitlementChecker>) -> Self {
        self.entitlement = Some(value);
        self
    }

    /// Plug in the rate-limit pool (T1.4 sqlite handle).
    #[must_use]
    pub fn rate_limit_pool(
        mut self,
        value: Arc<RwLock<Option<pellucid_db::Pool>>>,
    ) -> Self {
        self.rate_limit_pool = Some(value);
        self
    }

    /// Build the final config. Missing fields default to the
    /// permissive-for-tests value so the builder is ergonomic in
    /// test code.
    #[must_use]
    pub fn build(self) -> GatewayConfig {
        let permissive = GatewayConfig::permissive_for_tests();
        GatewayConfig {
            origins: self.origins.unwrap_or(permissive.origins),
            cors: self.cors.unwrap_or(permissive.cors),
            route_tiers: self.route_tiers.unwrap_or(permissive.route_tiers),
            rate_limits: self.rate_limits.unwrap_or(permissive.rate_limits),
            cache_rules: self.cache_rules.unwrap_or(permissive.cache_rules),
            clerk: self.clerk.unwrap_or(permissive.clerk),
            api_keys: self.api_keys.unwrap_or(permissive.api_keys),
            entitlement: self.entitlement.unwrap_or(permissive.entitlement),
            rate_limit_pool: self.rate_limit_pool,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_origin_is_permitted() {
        let allow = OriginAllowList::new();
        assert!(allow.permits(""));
    }

    #[test]
    fn exact_match_permitted() {
        let mut allow = OriginAllowList::new();
        allow.allow_exact("https://worldmonitor.app");
        assert!(allow.permits("https://worldmonitor.app"));
        assert!(!allow.permits("https://evil.example"));
    }

    #[test]
    fn suffix_match_permitted() {
        let mut allow = OriginAllowList::new();
        allow.allow_suffix(".worldmonitor.app");
        assert!(allow.permits("https://staging.worldmonitor.app"));
        assert!(!allow.permits("https://worldmonitor.app.evil"));
    }

    #[test]
    fn loopback_permitted_when_flag_set() {
        let allow = OriginAllowList::new().with_loopback(true);
        assert!(allow.permits("http://127.0.0.1:46123"));
        assert!(allow.permits("http://localhost:5173"));
        assert!(allow.permits("tauri://localhost"));
    }

    #[test]
    fn loopback_denied_when_flag_unset() {
        let allow = OriginAllowList::new();
        assert!(!allow.permits("http://127.0.0.1:46123"));
    }

    #[test]
    fn route_tier_defaults_to_anonymous() {
        let rules = RouteEntitlementRules::new();
        assert_eq!(rules.required_for("/api/anything"), Tier::Anonymous);
    }

    #[test]
    fn route_tier_lookup() {
        let mut rules = RouteEntitlementRules::new();
        rules.require("/api/aviation/v1/get-flight-status", Tier::Tier1);
        assert_eq!(
            rules.required_for("/api/aviation/v1/get-flight-status"),
            Tier::Tier1
        );
    }

    #[test]
    fn rate_limit_override_keeps_global_and_aggregate() {
        let mut rules = RouteRateLimitRules::default();
        rules.override_endpoint(
            "/api/x",
            BucketConfig {
                limit: 9,
                window_ms: 1_000,
            },
        );
        let cfg = rules.for_path("/api/x");
        assert_eq!(cfg.endpoint.limit, 9);
        assert_eq!(cfg.endpoint.window_ms, 1_000);
        assert_eq!(cfg.global.limit, 600); // unchanged
    }

    #[test]
    fn cache_rules_default_is_no_store() {
        let rules = RouteCacheRules::new();
        assert_eq!(rules.for_path("/x"), CacheControlPolicy::no_store());
    }

    #[test]
    fn cache_rules_per_path_override() {
        let mut rules = RouteCacheRules::new();
        rules.set("/cacheable", CacheControlPolicy::public(60));
        assert_eq!(rules.for_path("/cacheable").header, "public, max-age=60");
    }

    #[test]
    fn cors_defaults_include_authorization_and_content_type() {
        let c = CorsConfig::default();
        assert!(c.allowed_methods.contains(&Method::GET));
        assert!(c.allowed_methods.contains(&Method::OPTIONS));
        assert!(c
            .allowed_headers
            .iter()
            .any(|h| h.as_str() == "authorization"));
        assert!(c.allowed_headers.iter().any(|h| h.as_str() == "content-type"));
    }

    #[test]
    fn permissive_config_builds() {
        let c = GatewayConfig::permissive_for_tests();
        assert!(c.origins.allow_loopback);
    }

    #[test]
    fn builder_overrides_apply() {
        let mut tiers = RouteEntitlementRules::new();
        tiers.require("/api/x", Tier::Tier2);
        let cfg = GatewayConfig::builder().route_tiers(tiers).build();
        assert_eq!(cfg.route_tiers.required_for("/api/x"), Tier::Tier2);
    }
}

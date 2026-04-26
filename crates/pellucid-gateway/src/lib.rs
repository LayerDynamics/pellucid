//! pellucid-gateway — 14-stage Tower middleware stack.
//!
//! Direct port of the original WorldMonitor `server/gateway.ts` plus
//! every fix flagged in `LoreDeepCodeReview.md` Path B (OP-3). Every
//! request flows through the same ordered pipeline:
//!
//! ```text
//! Stage 1  Origin allow-list                  → 403 on bad Origin
//! Stage 2  CORS merge                          → adds CORS headers
//! Stage 3  OPTIONS preflight                   → 204 short-circuit
//! Stage 4  Tier gate                           → marks tier-2 routes
//! Stage 5  Clerk session                       → 401 on bad/missing JWT
//! Stage 6  API key                             → 401 on bad x-api-key
//! Stage 7  Entitlement                         → 403 / 503 (H2 fix)
//! Stage 8  Endpoint rate-limit                 → 429 + Retry-After
//! Stage 9  Global rate-limit                   → 429 + Retry-After
//! (Stage 10 = Axum router; 404 / 405)
//! Stage 11 Handler error boundary              → 500 on panic / Err
//! Stage 12 Header merge                        → standard response headers
//! Stage 13 ETag                                → 304 on If-None-Match
//! Stage 14 Cache-Control                       → per-route policy
//! ```
//!
//! `build_router(handlers, config)` composes the stack and returns an
//! `axum::Router`. Both `pellucid-edge-bin` and `pellucid-sidecar-bin`
//! mount the same router so the two deployments share a single source
//! of truth.

pub mod config;
pub mod error_mapper;
pub mod identity;
pub mod router;
pub mod stages;
pub mod traits;

pub use config::{
    CacheControlPolicy, CorsConfig, GatewayConfig, OriginAllowList, RouteCacheRules,
    RouteEntitlementRules, RouteRateLimitRules,
};
pub use error_mapper::{GatewayError, GatewayResponse};
pub use identity::{ApiIdentity, ClientIdentity, RequestIdentity};
pub use router::{build_router, GatewayState, HandlerSet};
pub use traits::{
    AlwaysAllowEntitlement, AlwaysDenyEntitlement, ApiKeyDecision, ApiKeyStore, ClerkClaims,
    ClerkVerifier, EntitlementChecker, EntitlementDecision, NoopApiKeyStore, NoopClerkVerifier,
    Tier,
};

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}

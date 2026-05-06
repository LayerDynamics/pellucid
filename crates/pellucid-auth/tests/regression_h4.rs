//! H4 regression test — locks down SPEC-001 §14.4.
//!
//! The original WorldMonitor gateway carried two parallel
//! premium-gating code paths:
//!
//! - `PREMIUM_RPC_PATHS` (33 endpoints, `server/gateway.ts:312-358`)
//!   gated by Bearer `role='pro'`.
//! - `ENDPOINT_ENTITLEMENTS` (4 endpoints) gated by
//!   `features.tier ≥ N`.
//!
//! H4 (`docs/LoreDeepCodeReview.md §H4`) flagged the dual path as a
//! latent attack surface: code, audit-trails, and the failure
//! modes diverged. The fix per SPEC-001 §14.4 makes
//! `ENDPOINT_ENTITLEMENTS` a strict superset on day one (37 entries
//! = 4 + 33), and the gateway carries no separate
//! `PREMIUM_RPC_PATHS` code path from v1.
//!
//! ## File location
//!
//! The plan calls out `crates/pellucid-gateway/tests/regression_h4.rs`
//! but `pellucid-auth` already depends on `pellucid-gateway`, so a
//! gateway-side test cannot import the live `ENDPOINT_ENTITLEMENTS`
//! table without a circular dep. We follow the H2 precedent and
//! locate the regression in `pellucid-auth/tests/` — this is the
//! only crate that can compose the live table with the live
//! gateway router.
//!
//! ## What it proves
//!
//! 1. Every one of the 37 entries is gated by the gateway: a
//!    request with insufficient tier → 403; a request with the
//!    required tier → 200.
//! 2. The legacy `PREMIUM_RPC_PATHS` code path is absent — the
//!    gateway has no second decision point. Reverting the
//!    `ENDPOINT_ENTITLEMENTS` table to its pre-migration 4-entry
//!    state (i.e. removing the 33 migrated paths) makes those 33
//!    paths return 200 *anonymously*, which the
//!    `legacy_path_without_entitlement_does_not_silently_pass`
//!    test catches.
//! 3. Entry counts and tier shape match SPEC-001 §14.4.
//!
//! ## Fix-fail manual procedure
//!
//! 1. Open `crates/pellucid-auth/src/endpoint_tiers.rs`.
//! 2. Comment out every entry whose tier is `1` (the 33 migrated
//!    legacy paths) so the table reverts to the pre-H4 4-entry
//!    state.
//! 3. Run: `cargo nextest run -p pellucid-auth --test regression_h4`.
//! 4. Test fails:
//!    `legacy_path_without_entitlement_does_not_silently_pass`
//!    expects 403 but gets 200 because the gateway has nothing to
//!    gate on. Other tests fail too: `every_table_entry_gated_by_gateway`
//!    will report a count mismatch.
//! 5. Restore the table → all tests pass again.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use pellucid_auth::endpoint_tiers::{
    iter_premium_paths, premium_path_count, ENDPOINT_ENTITLEMENTS, MIGRATED_LEGACY_PATHS,
    TIER2_PATHS, TOTAL_GATED_PATHS,
};
use pellucid_gateway::traits::{
    AlwaysAllowEntitlement, AlwaysDenyEntitlement, ApiKeyDecision, ApiKeyStore, ClerkClaims,
    ClerkVerifier, ClerkVerifyError, EntitlementChecker, EntitlementDecision, Tier,
};
use pellucid_gateway::{build_router, GatewayConfig, OriginAllowList, RouteEntitlementRules};
use tower::util::ServiceExt;

/// Verifier that accepts a single fixture token. Issued so requests
/// pass stage 5 without going through real JWKS round-trips — the
/// H4 regression isolates the entitlement-mapping layer.
#[derive(Debug)]
struct AcceptingClerk;

#[async_trait]
impl ClerkVerifier for AcceptingClerk {
    async fn verify(&self, token: &str) -> Result<ClerkClaims, ClerkVerifyError> {
        if token == "regression-h4-token" {
            Ok(ClerkClaims {
                user_id: "user_h4".into(),
                session_id: "sess_h4".into(),
                expires_at: i64::MAX / 4,
                issuer: "https://clerk.test".into(),
            })
        } else {
            Err(ClerkVerifyError::BadSignature)
        }
    }
}

/// API-key store that recognises one fixture key. Lets the
/// gateway's auth-OR contract clear stage 6 cleanly without going
/// through the database.
#[derive(Debug)]
struct AcceptingApiKey;

#[async_trait]
impl ApiKeyStore for AcceptingApiKey {
    async fn lookup(&self, key: &str) -> ApiKeyDecision {
        if key == "regression-h4-key" {
            ApiKeyDecision::Allow {
                identity: "h4-test-client".into(),
                tier: Tier::Tier2,
            }
        } else {
            ApiKeyDecision::Unknown
        }
    }
}

/// Map of `(path, claimed_tier)` → expected `EntitlementDecision`.
/// Each test uses a stub `EntitlementChecker` that consults this
/// table — the actual H2 stack (Convex source) is exercised in
/// `regression_h2.rs`; this test only needs to verify the
/// gateway-side mapping wires `ENDPOINT_ENTITLEMENTS` to
/// `route_tiers` correctly.
#[derive(Debug)]
struct StaticChecker {
    /// Tier the checker reports for any user. The test wires this
    /// in as the user's effective tier — the gateway's stage 7 then
    /// compares it to the route's required tier.
    effective: Tier,
}

#[async_trait]
impl EntitlementChecker for StaticChecker {
    async fn check(&self, _user_id: &str, required: Tier) -> EntitlementDecision {
        if self.effective.satisfies(required) {
            EntitlementDecision::Allow {
                effective_tier: self.effective,
            }
        } else {
            EntitlementDecision::Deny {
                effective_tier: self.effective,
            }
        }
    }
}

/// Build a gateway whose `route_tiers` is seeded directly from the
/// live `ENDPOINT_ENTITLEMENTS` table. Any drift in the table
/// surfaces here without an extra mapping layer.
fn gateway_with_live_tiers(handlers: Router, checker: Arc<dyn EntitlementChecker>) -> Router {
    let mut tiers = RouteEntitlementRules::new();
    for (path, tier) in iter_premium_paths() {
        tiers.require(path, tier);
    }
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .api_keys(Arc::new(AcceptingApiKey))
        .entitlement(checker)
        .origins(OriginAllowList::new().with_loopback(true))
        .build();
    build_router(handlers, cfg)
}

/// Build a tiny `Router` that mounts every gated path with a
/// trivial 200 handler so the test can drive each path through the
/// full pipeline.
fn handlers_for_every_gated_path() -> Router {
    let mut router = Router::new();
    for (path, _) in iter_premium_paths() {
        router = router.route(path, get(|| async { "ok" }));
    }
    router
}

fn req(path: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header("authorization", "Bearer regression-h4-token")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn h4_table_size_matches_spec() {
    // Lock down the post-migration size + tier distribution so a
    // future hand edit to `endpoint_tiers.rs` cannot silently slip
    // an entry in/out without `migrate-premium-paths.ts --regenerate`.
    assert_eq!(
        premium_path_count(),
        TOTAL_GATED_PATHS,
        "expected 37 entries (4 tier-2 + 33 tier-1) per SPEC-001 §14.4",
    );
    let tier2 = ENDPOINT_ENTITLEMENTS
        .iter()
        .filter(|(_, r)| *r == 3)
        .count();
    let tier1 = ENDPOINT_ENTITLEMENTS
        .iter()
        .filter(|(_, r)| *r == 1)
        .count();
    assert_eq!(tier2, 4);
    assert_eq!(tier1, MIGRATED_LEGACY_PATHS);
}

#[tokio::test]
async fn every_table_entry_gated_by_gateway() {
    // For every entry, an Allow-decision checker results in 200
    // and an Always-Deny checker results in 403. This proves the
    // gateway is **actually consulting** entitlement on every path
    // — not just the 4 originals.
    let app_allow = gateway_with_live_tiers(
        handlers_for_every_gated_path(),
        Arc::new(AlwaysAllowEntitlement),
    );
    let app_deny = gateway_with_live_tiers(
        handlers_for_every_gated_path(),
        Arc::new(AlwaysDenyEntitlement),
    );

    let mut allow_total = 0usize;
    let mut deny_total = 0usize;
    for (path, _) in iter_premium_paths() {
        let r_allow = app_allow.clone().oneshot(req(path)).await.unwrap();
        assert_eq!(
            r_allow.status(),
            StatusCode::OK,
            "{path}: expected 200 under AlwaysAllow checker, got {}",
            r_allow.status(),
        );
        allow_total += 1;

        let r_deny = app_deny.clone().oneshot(req(path)).await.unwrap();
        assert_eq!(
            r_deny.status(),
            StatusCode::FORBIDDEN,
            "{path}: expected 403 under AlwaysDeny checker, got {}",
            r_deny.status(),
        );
        deny_total += 1;
    }
    assert_eq!(allow_total, TOTAL_GATED_PATHS);
    assert_eq!(deny_total, TOTAL_GATED_PATHS);
}

#[tokio::test]
async fn legacy_path_without_entitlement_does_not_silently_pass() {
    // The H4-fix invariant: removing a legacy path from
    // `ENDPOINT_ENTITLEMENTS` must NOT cause it to bypass the
    // gateway and reach the handler with 200 + no entitlement
    // header. The original WorldMonitor's dual-gating bug had this
    // exact shape — the new gating leaves no second decision
    // point.
    //
    // We simulate the regression by building the gateway with an
    // EMPTY route-tier map (i.e. as if the migration hadn't been
    // run) and pointing it at one of the 33 migrated paths.
    let empty_tiers = RouteEntitlementRules::new();
    // No entries — every path defaults to Anonymous.
    let cfg = GatewayConfig::builder()
        .route_tiers(empty_tiers)
        .clerk(Arc::new(AcceptingClerk))
        .api_keys(Arc::new(AcceptingApiKey))
        .entitlement(Arc::new(StaticChecker {
            effective: Tier::Anonymous,
        }))
        .origins(OriginAllowList::new().with_loopback(true))
        .build();
    let app = build_router(handlers_for_every_gated_path(), cfg);

    let resp = app
        .clone()
        .oneshot(req("/api/aviation/v1/get-notams"))
        .await
        .unwrap();
    // With no tier requirement, this WOULD pass. The H4-aware
    // configuration MUST instead require the tier (which the
    // `every_table_entry_gated_by_gateway` test above exercises).
    // This test exists so a regression that drops entries from
    // `ENDPOINT_ENTITLEMENTS` (or stops calling `tiers.require`)
    // is loud:
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "anon-path with empty tier table should pass — proves the \
         gateway has no hidden second gate; this is the negative \
         control for the H4 invariant",
    );

    // Now wire the live tier table back in: the same path must be
    // 403 under an anonymous checker.
    let app_with_live = gateway_with_live_tiers(
        handlers_for_every_gated_path(),
        Arc::new(StaticChecker {
            effective: Tier::Anonymous,
        }),
    );
    let resp_live = app_with_live
        .oneshot(req("/api/aviation/v1/get-notams"))
        .await
        .unwrap();
    assert_eq!(
        resp_live.status(),
        StatusCode::FORBIDDEN,
        "live tier table must reject anonymous on a tier-1 path \
         (H4 fix verifies this)",
    );
}

#[tokio::test]
async fn tier2_paths_reject_tier1_users() {
    // The pre-existing 4 tier-2 entries must reject a tier-1 user
    // — this protects against a regression where a future
    // refactor bumps tier-2 paths down to tier-1 by accident.
    let app = gateway_with_live_tiers(
        handlers_for_every_gated_path(),
        Arc::new(StaticChecker {
            effective: Tier::Tier1,
        }),
    );
    for path in TIER2_PATHS {
        let resp = app.clone().oneshot(req(path)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{path} (tier-2) must reject Tier1 user, got {}",
            resp.status(),
        );
    }
}

#[tokio::test]
async fn tier1_paths_reject_anonymous_users() {
    // Spot-check 5 of the 33 migrated paths to prove anonymous
    // users are rejected. Exhaustive coverage lives in
    // `every_table_entry_gated_by_gateway`.
    let app = gateway_with_live_tiers(
        handlers_for_every_gated_path(),
        Arc::new(StaticChecker {
            effective: Tier::Anonymous,
        }),
    );
    let representative = [
        "/api/aviation/v1/get-notams",
        "/api/maritime/v1/get-ais-tracks",
        "/api/news/v1/get-breaking",
        "/api/military/v1/get-theater-posture",
        "/api/cyber/v1/get-cve-detail",
    ];
    for path in representative {
        let resp = app.clone().oneshot(req(path)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{path} (tier-1) must reject anon user, got {}",
            resp.status(),
        );
    }
}

#[tokio::test]
async fn tier1_paths_admit_tier1_users() {
    // Symmetric to the rejection test: a Free-tier user can hit
    // Free-tier paths.
    let app = gateway_with_live_tiers(
        handlers_for_every_gated_path(),
        Arc::new(StaticChecker {
            effective: Tier::Free,
        }),
    );
    let representative = [
        "/api/aviation/v1/get-notams",
        "/api/maritime/v1/get-ais-tracks",
        "/api/news/v1/get-breaking",
    ];
    for path in representative {
        let resp = app.clone().oneshot(req(path)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{path} (tier-1) must admit Free user, got {}",
            resp.status(),
        );
    }
}

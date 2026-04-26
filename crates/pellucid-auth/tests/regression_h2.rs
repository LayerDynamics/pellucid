//! H2 regression test — locks down SPEC-001 §14.1 / §24.
//!
//! The original WorldMonitor entitlement-check collapsed upstream
//! failures into 403 + "upgrade your plan", which surfaced to the
//! webview as a misleading prompt during a Convex outage. The H2
//! fix introduces a three-arm `EntitlementDecision::UpstreamDown`
//! that maps to **503 + `Retry-After: 30`** through gateway stage 7,
//! letting the webview render an outage banner instead.
//!
//! ## Fix-fail manual procedure
//!
//! 1. Open `crates/pellucid-auth/src/entitlement.rs`.
//! 2. Edit `EntitlementChecker::check`'s match arm so
//!    `UpstreamDown` is replaced with
//!    `EntitlementDecision::Deny { effective_tier: Tier::Free }`.
//! 3. Run: `cargo nextest run -p pellucid-auth --test regression_h2`.
//! 4. Test fails: gateway emits 403 (not 503), no `Retry-After`.
//! 5. Restore the original code → test passes again.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use pellucid_auth::test_keys::{fixture_jwk_set, fixture_signing_key, FIXTURE_KID};
use pellucid_auth::{ClerkEntitlementChecker, ClerkJwtVerifier, ConvexEntitlementSource};
use pellucid_db::open_in_memory;
use pellucid_gateway::traits::{ClerkClaims, ClerkVerifier, ClerkVerifyError, Tier};
use pellucid_gateway::{
    build_router, GatewayConfig, OriginAllowList, RouteEntitlementRules,
};
use serde_json::json;
use tower::util::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISSUER: &str = "https://clerk.test";
const TIER2_PATH: &str = "/api/market/v1/analyze-stock";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// In-process Clerk verifier that accepts a single fixture token.
/// Used so the regression isolates the H2 path — Clerk verification
/// itself is exercised in `tests/clerk_e2e.rs`.
#[derive(Debug)]
struct TestClerk;

#[async_trait]
impl ClerkVerifier for TestClerk {
    async fn verify(&self, token: &str) -> Result<ClerkClaims, ClerkVerifyError> {
        if token == "regression-h2-token" {
            Ok(ClerkClaims {
                user_id: "user_h2".into(),
                session_id: "sess_h2".into(),
                expires_at: now_secs() + 3600,
                issuer: ISSUER.into(),
            })
        } else {
            Err(ClerkVerifyError::BadSignature)
        }
    }
}

fn handlers() -> Router {
    Router::new()
        .route(TIER2_PATH, get(|| async { "secret-stock-data" }))
}

#[tokio::test]
async fn convex_5xx_with_cold_cache_yields_503_plus_retry_after_30() {
    // Arrange — a real Convex stand-in returning 5xx.
    let convex = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/internal-entitlements"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&convex)
        .await;

    let pool = open_in_memory().await.unwrap();
    let source = Arc::new(ConvexEntitlementSource::new(
        convex.uri(),
        "shared-secret",
        reqwest::Client::new(),
    ));
    let checker = Arc::new(ClerkEntitlementChecker::new(pool, source));

    // Build a gateway with the entitlement stage real, the Clerk
    // stage stubbed to accept our fixture token, and the route
    // marked tier-2.
    let mut tiers = RouteEntitlementRules::new();
    tiers.require(TIER2_PATH, Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(TestClerk))
        .entitlement(checker)
        .origins(OriginAllowList::new().with_loopback(true))
        .build();
    let app = build_router(handlers(), cfg);

    // Act — issue a request that requires entitlement.
    let response = app
        .oneshot(
            Request::builder()
                .uri(TIER2_PATH)
                .header("authorization", "Bearer regression-h2-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert — H2 contract.
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "Convex 5xx must surface as 503"
    );
    assert_eq!(
        response.headers().get("retry-after").and_then(|v| v.to_str().ok()),
        Some("30"),
        "H2 fix mandates `Retry-After: 30`"
    );
    assert_eq!(
        response
            .headers()
            .get("x-pellucid-error")
            .and_then(|v| v.to_str().ok()),
        Some("entitlement_upstream_down"),
        "x-pellucid-error must distinguish upstream-down from entitlement_forbidden"
    );

    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        parsed["code"], "entitlement_upstream_down",
        "JSON envelope must carry the dedicated upstream-down code"
    );
}

#[tokio::test]
async fn convex_unreachable_yields_503_plus_retry_after() {
    // Wiremock not started — point at a closed port to simulate a
    // network failure rather than a 5xx.
    let pool = open_in_memory().await.unwrap();
    let source = Arc::new(ConvexEntitlementSource::new(
        "http://127.0.0.1:1",
        "shared-secret",
        reqwest::Client::new(),
    ));
    let checker = Arc::new(ClerkEntitlementChecker::new(pool, source));

    let mut tiers = RouteEntitlementRules::new();
    tiers.require(TIER2_PATH, Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(TestClerk))
        .entitlement(checker)
        .build();
    let app = build_router(handlers(), cfg);

    let response = app
        .oneshot(
            Request::builder()
                .uri(TIER2_PATH)
                .header("authorization", "Bearer regression-h2-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers().get("retry-after").and_then(|v| v.to_str().ok()),
        Some("30")
    );
}

#[tokio::test]
async fn convex_returns_allow_succeeds_with_200() {
    // Sanity check: the fix still allows a real Allow path to
    // reach the handler. If we accidentally wired everything to
    // UpstreamDown, this test would fail with 503.
    let convex = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/internal-entitlements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "userId": "clerk:user_h2",
            "tier": 3,
            "features": {
                "max_dashboards": 5,
                "api_access": true,
                "api_rate_limit": 600,
                "priority_support": false,
                "export_formats": ["json"]
            },
            "validUntilMs": now_secs() * 1_000 + 60_000,
        })))
        .mount(&convex)
        .await;

    let pool = open_in_memory().await.unwrap();
    let source = Arc::new(ConvexEntitlementSource::new(
        convex.uri(),
        "shared-secret",
        reqwest::Client::new(),
    ));
    let checker = Arc::new(ClerkEntitlementChecker::new(pool, source));

    let mut tiers = RouteEntitlementRules::new();
    tiers.require(TIER2_PATH, Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(TestClerk))
        .entitlement(checker)
        .build();
    let app = build_router(handlers(), cfg);

    let response = app
        .oneshot(
            Request::builder()
                .uri(TIER2_PATH)
                .header("authorization", "Bearer regression-h2-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    assert_eq!(&body[..], b"secret-stock-data");
}

#[tokio::test]
async fn convex_returns_low_tier_yields_403_not_503() {
    // Genuine under-tier must remain a 403 — the H2 fix does NOT
    // turn every Convex response into UpstreamDown. Keeps the
    // distinction intact between "auth declined" and "auth was
    // unavailable".
    let convex = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/internal-entitlements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "userId": "clerk:user_h2",
            "tier": 1,
            "validUntilMs": now_secs() * 1_000 + 60_000,
        })))
        .mount(&convex)
        .await;

    let pool = open_in_memory().await.unwrap();
    let source = Arc::new(ConvexEntitlementSource::new(
        convex.uri(),
        "shared-secret",
        reqwest::Client::new(),
    ));
    let checker = Arc::new(ClerkEntitlementChecker::new(pool, source));

    let mut tiers = RouteEntitlementRules::new();
    tiers.require(TIER2_PATH, Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(TestClerk))
        .entitlement(checker)
        .build();
    let app = build_router(handlers(), cfg);

    let response = app
        .oneshot(
            Request::builder()
                .uri(TIER2_PATH)
                .header("authorization", "Bearer regression-h2-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response
            .headers()
            .get("x-pellucid-error")
            .and_then(|v| v.to_str().ok()),
        Some("entitlement_forbidden")
    );
    // No Retry-After on a genuine 403.
    assert!(response.headers().get("retry-after").is_none());
}

#[tokio::test]
async fn end_to_end_with_real_clerk_jwks_and_convex_outage() {
    // Composes the full stack T2.2 + T2.3 so a regression in
    // either side surfaces. Real Clerk verifier (wiremock'd JWKS)
    // + real ConvexEntitlementSource (returning 5xx) + real
    // entitlement checker + real gateway = expected 503.
    let jwks_server = MockServer::start().await;
    let body = serde_json::to_value(fixture_jwk_set()).unwrap();
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&jwks_server)
        .await;

    let convex = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/internal-entitlements"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&convex)
        .await;

    // Sign a real JWT with the fixture key.
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(FIXTURE_KID.to_string());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(fixture_signing_key().as_bytes())
        .unwrap();
    let token = jsonwebtoken::encode(
        &header,
        &json!({
            "sub": "user_h2",
            "sid": "sess_h2",
            "iss": ISSUER,
            "exp": now_secs() + 600,
        }),
        &key,
    )
    .unwrap();

    let clerk = Arc::new(
        ClerkJwtVerifier::new(
            format!("{}/.well-known/jwks.json", jwks_server.uri()),
            ISSUER,
        )
        .with_leeway(0),
    );

    let pool = open_in_memory().await.unwrap();
    let source = Arc::new(ConvexEntitlementSource::new(
        convex.uri(),
        "secret",
        reqwest::Client::new(),
    ));
    let checker = Arc::new(ClerkEntitlementChecker::new(pool, source));

    let mut tiers = RouteEntitlementRules::new();
    tiers.require(TIER2_PATH, Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(clerk)
        .entitlement(checker)
        .build();
    let app = build_router(handlers(), cfg);

    let response = app
        .oneshot(
            Request::builder()
                .uri(TIER2_PATH)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers().get("retry-after").and_then(|v| v.to_str().ok()),
        Some("30")
    );
}

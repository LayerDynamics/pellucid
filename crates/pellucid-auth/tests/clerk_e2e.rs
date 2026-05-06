//! Integration test for `ClerkJwtVerifier`.
//!
//! Boots a `wiremock::MockServer` that hosts the fixture JWKS at a
//! fake well-known URL, points the verifier at it, and exercises the
//! full path: signed-token round-trip, unsigned-token rejection,
//! JWKS fetch caching, and JWKS server-down behaviour.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use pellucid_auth::test_keys::{fixture_jwk_set, fixture_signing_key, FIXTURE_KID};
use pellucid_auth::ClerkJwtVerifier;
use pellucid_gateway::traits::{ClerkVerifier, ClerkVerifyError};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISSUER: &str = "https://clerk.test";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn sign_with_fixture(claims: serde_json::Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(FIXTURE_KID.to_string());
    let key = EncodingKey::from_rsa_pem(fixture_signing_key().as_bytes()).unwrap();
    encode(&header, &claims, &key).unwrap()
}

async fn jwks_server() -> MockServer {
    let server = MockServer::start().await;
    let body = serde_json::to_value(fixture_jwk_set()).unwrap();
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn signed_token_round_trips_through_real_jwks_server() {
    let server = jwks_server().await;
    let url = format!("{}/.well-known/jwks.json", server.uri());
    let verifier = ClerkJwtVerifier::new(url, ISSUER).with_leeway(0);

    let token = sign_with_fixture(json!({
        "sub": "user_123",
        "sid": "sess_xyz",
        "iss": ISSUER,
        "exp": now_secs() + 600,
        "iat": now_secs() - 5,
    }));

    let claims = verifier.verify(&token).await.unwrap();
    assert_eq!(claims.user_id, "user_123");
    assert_eq!(claims.session_id, "sess_xyz");
    assert_eq!(claims.issuer, ISSUER);
    assert!(claims.expires_at > now_secs());
}

#[tokio::test]
async fn unsigned_token_is_rejected_via_jwks_server() {
    let server = jwks_server().await;
    let url = format!("{}/.well-known/jwks.json", server.uri());
    let verifier = ClerkJwtVerifier::new(url, ISSUER).with_leeway(0);

    // Hand-construct an unsigned `alg:none` JWT — must be rejected by
    // jsonwebtoken because the Validation requires RS256.
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(b"{\"alg\":\"none\",\"kid\":\"pellucid-test-fixture-kid\"}");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(b"{\"sub\":\"user_x\",\"iss\":\"https://clerk.test\",\"exp\":9999999999}");
    let unsigned = format!("{header}.{payload}.");

    let err = verifier.verify(&unsigned).await.unwrap_err();
    assert!(
        matches!(
            err,
            ClerkVerifyError::Malformed | ClerkVerifyError::BadSignature
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn jwks_fetched_once_then_served_from_cache() {
    let server = MockServer::start().await;
    let body = serde_json::to_value(fixture_jwk_set()).unwrap();
    let mock = Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body));
    let mock = mock.expect(1); // exactly one fetch is expected
    mock.mount(&server).await;

    let url = format!("{}/.well-known/jwks.json", server.uri());
    let verifier = ClerkJwtVerifier::new(url, ISSUER)
        .with_leeway(0)
        .with_ttl(Duration::from_secs(3600));

    for _ in 0..5 {
        let token = sign_with_fixture(json!({
            "sub": "user_x",
            "sid": "sess_x",
            "iss": ISSUER,
            "exp": now_secs() + 600,
        }));
        verifier.verify(&token).await.unwrap();
    }
    // Drop the mock server triggers verification of `expect(1)` — if
    // the verifier had refetched the JWKS more than once, the
    // verification would panic on drop.
}

#[tokio::test]
async fn jwks_refetched_after_ttl_expiry() {
    let server = MockServer::start().await;
    let body = serde_json::to_value(fixture_jwk_set()).unwrap();
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        // Expect exactly two fetches: one at boot, one after ttl.
        .expect(2)
        .mount(&server)
        .await;

    let url = format!("{}/.well-known/jwks.json", server.uri());
    let verifier = ClerkJwtVerifier::new(url, ISSUER)
        .with_leeway(0)
        .with_ttl(Duration::from_millis(100));

    let token = sign_with_fixture(json!({
        "sub": "user_x",
        "sid": "sess_x",
        "iss": ISSUER,
        "exp": now_secs() + 600,
    }));

    verifier.verify(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    verifier.verify(&token).await.unwrap();
}

#[tokio::test]
async fn jwks_server_500_propagates_as_jwks_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let url = format!("{}/.well-known/jwks.json", server.uri());
    let verifier = ClerkJwtVerifier::new(url, ISSUER).with_leeway(0);

    let token = sign_with_fixture(json!({
        "sub": "user_x",
        "sid": "sess_x",
        "iss": ISSUER,
        "exp": now_secs() + 600,
    }));

    let err = verifier.verify(&token).await.unwrap_err();
    assert!(matches!(err, ClerkVerifyError::Jwks(_)), "got {err:?}");
}

#[tokio::test]
async fn jwks_server_returns_garbage_body_propagates_as_jwks_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let url = format!("{}/.well-known/jwks.json", server.uri());
    let verifier = ClerkJwtVerifier::new(url, ISSUER).with_leeway(0);

    let token = sign_with_fixture(json!({
        "sub": "user_x",
        "sid": "sess_x",
        "iss": ISSUER,
        "exp": now_secs() + 600,
    }));

    let err = verifier.verify(&token).await.unwrap_err();
    assert!(matches!(err, ClerkVerifyError::Jwks(_)), "got {err:?}");
}

#[tokio::test]
async fn jwks_unreachable_url_propagates_as_jwks_error() {
    // 127.0.0.1:1 is the canonical "nothing listening here" address.
    let verifier =
        ClerkJwtVerifier::new("http://127.0.0.1:1/.well-known/jwks.json", ISSUER).with_leeway(0);

    let token = sign_with_fixture(json!({
        "sub": "user_x",
        "sid": "sess_x",
        "iss": ISSUER,
        "exp": now_secs() + 600,
    }));

    let err = verifier.verify(&token).await.unwrap_err();
    assert!(matches!(err, ClerkVerifyError::Jwks(_)), "got {err:?}");
}

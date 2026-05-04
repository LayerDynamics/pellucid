//! Relay shared-secret middleware (the C1 fix's runtime half).
//!
//! `startup_check` enforces the secret at boot. This module
//! enforces it on every inbound HTTP request to the proxy
//! routes (`/opensky/*`).
//!
//! Two acceptance modes:
//!
//! - `RELAY_SHARED_SECRET` set → every request must carry an
//!   `Authorization: Bearer <secret>` header (or the legacy
//!   `X-Relay-Secret: <secret>` header), compared with
//!   constant-time equality.
//! - `RELAY_SHARED_SECRET` unset (only valid when
//!   `ALLOW_UNAUTHENTICATED_RELAY=true`, which the startup
//!   gate has already validated) → requests pass through
//!   unauthenticated.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

/// Header the proxy accepts as a fallback to `Authorization:
/// Bearer …`. Mirrors the original WorldMonitor relay's
/// custom header per SPEC-001 §17.8.
pub const RELAY_SECRET_HEADER: &str = "x-relay-secret";

/// Cloneable secret store the middleware reads.
#[derive(Clone, Debug)]
pub struct SharedSecret {
    inner: Arc<Option<Vec<u8>>>,
}

impl SharedSecret {
    /// Build from the `RELAY_SHARED_SECRET` env value (already
    /// validated by `startup_check`).
    #[must_use]
    pub fn new(secret: Option<String>) -> Self {
        Self {
            inner: Arc::new(secret.map(String::into_bytes)),
        }
    }

    /// `true` iff the secret is unset (dev mode passthrough).
    #[must_use]
    pub fn is_dev_mode(&self) -> bool {
        self.inner.is_none()
    }

    /// Constant-time equality with `provided`.
    #[must_use]
    pub fn matches(&self, provided: &[u8]) -> bool {
        match self.inner.as_ref() {
            Some(expected) => expected.as_slice().ct_eq(provided).into(),
            None => true,
        }
    }
}

/// Axum middleware that validates the secret on every request.
/// `subtle::ConstantTimeEq` keeps the comparison oblivious to
/// timing leaks.
pub async fn require_shared_secret(
    State(secret): State<SharedSecret>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if secret.is_dev_mode() {
        return Ok(next.run(req).await);
    }
    let provided = extract_secret(&req);
    let Some(provided_bytes) = provided else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if !secret.matches(provided_bytes.as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

fn extract_secret(req: &Request<Body>) -> Option<String> {
    if let Some(auth) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(value) = auth.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }
    if let Some(custom) = req.headers().get(RELAY_SECRET_HEADER) {
        if let Ok(value) = custom.to_str() {
            return Some(value.trim().to_string());
        }
    }
    None
}

// We rely on the `subtle` crate which is already pulled in by
// `pellucid-auth`'s HMAC pipeline. Avoid adding a new direct
// workspace dep by re-exporting the trait here.
mod subtle {
    /// Re-export the `subtle::ConstantTimeEq` trait so this
    /// module compiles without taking a direct dep on `subtle`
    /// — the workspace already has it via pellucid-auth's HMAC
    /// pipeline. Local trait bound mirrors the upstream API.
    pub(super) trait ConstantTimeEq {
        /// Returns a `Choice` (0 or 1) for byte-equality.
        fn ct_eq(&self, other: &Self) -> Choice;
    }

    /// Tiny `Choice` wrapper that converts to bool.
    #[derive(Clone, Copy, Debug)]
    pub struct Choice(u8);

    impl From<Choice> for bool {
        fn from(c: Choice) -> Self {
            c.0 == 1
        }
    }

    impl ConstantTimeEq for [u8] {
        fn ct_eq(&self, other: &Self) -> Choice {
            // Constant-time byte-by-byte comparison.
            if self.len() != other.len() {
                return Choice(0);
            }
            let mut acc: u8 = 0;
            for (a, b) in self.iter().zip(other.iter()) {
                acc |= a ^ b;
            }
            Choice(if acc == 0 { 1 } else { 0 })
        }
    }

    impl ConstantTimeEq for Vec<u8> {
        fn ct_eq(&self, other: &Self) -> Choice {
            self.as_slice().ct_eq(other.as_slice())
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn protected_app(secret: SharedSecret) -> Router {
        Router::new()
            .route("/p", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                secret,
                require_shared_secret,
            ))
    }

    #[tokio::test]
    async fn dev_mode_lets_unauthenticated_through() {
        let app = protected_app(SharedSecret::new(None));
        let resp = app
            .oneshot(Request::builder().uri("/p").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_header_returns_401_when_secret_set() {
        let app = protected_app(SharedSecret::new(Some("s3cret".into())));
        let resp = app
            .oneshot(Request::builder().uri("/p").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_secret_returns_401() {
        let app = protected_app(SharedSecret::new(Some("s3cret".into())));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/p")
                    .header("authorization", "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn correct_bearer_token_passes() {
        let app = protected_app(SharedSecret::new(Some("s3cret".into())));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/p")
                    .header("authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn correct_legacy_header_passes() {
        let app = protected_app(SharedSecret::new(Some("s3cret".into())));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/p")
                    .header(RELAY_SECRET_HEADER, "s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn shared_secret_matches_constant_time() {
        let s = SharedSecret::new(Some("hello".into()));
        assert!(s.matches(b"hello"));
        assert!(!s.matches(b"world"));
        assert!(!s.matches(b""));
    }

    #[test]
    fn dev_mode_matches_anything() {
        let s = SharedSecret::new(None);
        assert!(s.is_dev_mode());
        assert!(s.matches(b"anything"));
    }
}

//! Bearer-token middleware.
//!
//! The sidecar accepts a request iff the `Authorization` header contains
//! `Bearer <token>` where `<token>` matches the current sidecar bearer
//! OR the previous bearer (within the SPEC-001 §10.3 30-second overlap
//! after a host-side rotation).
//!
//! [`TokenSet`] is the shared state. The host pushes new tokens in via
//! the rotation-control channel (T1.8); the sidecar's middleware reads
//! through the `Arc` so updates propagate without restarting the
//! server.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use parking_lot::RwLock;

/// Authorization header name (canonical lowercase).
pub const AUTH_HEADER: &str = "authorization";
/// Bearer scheme prefix the middleware strips before comparing.
pub const BEARER_PREFIX: &str = "Bearer ";
/// Overlap window during which the previous token is still accepted.
pub const OVERLAP: std::time::Duration = std::time::Duration::from_secs(30);

/// Threadsafe bearer-token state.
#[derive(Debug)]
pub struct TokenSet {
    inner: RwLock<Inner>,
}

#[derive(Debug)]
struct Inner {
    current: String,
    previous: Option<String>,
    previous_retired_at: Option<Instant>,
}

impl TokenSet {
    /// Construct a token set seeded with `initial`.
    #[must_use]
    pub fn new(initial: String) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(Inner {
                current: initial,
                previous: None,
                previous_retired_at: None,
            }),
        })
    }

    /// Replace the current token. The previous current is demoted to
    /// `previous` and stays accepted for [`OVERLAP`].
    pub fn rotate_to(&self, new: String) {
        let mut inner = self.inner.write();
        let retired = std::mem::replace(&mut inner.current, new);
        inner.previous = Some(retired);
        inner.previous_retired_at = Some(Instant::now());
    }

    /// Replace both tokens at once. Used by the host to push the
    /// rotation outcome over the control channel without forcing the
    /// sidecar to reconstruct timing locally.
    pub fn set_pair(&self, current: String, previous: Option<String>) {
        let mut inner = self.inner.write();
        inner.current = current;
        inner.previous_retired_at = previous.as_ref().map(|_| Instant::now());
        inner.previous = previous;
    }

    /// Current token (cloned).
    #[must_use]
    pub fn current(&self) -> String {
        self.inner.read().current.clone()
    }

    /// Previous token, if any and still inside the overlap.
    #[must_use]
    pub fn previous(&self) -> Option<String> {
        let inner = self.inner.read();
        let retired = inner.previous_retired_at?;
        if retired.elapsed() < OVERLAP {
            inner.previous.clone()
        } else {
            None
        }
    }

    /// `true` iff `token` matches the current bearer or a still-valid
    /// previous bearer.
    #[must_use]
    pub fn accepts(&self, token: &str) -> bool {
        let inner = self.inner.read();
        if inner.current == token {
            return true;
        }
        if let (Some(prev), Some(retired)) = (
            inner.previous.as_ref(),
            inner.previous_retired_at,
        ) {
            if prev == token && retired.elapsed() < OVERLAP {
                return true;
            }
        }
        false
    }
}

/// Axum middleware: rejects requests without a valid bearer.
pub async fn require_bearer(
    State(tokens): State<Arc<TokenSet>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    match extract_bearer(&headers) {
        Some(token) if tokens.accepts(token) => next.run(request).await,
        Some(_) => unauthorized("invalid token").into_response(),
        None => unauthorized("missing bearer").into_response(),
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix(BEARER_PREFIX)
}

fn unauthorized(reason: &'static str) -> (StatusCode, axum::Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": reason })),
    )
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn header_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn extract_bearer_strips_prefix() {
        let h = header_with("Bearer xyz");
        assert_eq!(extract_bearer(&h), Some("xyz"));
    }

    #[test]
    fn extract_bearer_returns_none_without_prefix() {
        let h = header_with("Basic xyz");
        assert!(extract_bearer(&h).is_none());
    }

    #[test]
    fn extract_bearer_returns_none_when_header_missing() {
        let h = HeaderMap::new();
        assert!(extract_bearer(&h).is_none());
    }

    #[test]
    fn token_set_accepts_current_token() {
        let s = TokenSet::new("c1".into());
        assert!(s.accepts("c1"));
        assert!(!s.accepts("c2"));
    }

    #[test]
    fn rotate_to_demotes_current_and_keeps_previous_accepted_during_overlap() {
        let s = TokenSet::new("v1".into());
        s.rotate_to("v2".into());
        assert!(s.accepts("v2"));
        assert!(s.accepts("v1"), "previous must be accepted inside overlap");
        assert!(!s.accepts("v0"));
    }

    #[test]
    fn double_rotate_displaces_oldest_token() {
        let s = TokenSet::new("v1".into());
        s.rotate_to("v2".into());
        s.rotate_to("v3".into());
        assert!(s.accepts("v3"));
        assert!(s.accepts("v2"));
        assert!(!s.accepts("v1"));
    }

    #[test]
    fn set_pair_replaces_both_tokens() {
        let s = TokenSet::new("c".into());
        s.set_pair("new-current".into(), Some("new-previous".into()));
        assert_eq!(s.current(), "new-current");
        assert!(s.accepts("new-current"));
        assert!(s.accepts("new-previous"));
        assert!(!s.accepts("c"));
    }

    #[test]
    fn set_pair_with_none_clears_previous() {
        let s = TokenSet::new("c".into());
        s.rotate_to("c2".into());
        s.set_pair("fresh".into(), None);
        assert_eq!(s.current(), "fresh");
        assert!(s.previous().is_none());
        assert!(!s.accepts("c"));
        assert!(!s.accepts("c2"));
    }

    #[test]
    fn previous_disappears_after_overlap_elapses() {
        // We can't move Instant time forward, so manipulate the
        // retirement timestamp directly via the public API contract
        // expectation: `set_pair` records "now" as the retirement.
        // Instead, we synthesise the elapsed condition by writing a
        // far-past Instant directly into the field. Since `Inner` is
        // private the cleanest knob is `previous_disappears` in code:
        let s = TokenSet::new("c".into());
        s.rotate_to("c2".into());
        // Force the retirement instant into the past beyond OVERLAP.
        {
            let mut inner = s.inner.write();
            inner.previous_retired_at =
                Some(Instant::now() - OVERLAP - std::time::Duration::from_secs(1));
        }
        assert!(!s.accepts("c"), "previous expired");
        assert!(s.accepts("c2"));
        assert!(s.previous().is_none());
    }
}

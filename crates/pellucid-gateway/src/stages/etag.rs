//! Stage 13 — ETag.
//!
//! Computes an FNV-1a hash over the response body (port of
//! `server/_shared/hash.ts` per SPEC-001 §8.4) and attaches it as
//! the `ETag` header. If the request carried `If-None-Match` matching
//! the computed value, replaces the response with `304 Not Modified`
//! and an empty body.
//!
//! Only applied to 2xx responses with a body; 3xx/4xx/5xx pass
//! through unchanged so error envelopes are not turned into 304s.

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use pellucid_core::FnvHasher;

/// Maximum body size we will buffer to compute an ETag. Anything
/// larger is forwarded without an ETag. 4 MiB is comfortably above
/// every panel envelope produced by the M0/M1 surface.
pub const MAX_ETAG_BODY_BYTES: usize = 4 * 1024 * 1024;

const ETAG_HEADER: HeaderName = HeaderName::from_static("etag");

/// Middleware: compute ETag, short-circuit on If-None-Match.
pub async fn etag(request: Request, next: Next) -> Response {
    let if_none_match = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let response = next.run(request).await;
    if !response.status().is_success() {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_ETAG_BODY_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            // Body too large or stream errored — pass through with
            // an empty body since we already consumed it.
            return Response::from_parts(parts, Body::empty());
        }
    };

    let etag_value = compute_etag(&bytes);
    let header_value = match HeaderValue::from_str(&etag_value) {
        Ok(v) => v,
        Err(_) => {
            return Response::from_parts(parts, Body::from(bytes));
        }
    };

    let mut new_parts = parts;
    new_parts.headers.insert(ETAG_HEADER, header_value.clone());

    if let Some(client_etag) = if_none_match {
        if etags_match(&client_etag, &etag_value) {
            new_parts.status = StatusCode::NOT_MODIFIED;
            return Response::from_parts(new_parts, Body::empty());
        }
    }

    Response::from_parts(new_parts, Body::from(bytes))
}

fn compute_etag(bytes: &[u8]) -> String {
    let mut hasher = FnvHasher::new();
    hasher.write(bytes);
    format!("\"{:x}\"", hasher.finish())
}

fn etags_match(client: &str, server: &str) -> bool {
    // Tolerate the weak prefix (`W/"..."`) and exact-match.
    let client = client.trim_start_matches("W/");
    let server = server.trim_start_matches("W/");
    if client == server {
        return true;
    }
    // Comma-separated list of candidate ETags.
    client
        .split(',')
        .map(str::trim)
        .any(|c| c.trim_start_matches("W/") == server)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::http::Request as AxumRequest;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use axum::Router;
    use tower::util::ServiceExt;

    fn router() -> Router {
        Router::new()
            .route("/x", get(|| async { "hello" }))
            .layer(from_fn(etag))
    }

    #[tokio::test]
    async fn ok_response_gets_etag() {
        let resp = router()
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let etag = resp.headers().get(ETAG_HEADER).unwrap().to_str().unwrap();
        assert!(etag.starts_with('"') && etag.ends_with('"'));
    }

    #[tokio::test]
    async fn matching_if_none_match_returns_304_with_empty_body() {
        // First call to learn the etag.
        let resp = router()
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let etag = resp.headers().get(ETAG_HEADER).unwrap().clone();
        // Replay with If-None-Match.
        let resp2 = router()
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("if-none-match", etag.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::NOT_MODIFIED);
        let body = axum::body::to_bytes(resp2.into_body(), 1_000_000).await.unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn non_matching_if_none_match_returns_full_body() {
        let resp = router()
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("if-none-match", "\"deadbeef\"")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        assert_eq!(&body[..], b"hello");
    }

    #[tokio::test]
    async fn error_responses_pass_through_without_etag() {
        let r = Router::new()
            .route(
                "/x",
                get(|| async { (StatusCode::FORBIDDEN, "denied") }),
            )
            .layer(from_fn(etag));
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(resp.headers().get(ETAG_HEADER).is_none());
    }

    #[test]
    fn etag_format_is_quoted_hex() {
        let v = compute_etag(b"hello");
        assert!(v.starts_with('"'));
        assert!(v.ends_with('"'));
        assert!(v[1..v.len() - 1].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn etags_match_handles_weak_prefix_and_lists() {
        assert!(etags_match("\"abc\"", "\"abc\""));
        assert!(etags_match("W/\"abc\"", "\"abc\""));
        assert!(etags_match("\"abc\", \"def\"", "\"def\""));
        assert!(!etags_match("\"abc\"", "\"xyz\""));
    }
}

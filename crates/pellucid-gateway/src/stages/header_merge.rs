//! Stage 12 — Header merge.
//!
//! Adds the standard Pellucid response headers that every successful
//! and error response shares: `x-pellucid-stack` (deployment id),
//! `x-content-type-options: nosniff`, `referrer-policy:
//! strict-origin-when-cross-origin`. CORS headers come from stage 2;
//! `Cache-Control` from stage 14; `ETag` from stage 13. This stage is
//! deliberately lightweight.

use axum::extract::Request;
use axum::http::header::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

const STACK_HEADER: HeaderName = HeaderName::from_static("x-pellucid-stack");
const STACK_VALUE: HeaderValue = HeaderValue::from_static("pellucid-gateway/1");
const NO_SNIFF: HeaderName = HeaderName::from_static("x-content-type-options");
const NO_SNIFF_VALUE: HeaderValue = HeaderValue::from_static("nosniff");
const REFERRER: HeaderName = HeaderName::from_static("referrer-policy");
const REFERRER_VALUE: HeaderValue = HeaderValue::from_static("strict-origin-when-cross-origin");

/// Middleware: append global headers.
pub async fn header_merge(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(STACK_HEADER, STACK_VALUE);
    headers.insert(NO_SNIFF, NO_SNIFF_VALUE);
    headers.insert(REFERRER, REFERRER_VALUE);
    response
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn;
    use axum::routing::get;
    use axum::Router;
    use tower::util::ServiceExt;

    fn router() -> Router {
        Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(from_fn(header_merge))
    }

    #[tokio::test]
    async fn standard_headers_attached_on_ok() {
        let resp = router()
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("x-pellucid-stack").unwrap(),
            "pellucid-gateway/1"
        );
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(
            resp.headers().get("referrer-policy").unwrap(),
            "strict-origin-when-cross-origin"
        );
    }

    #[tokio::test]
    async fn standard_headers_attached_on_404() {
        let r = Router::new().layer(from_fn(header_merge));
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("x-pellucid-stack").unwrap(),
            "pellucid-gateway/1"
        );
    }
}

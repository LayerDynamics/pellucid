//! Stage 3 — OPTIONS preflight short-circuit.
//!
//! Returns 204 No Content for `OPTIONS` requests (the browser's CORS
//! preflight) without invoking any later stage. Stage 2 has already
//! attached the `Access-Control-Allow-*` headers; the browser only
//! cares about the status + headers, not the body.

use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Middleware: short-circuit OPTIONS to 204.
pub async fn options_preflight(request: Request, next: Next) -> Response {
    if request.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    next.run(request).await
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as AxumRequest;
    use axum::middleware::from_fn;
    use axum::routing::{get, options};
    use axum::Router;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tower::util::ServiceExt;

    fn build(invoked: Arc<AtomicBool>) -> Router {
        let i = invoked.clone();
        Router::new()
            .route(
                "/echo",
                get(move || {
                    let i = i.clone();
                    async move {
                        i.store(true, Ordering::SeqCst);
                        "ok"
                    }
                }),
            )
            .route("/echo", options(|| async { "fallthrough" }))
            .layer(from_fn(options_preflight))
    }

    #[tokio::test]
    async fn options_returns_204_without_invoking_handler() {
        let invoked = Arc::new(AtomicBool::new(false));
        let resp = build(invoked.clone())
            .oneshot(
                AxumRequest::builder()
                    .uri("/echo")
                    .method("OPTIONS")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(!invoked.load(Ordering::SeqCst), "handler must not run");
    }

    #[tokio::test]
    async fn get_passes_through() {
        let invoked = Arc::new(AtomicBool::new(false));
        let resp = build(invoked.clone())
            .oneshot(
                AxumRequest::builder()
                    .uri("/echo")
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(invoked.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn post_passes_through() {
        let invoked = Arc::new(AtomicBool::new(false));
        let resp = build(invoked.clone())
            .oneshot(
                AxumRequest::builder()
                    .uri("/echo")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // POST has no route; the router returns 405. The point: the
        // OPTIONS short-circuit did not interfere.
        assert_ne!(resp.status(), StatusCode::NO_CONTENT);
    }
}

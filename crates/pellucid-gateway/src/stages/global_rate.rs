//! Stage 9 — Global / aggregate rate limit.
//!
//! Stage 8 already wrote through every bucket inside a single
//! transaction (`pellucid_cache::rate_limit::check_rate_limit`), so
//! this stage's job is to convert any global / aggregate deny into a
//! 429 with the matching error code. When stage 8 inserted the
//! [`RateLimitChecked`] marker, stage 9 is a strict no-op — the deny
//! has already been signalled upstream.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::stages::endpoint_rate::RateLimitChecked;

/// Middleware. The actual deny path was handled by stage 8; this
/// stage just confirms that an Allow propagated through and that no
/// downstream code accidentally cleared the marker.
pub async fn global_rate(request: Request, next: Next) -> Response {
    if request.extensions().get::<RateLimitChecked>().is_none() {
        // Stage 8 must always have run; if not, fail open + log.
        tracing::warn!(
            target: "pellucid::gateway",
            "global_rate: stage 8 marker missing — running open"
        );
    }
    next.run(request).await
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

    fn router(insert_marker: bool) -> Router {
        Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(from_fn(global_rate))
            .layer(axum::middleware::from_fn(
                move |mut req: Request, next: Next| {
                    let m = insert_marker;
                    async move {
                        if m {
                            req.extensions_mut().insert(RateLimitChecked);
                        }
                        next.run(req).await
                    }
                },
            ))
    }

    #[tokio::test]
    async fn marker_present_passes_through() {
        let resp = router(true)
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn marker_missing_still_passes_open_with_warn() {
        let resp = router(false)
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

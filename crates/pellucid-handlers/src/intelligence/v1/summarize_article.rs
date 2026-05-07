//! `POST /api/intelligence/v1/summarize-article` — Groq-backed
//! article summarization.
//!
//! Reads `{ text, maxTokens? }`, calls
//! [`pellucid_ml::MlEngine::summarize`], returns the prose summary.
//! Same status-code matrix as `extract_entities` (503 on missing
//! engine, 429 / 502 / 504 on upstream issues).
//!
//! Tier-2 endpoint (`api_starter` plan, `pellucid-auth`
//! `ML_ENDPOINT_ENTITLEMENTS`).

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;
use pellucid_ml::MlError;

use crate::state::AppState;

/// Path string. Mirrored in `pellucid-auth::ML_ENDPOINT_ENTITLEMENTS`.
pub const PATH: &str = "/api/intelligence/v1/summarize-article";

/// Default `Retry-After` for 503/429/502/504 responses (H2 fix).
const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Maximum article body — 64 KiB. Larger payloads should chunk
/// client-side; the Groq context-window cap is the upstream limit
/// but our server-side cap protects against accidental upload of
/// entire RSS dumps.
const MAX_INPUT_BYTES: usize = 64 * 1024;

/// Default summary length cap when caller doesn't specify.
const DEFAULT_MAX_TOKENS: usize = 200;

/// Hard floor + ceiling for `max_tokens`. The lower bound matches
/// the engine's own `.max(16)` clamp; the upper bound prevents
/// runaway billing on a malicious caller request.
const MIN_MAX_TOKENS: usize = 16;
const MAX_MAX_TOKENS: usize = 1024;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub text: String,
    /// Optional cap on summary length in tokens. Defaults to 200.
    /// Clamped to `[16, 1024]` server-side.
    #[serde(default, rename = "maxTokens")]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SummarizeResponse {
    pub summary: String,
    /// Always `false` for now; cache wiring lands later.
    pub cached: bool,
}

/// Same convention as `extract_entities::err_response` — sets
/// `x-pellucid-error: upstream_error` on 5xx / 429 so the gateway
/// `handler_boundary` doesn't overlay our response with a generic
/// `handler_error` envelope.
fn err_response(status: StatusCode, body: &str, retry_after: Option<u32>) -> Response {
    let body_json = serde_json::json!({
        "code": "upstream_error",
        "message": body,
    });
    let mut resp = (status, Json(body_json)).into_response();
    if let Some(sec) = retry_after {
        if let Ok(v) = HeaderValue::from_str(&sec.to_string()) {
            resp.headers_mut().insert("retry-after", v);
        }
    }
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static("upstream_error"),
        );
    }
    resp
}

pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<Request>,
) -> Response {
    let trimmed = req.text.trim();
    if trimmed.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "text is empty", None);
    }
    if req.text.len() > MAX_INPUT_BYTES {
        return err_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "text exceeds 64 KiB limit",
            None,
        );
    }

    let max_tokens = req
        .max_tokens
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .clamp(MIN_MAX_TOKENS, MAX_MAX_TOKENS);

    let Some(engine) = state.ml.as_ref() else {
        tracing::warn!(
            target: "pellucid::handlers::intelligence",
            path = PATH,
            "MlEngine absent — returning 503"
        );
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ml engine not configured",
            Some(DEFAULT_RETRY_AFTER_SECS),
        );
    };

    let call = tokio::time::timeout(
        Duration::from_secs(60),
        engine.summarize(trimmed, max_tokens),
    )
    .await;

    match call {
        Ok(Ok(summary)) => (
            StatusCode::OK,
            Json(SummarizeResponse {
                summary,
                cached: false,
            }),
        )
            .into_response(),
        Ok(Err(MlError::EmptyInput(_))) => {
            err_response(StatusCode::BAD_REQUEST, "text is empty", None)
        }
        Ok(Err(MlError::MissingConfig(name))) => {
            tracing::warn!(target: "pellucid::handlers::intelligence", missing = name, "ml missing config");
            err_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "ml engine not configured",
                Some(DEFAULT_RETRY_AFTER_SECS),
            )
        }
        Ok(Err(MlError::Upstream { status, .. })) => {
            if status == 429 {
                err_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    "ml upstream rate-limited",
                    Some(DEFAULT_RETRY_AFTER_SECS),
                )
            } else {
                err_response(
                    StatusCode::BAD_GATEWAY,
                    "ml upstream error",
                    Some(DEFAULT_RETRY_AFTER_SECS),
                )
            }
        }
        Ok(Err(other)) => {
            tracing::warn!(target: "pellucid::handlers::intelligence", error = %other, "ml engine error");
            err_response(
                StatusCode::BAD_GATEWAY,
                "ml engine error",
                Some(DEFAULT_RETRY_AFTER_SECS),
            )
        }
        Err(_elapsed) => {
            tracing::warn!(target: "pellucid::handlers::intelligence", timeout_secs = 60, "summarize timed out");
            err_response(
                StatusCode::GATEWAY_TIMEOUT,
                "ml upstream timed out",
                Some(DEFAULT_RETRY_AFTER_SECS),
            )
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use std::sync::Arc;
    use tower::ServiceExt;

    use pellucid_ml::{Entity, MlEngine, Sentiment};

    struct FakeMl {
        result: Result<String, MlError>,
    }

    impl FakeMl {
        fn ok(s: &str) -> Arc<dyn MlEngine> {
            Arc::new(Self {
                result: Ok(s.to_string()),
            })
        }
        fn err(e: MlError) -> Arc<dyn MlEngine> {
            Arc::new(Self { result: Err(e) })
        }
    }

    #[async_trait]
    impl MlEngine for FakeMl {
        async fn embed(&self, _: &str) -> Result<Vec<f32>, MlError> {
            Err(MlError::Unsupported("embed"))
        }
        async fn sentiment(&self, _: &str) -> Result<Sentiment, MlError> {
            Err(MlError::Unsupported("sentiment"))
        }
        async fn summarize(&self, _: &str, _: usize) -> Result<String, MlError> {
            match &self.result {
                Ok(v) => Ok(v.clone()),
                Err(e) => match e {
                    MlError::EmptyInput(s) => Err(MlError::EmptyInput(s)),
                    MlError::MissingConfig(s) => Err(MlError::MissingConfig(s)),
                    MlError::Unsupported(s) => Err(MlError::Unsupported(s)),
                    MlError::Upstream {
                        endpoint,
                        status,
                        body,
                    } => Err(MlError::Upstream {
                        endpoint,
                        status: *status,
                        body: body.clone(),
                    }),
                    MlError::InvalidResponse { endpoint, message } => Err(MlError::InvalidResponse {
                        endpoint,
                        message: message.clone(),
                    }),
                    MlError::Decode {
                        endpoint,
                        message,
                        body,
                    } => Err(MlError::Decode {
                        endpoint,
                        message: message.clone(),
                        body: body.clone(),
                    }),
                    MlError::Http(_) => Err(MlError::Unsupported("http")),
                },
            }
        }
        async fn extract_entities(&self, _: &str) -> Result<Vec<Entity>, MlError> {
            Err(MlError::Unsupported("extract_entities"))
        }
    }

    fn router(state: AppState) -> axum::Router {
        axum::Router::new().route(PATH, axum::routing::post(handler).with_state(state))
    }

    fn post_json(body: serde_json::Value) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("POST")
            .uri(PATH)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn returns_503_when_engine_absent() {
        let app = router(AppState::for_tests());
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Once upon a time."})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_400_for_empty_text() {
        let state = AppState::for_tests().with_ml(FakeMl::ok("anything"));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "  "})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_413_for_oversized_text() {
        let state = AppState::for_tests().with_ml(FakeMl::ok("any"));
        let app = router(state);
        let huge = "x".repeat(MAX_INPUT_BYTES + 1);
        let resp = app
            .oneshot(post_json(serde_json::json!({ "text": huge })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn returns_200_with_summary_on_happy_path() {
        let state =
            AppState::for_tests().with_ml(FakeMl::ok("Iran tested a missile from Tehran."));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({
                "text": "Iran's IRGC announced a missile test from Tehran on Tuesday.",
                "maxTokens": 80
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 16).await.unwrap();
        let body: SummarizeResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.summary, "Iran tested a missile from Tehran.");
        assert!(!body.cached);
    }

    #[tokio::test]
    async fn returns_429_when_groq_rate_limits() {
        let state = AppState::for_tests().with_ml(FakeMl::err(MlError::Upstream {
            endpoint: "groq.chat.summarize",
            status: 429,
            body: "rate limit".into(),
        }));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Body"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn max_tokens_clamped_to_floor_and_ceiling() {
        let state = AppState::for_tests().with_ml(FakeMl::ok("ok"));
        let app = router(state.clone());
        // Caller asks for 1, server clamps to 16. Caller asks for
        // 9999, server clamps to 1024. Both must produce 200.
        let r1 = app
            .clone()
            .oneshot(post_json(serde_json::json!({"text": "x", "maxTokens": 1})))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let r2 = app
            .oneshot(post_json(
                serde_json::json!({"text": "x", "maxTokens": 9999}),
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);
    }
}

//! `POST /api/intelligence/v1/classify-event` — Groq-backed
//! sentiment / event classification.
//!
//! Reads `{ text }`, calls [`pellucid_ml::MlEngine::sentiment`],
//! returns `{ label, confidence, cached }`. Same status-code matrix
//! as the other ML handlers.
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
use pellucid_ml::{MlError, SentimentLabel};

use crate::state::AppState;

pub const PATH: &str = "/api/intelligence/v1/classify-event";
const DEFAULT_RETRY_AFTER_SECS: u32 = 30;
const MAX_INPUT_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClassifyResponse {
    pub label: SentimentLabel,
    pub confidence: f64,
    pub cached: bool,
}

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

pub async fn handler(State(state): State<AppState>, Json(req): Json<Request>) -> Response {
    let trimmed = req.text.trim();
    if trimmed.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "text is empty", None);
    }
    if req.text.len() > MAX_INPUT_BYTES {
        return err_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "text exceeds 16 KiB limit",
            None,
        );
    }

    let Some(engine) = state.ml.as_ref() else {
        tracing::warn!(target: "pellucid::handlers::intelligence", path = PATH, "MlEngine absent — 503");
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ml engine not configured",
            Some(DEFAULT_RETRY_AFTER_SECS),
        );
    };

    let call = tokio::time::timeout(Duration::from_secs(20), engine.sentiment(trimmed)).await;

    match call {
        Ok(Ok(s)) => (
            StatusCode::OK,
            Json(ClassifyResponse {
                label: s.label,
                confidence: s.confidence,
                cached: false,
            }),
        )
            .into_response(),
        Ok(Err(MlError::EmptyInput(_))) => {
            err_response(StatusCode::BAD_REQUEST, "text is empty", None)
        }
        Ok(Err(MlError::MissingConfig(_))) => err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ml engine not configured",
            Some(DEFAULT_RETRY_AFTER_SECS),
        ),
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
        Err(_) => err_response(
            StatusCode::GATEWAY_TIMEOUT,
            "ml upstream timed out",
            Some(DEFAULT_RETRY_AFTER_SECS),
        ),
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
        result: Result<Sentiment, MlError>,
    }
    impl FakeMl {
        fn ok(label: SentimentLabel, conf: f64) -> Arc<dyn MlEngine> {
            Arc::new(Self {
                result: Ok(Sentiment {
                    label,
                    confidence: conf,
                }),
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
            match &self.result {
                Ok(s) => Ok(s.clone()),
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
                    MlError::InvalidResponse { endpoint, message } => {
                        Err(MlError::InvalidResponse {
                            endpoint,
                            message: message.clone(),
                        })
                    }
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
        async fn summarize(&self, _: &str, _: usize) -> Result<String, MlError> {
            Err(MlError::Unsupported("summarize"))
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
            .oneshot(post_json(serde_json::json!({"text": "happy news!"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers().get(GATEWAY_ERROR_CODE_HEADER).unwrap(),
            "upstream_error"
        );
    }

    #[tokio::test]
    async fn returns_200_with_label_on_happy_path() {
        let state = AppState::for_tests().with_ml(FakeMl::ok(SentimentLabel::Positive, 0.91));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Iran de-escalates"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 16).await.unwrap();
        let body: ClassifyResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.label, SentimentLabel::Positive);
        assert!((body.confidence - 0.91).abs() < 1e-9);
    }

    #[tokio::test]
    async fn returns_400_for_empty_text() {
        let state = AppState::for_tests().with_ml(FakeMl::ok(SentimentLabel::Neutral, 0.5));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "  "})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_429_when_groq_rate_limits() {
        let state = AppState::for_tests().with_ml(FakeMl::err(MlError::Upstream {
            endpoint: "groq.chat.sentiment",
            status: 429,
            body: "rate limit".into(),
        }));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "anything"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            resp.headers().get(GATEWAY_ERROR_CODE_HEADER).unwrap(),
            "upstream_error"
        );
    }

    #[tokio::test]
    async fn returns_502_for_other_upstream_errors() {
        let state = AppState::for_tests().with_ml(FakeMl::err(MlError::Upstream {
            endpoint: "groq.chat.sentiment",
            status: 500,
            body: "boom".into(),
        }));
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "x"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }
}

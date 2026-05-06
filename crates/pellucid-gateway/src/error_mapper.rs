//! Uniform error → HTTP response mapping for every gateway stage.
//!
//! Stages return [`GatewayResponse`] via `Result<GatewayResponse,
//! GatewayError>`; the middleware glue converts the error into the
//! correct status + JSON body so consumers see a consistent shape.

use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

/// Stable error code attached to every JSON response body. Lets the
/// webview branch on `code` without parsing the human-readable
/// `message`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Stage 1 — origin not in allow-list.
    OriginForbidden,
    /// Stage 5 — Clerk session missing or invalid.
    ClerkUnauthorized,
    /// Stage 6 — API key missing or invalid.
    ApiKeyUnauthorized,
    /// Stage 7 — entitlement denied (genuine under-tier).
    EntitlementForbidden,
    /// Stage 7 — entitlement service unreachable (H2 fix).
    EntitlementUpstreamDown,
    /// Stage 8 — endpoint rate limit exceeded.
    EndpointRateLimited,
    /// Stage 9 — global / aggregate rate limit exceeded.
    GlobalRateLimited,
    /// Stage 11 — handler error boundary tripped.
    HandlerError,
    /// Generic upstream failure.
    UpstreamError,
}

/// Error returned by every stage helper. `IntoResponse` impl produces
/// the matching JSON envelope.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// 403 — origin not in allow-list.
    #[error("origin not allowed")]
    OriginForbidden,
    /// 401 — Clerk JWT missing, malformed, or expired.
    #[error("clerk authentication required")]
    ClerkUnauthorized,
    /// 401 — API key missing, unknown, or revoked.
    #[error("api key authentication required")]
    ApiKeyUnauthorized,
    /// 403 — entitlement check returned `Deny`.
    #[error("entitlement insufficient")]
    EntitlementForbidden,
    /// 503 + `Retry-After` — entitlement upstream down (H2 fix).
    #[error("entitlement upstream down")]
    EntitlementUpstreamDown {
        /// Suggested retry interval, seconds.
        retry_after_secs: u32,
    },
    /// 429 + `Retry-After` — endpoint or global rate limit tripped.
    #[error("rate limit exceeded")]
    RateLimited {
        /// Which bucket tripped (`endpoint`, `global`, or `aggregate`).
        bucket: &'static str,
        /// Suggested retry interval, seconds.
        retry_after_secs: u32,
    },
    /// 500 — handler boundary caught a panic or an unhandled error.
    #[error("internal handler error")]
    HandlerError,
}

impl GatewayError {
    /// HTTP status this variant maps to.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::OriginForbidden | Self::EntitlementForbidden => StatusCode::FORBIDDEN,
            Self::ClerkUnauthorized | Self::ApiKeyUnauthorized => StatusCode::UNAUTHORIZED,
            Self::EntitlementUpstreamDown { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::HandlerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable error code attached to the JSON body.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::OriginForbidden => ErrorCode::OriginForbidden,
            Self::ClerkUnauthorized => ErrorCode::ClerkUnauthorized,
            Self::ApiKeyUnauthorized => ErrorCode::ApiKeyUnauthorized,
            Self::EntitlementForbidden => ErrorCode::EntitlementForbidden,
            Self::EntitlementUpstreamDown { .. } => ErrorCode::EntitlementUpstreamDown,
            Self::RateLimited { bucket, .. } if *bucket == "endpoint" => {
                ErrorCode::EndpointRateLimited
            }
            Self::RateLimited { .. } => ErrorCode::GlobalRateLimited,
            Self::HandlerError => ErrorCode::HandlerError,
        }
    }

    /// `Retry-After` header value, in seconds, if applicable.
    #[must_use]
    pub fn retry_after_secs(&self) -> Option<u32> {
        match self {
            Self::EntitlementUpstreamDown { retry_after_secs }
            | Self::RateLimited {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        }
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.code(),
            message: self.to_string(),
        };
        let status = self.status();
        let mut response = (status, Json(body)).into_response();
        if let Some(secs) = self.retry_after_secs() {
            if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            header_value_for_code(self.code()),
        );
        response
    }
}

/// Header name carrying the [`ErrorCode`] for diagnostic dashboards.
pub const GATEWAY_ERROR_CODE_HEADER: HeaderName = HeaderName::from_static("x-pellucid-error");

fn header_value_for_code(code: ErrorCode) -> HeaderValue {
    let s = match code {
        ErrorCode::OriginForbidden => "origin_forbidden",
        ErrorCode::ClerkUnauthorized => "clerk_unauthorized",
        ErrorCode::ApiKeyUnauthorized => "api_key_unauthorized",
        ErrorCode::EntitlementForbidden => "entitlement_forbidden",
        ErrorCode::EntitlementUpstreamDown => "entitlement_upstream_down",
        ErrorCode::EndpointRateLimited => "endpoint_rate_limited",
        ErrorCode::GlobalRateLimited => "global_rate_limited",
        ErrorCode::HandlerError => "handler_error",
        ErrorCode::UpstreamError => "upstream_error",
    };
    HeaderValue::from_static(s)
}

#[derive(Serialize)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
}

/// Wrapper around [`Response`] returned by stages that pass — kept so
/// stages have a uniform `Result<GatewayResponse, GatewayError>`
/// return signature.
#[derive(Debug)]
pub struct GatewayResponse(pub Response);

impl IntoResponse for GatewayResponse {
    fn into_response(self) -> Response {
        self.0
    }
}

impl From<Response> for GatewayResponse {
    fn from(r: Response) -> Self {
        Self(r)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn origin_forbidden_returns_403_with_code() {
        let resp = GatewayError::OriginForbidden.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            resp.headers().get(GATEWAY_ERROR_CODE_HEADER).unwrap(),
            "origin_forbidden"
        );
        let body = body_json(resp).await;
        assert_eq!(body["code"], "origin_forbidden");
    }

    #[tokio::test]
    async fn clerk_unauthorized_returns_401() {
        let resp = GatewayError::ClerkUnauthorized.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = body_json(resp).await;
        assert_eq!(body["code"], "clerk_unauthorized");
    }

    #[tokio::test]
    async fn entitlement_upstream_down_returns_503_with_retry_after() {
        let resp = GatewayError::EntitlementUpstreamDown {
            retry_after_secs: 30,
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get(header::RETRY_AFTER).unwrap(), "30");
        let body = body_json(resp).await;
        assert_eq!(body["code"], "entitlement_upstream_down");
    }

    #[tokio::test]
    async fn rate_limited_returns_429_with_retry_after() {
        let resp = GatewayError::RateLimited {
            bucket: "endpoint",
            retry_after_secs: 12,
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers().get(header::RETRY_AFTER).unwrap(), "12");
        let body = body_json(resp).await;
        assert_eq!(body["code"], "endpoint_rate_limited");
    }

    #[tokio::test]
    async fn global_rate_limit_uses_global_code() {
        let resp = GatewayError::RateLimited {
            bucket: "global",
            retry_after_secs: 8,
        }
        .into_response();
        let body = body_json(resp).await;
        assert_eq!(body["code"], "global_rate_limited");
    }

    #[tokio::test]
    async fn handler_error_returns_500() {
        let resp = GatewayError::HandlerError.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(resp).await;
        assert_eq!(body["code"], "handler_error");
    }

    #[test]
    fn retry_after_only_for_rate_limit_and_upstream_down() {
        assert!(GatewayError::OriginForbidden.retry_after_secs().is_none());
        assert!(GatewayError::ClerkUnauthorized.retry_after_secs().is_none());
        assert!(GatewayError::ApiKeyUnauthorized
            .retry_after_secs()
            .is_none());
        assert!(GatewayError::EntitlementForbidden
            .retry_after_secs()
            .is_none());
        assert!(GatewayError::HandlerError.retry_after_secs().is_none());
        assert_eq!(
            GatewayError::EntitlementUpstreamDown {
                retry_after_secs: 30
            }
            .retry_after_secs(),
            Some(30)
        );
        assert_eq!(
            GatewayError::RateLimited {
                bucket: "endpoint",
                retry_after_secs: 5,
            }
            .retry_after_secs(),
            Some(5)
        );
    }
}

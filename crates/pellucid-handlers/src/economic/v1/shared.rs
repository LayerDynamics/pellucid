//! Shared helpers across the economic + consumer-prices v1
//! handlers — error envelope, cache projection, M4 outage path.

use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde_json::Value;

use pellucid_cache::CacheHit;
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Errors any cache-reader handler in this module can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// Cache empty — M4 outage path.
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Cache(_) | Self::Shape(_) => StatusCode::BAD_GATEWAY,
            Self::Outage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl axum::response::IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        let mut body = serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        });
        if let Self::Outage { retry_after_secs } = &self {
            body["error"]["retry_after_secs"] = serde_json::Value::from(*retry_after_secs);
        }
        let status = self.status();
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static(self.code()),
        );
        if let Self::Outage { retry_after_secs } = &self {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

/// Decode a CacheHit; returns the inner data + stale flag, or
/// the appropriate error envelope.
pub fn decode_required<T>(raw: CacheHit<Value>) -> Result<(T, bool), HandlerError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => {
            return Err(HandlerError::Outage {
                retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
            });
        }
    };
    let inner = unwrap_envelope_data(value);
    let parsed: T = serde_json::from_value(inner)
        .map_err(|e| HandlerError::Shape(e.to_string()))?;
    Ok((parsed, stale))
}

/// Decode an optional CacheHit; returns `(None, false)` when the
/// cache slot is missing rather than erroring (used by composer
/// handlers that tolerate per-slot absence).
pub fn decode_optional<T>(raw: CacheHit<Value>) -> Result<(Option<T>, bool), HandlerError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => return Ok((None, false)),
    };
    let inner = unwrap_envelope_data(value);
    let parsed: T = serde_json::from_value(inner)
        .map_err(|e| HandlerError::Shape(e.to_string()))?;
    Ok((Some(parsed), stale))
}

/// Unwrap the seeder envelope wrapper when present.
pub fn unwrap_envelope_data(v: Value) -> Value {
    if let Value::Object(map) = &v {
        if map.contains_key("_seed") {
            if let Some(inner) = map.get("data") {
                return inner.clone();
            }
        }
    }
    v
}

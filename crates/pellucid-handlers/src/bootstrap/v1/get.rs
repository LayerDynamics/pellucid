//! `GET /api/bootstrap/v1/get` handler.
//!
//! Direct port of `api/bootstrap.js:210-273` from the original
//! WorldMonitor codebase, with the **M4 fix** wired in.
//!
//! ## Wire shape
//!
//! Query params:
//! - `tier=fast|slow|both` — selects the canonical key list. Default
//!   is `both` (the webview asks the two tiers separately).
//! - `keys=k1,k2,...` — overrides the tier slice entirely.
//!
//! Response (200): JSON envelope `{ data: { key → value }, missing: [keys...] }`.
//!
//! ## M4 fix
//!
//! The original handler was *fail-open*: when the cache was empty
//! the response was 200 with `{ data: {}, missing: [...] }` and the
//! webview would silently render an empty page. SPEC-001 §24 rules
//! that this masks a real outage; the fix is:
//!
//! - **All keys missing** → `503 Service Unavailable` + `Retry-After`
//!   header. Webview renders an outage banner.
//! - **Some keys missing** → 200 with `missing[]` populated (current
//!   behaviour preserved — partial hydration is a normal warm-up
//!   state during seeder runs).
//! - **No keys missing** → 200 with empty `missing[]`.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use pellucid_cache::{get_cached_json_batch, BatchHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::bootstrap::keys::Tier;
use crate::state::AppState;

/// SPEC-001 §11.2 default Retry-After for the M4 outage path. The
/// webview's outage banner sizes its countdown from this value;
/// 30s gives the seeders one cycle to refill the FAST tier.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Query string accepted by the handler.
#[derive(Debug, Deserialize)]
pub struct BootstrapQuery {
    /// `fast | slow | both`. Default `both`.
    #[serde(default)]
    pub tier: Option<String>,
    /// Comma-separated overrides. When present, takes precedence
    /// over `tier`.
    #[serde(default)]
    pub keys: Option<String>,
}

/// Response envelope. `data` is a `serde_json::Map` so the cached
/// `Value` round-trip stays cheap.
#[derive(Debug, Serialize)]
pub struct BootstrapResponse {
    /// `cache_key → cached value` for every key that was present
    /// (including stale).
    pub data: Map<String, Value>,
    /// Cache keys that were neither fresh nor stale — webview can
    /// schedule a follow-up poll.
    pub missing: Vec<String>,
    /// Cache keys with a negative-sentinel hit. Distinct from
    /// `missing` because the webview should NOT poll these — the
    /// sentinel TTL is meaningful.
    pub negative: Vec<String>,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Unknown `tier` value.
    #[error("invalid request: tier ({0:?})")]
    InvalidTier(String),
    /// `keys=` had zero entries (whitespace or empty after split).
    #[error("invalid request: keys (empty)")]
    EmptyKeys,
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// M4 outage path: every requested key was missing. The webview
    /// gets 503 + `Retry-After: {retry_after_secs}` and renders an
    /// outage banner instead of an empty page.
    #[error("bootstrap upstream is empty (M4 outage path)")]
    AllMissing {
        /// `Retry-After` header value emitted with the 503.
        retry_after_secs: u32,
        /// Number of keys requested. Diagnostic — surfaces in the
        /// envelope so the webview can size its retry policy.
        requested: usize,
    },
}

impl HandlerError {
    /// Stable error code — webview branches on this.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidTier(_) | Self::EmptyKeys => "invalid_request",
            Self::Cache(_) => "cache_failure",
            Self::AllMissing { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status for this error.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::InvalidTier(_) | Self::EmptyKeys => StatusCode::BAD_REQUEST,
            Self::Cache(_) => StatusCode::BAD_GATEWAY,
            Self::AllMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
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
        if let Self::AllMissing {
            retry_after_secs,
            requested,
        } = &self
        {
            body["error"]["retry_after_secs"] =
                serde_json::Value::from(*retry_after_secs);
            body["error"]["requested"] = serde_json::Value::from(*requested);
        }
        let status = self.status();
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static(self.code()),
        );
        if let Self::AllMissing {
            retry_after_secs, ..
        } = &self
        {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

/// Resolve which keys to fetch from the (`tier`, `keys`) pair.
///
/// `keys=` always wins; otherwise we map `tier` → canonical slice.
/// Default tier is `both` so a caller hitting the route with no
/// query params still gets a sensible response.
pub fn resolve_requested_keys(q: &BootstrapQuery) -> Result<Vec<String>, HandlerError> {
    if let Some(raw) = q.keys.as_deref() {
        let split: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if split.is_empty() {
            return Err(HandlerError::EmptyKeys);
        }
        return Ok(split);
    }
    let tier_str = q.tier.as_deref().unwrap_or("both");
    let tier = Tier::parse(tier_str)
        .ok_or_else(|| HandlerError::InvalidTier(tier_str.to_string()))?;
    Ok(tier.keys().into_iter().map(String::from).collect())
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<BootstrapQuery>,
) -> Result<Json<BootstrapResponse>, HandlerError> {
    let requested = resolve_requested_keys(&q)?;
    let key_refs: Vec<&str> = requested.iter().map(String::as_str).collect();

    let batch = get_cached_json_batch::<Value>(&state.pool, &key_refs)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let mut data: Map<String, Value> = Map::with_capacity(requested.len());
    let mut missing: Vec<String> = Vec::new();
    let mut negative: Vec<String> = Vec::new();
    for key in &requested {
        match batch.get(key) {
            Some(BatchHit::Fresh(v)) | Some(BatchHit::Stale(v)) => {
                // Cached envelope is `{_seed, data}` — the cache
                // layer's batch reader returns the inner Value
                // already unwrapped where present.
                data.insert(key.clone(), unwrap_envelope_data(v));
            }
            Some(BatchHit::NegativeSentinel) => {
                negative.push(key.clone());
            }
            Some(BatchHit::Miss) | None => {
                missing.push(key.clone());
            }
        }
    }

    // M4 fix: every key absent → 503 + Retry-After. Negative
    // sentinels do NOT count as "missing" for this purpose; they
    // are deliberately-empty cache slots that the webview should
    // not retry. An "outage" means we have no signal at all from
    // the cache: no fresh, no stale, no sentinel. So we trigger
    // the outage path only when `data.is_empty() && negative.is_empty()`.
    if data.is_empty() && negative.is_empty() && !requested.is_empty() {
        return Err(HandlerError::AllMissing {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
            requested: requested.len(),
        });
    }

    Ok(Json(BootstrapResponse {
        data,
        missing,
        negative,
    }))
}

/// The cache layer stores `Envelope<T> = { _seed, data }`. When the
/// batch reader returns a `Value`, it may either be the unwrapped
/// `data` payload (the canonical write path via `set_cached_json`)
/// or the full envelope (legacy seeders that wrote raw values
/// before the envelope contract was tightened). This helper handles
/// both shapes so the webview only ever sees the inner payload.
fn unwrap_envelope_data(v: &Value) -> Value {
    if let Value::Object(map) = v {
        if map.contains_key("_seed") {
            if let Some(inner) = map.get("data") {
                return inner.clone();
            }
        }
    }
    v.clone()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bootstrap::keys::{FAST_KEYS, SLOW_KEYS};
    use axum::response::IntoResponse;

    fn q(tier: Option<&str>, keys: Option<&str>) -> BootstrapQuery {
        BootstrapQuery {
            tier: tier.map(str::to_string),
            keys: keys.map(str::to_string),
        }
    }

    #[test]
    fn resolve_default_tier_is_both() {
        let resolved = resolve_requested_keys(&q(None, None)).unwrap();
        assert_eq!(resolved.len(), 112);
    }

    #[test]
    fn resolve_fast_tier_has_67_keys() {
        let resolved = resolve_requested_keys(&q(Some("fast"), None)).unwrap();
        assert_eq!(resolved.len(), 67);
    }

    #[test]
    fn resolve_slow_tier_has_45_keys() {
        let resolved = resolve_requested_keys(&q(Some("slow"), None)).unwrap();
        assert_eq!(resolved.len(), 45);
    }

    #[test]
    fn resolve_keys_override_takes_precedence() {
        // Even with tier=fast, the explicit keys list wins.
        let resolved =
            resolve_requested_keys(&q(Some("fast"), Some("a,b,c"))).unwrap();
        assert_eq!(resolved, vec!["a", "b", "c"]);
    }

    #[test]
    fn resolve_keys_trims_whitespace_and_drops_empties() {
        let resolved =
            resolve_requested_keys(&q(None, Some("  a , b ,, c , "))).unwrap();
        assert_eq!(resolved, vec!["a", "b", "c"]);
    }

    #[test]
    fn resolve_empty_keys_is_rejected() {
        let err = resolve_requested_keys(&q(None, Some(""))).unwrap_err();
        assert_eq!(err.code(), "invalid_request");
        let err2 = resolve_requested_keys(&q(None, Some(" , , "))).unwrap_err();
        assert_eq!(err2.code(), "invalid_request");
    }

    #[test]
    fn resolve_unknown_tier_is_rejected() {
        let err = resolve_requested_keys(&q(Some("BOGUS"), None)).unwrap_err();
        assert_eq!(err.code(), "invalid_request");
    }

    #[test]
    fn handler_error_status_codes() {
        assert_eq!(
            HandlerError::InvalidTier("x".into()).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            HandlerError::EmptyKeys.status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            HandlerError::Cache("x".into()).status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            HandlerError::AllMissing {
                retry_after_secs: 30,
                requested: 1,
            }
            .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn handler_error_codes() {
        assert_eq!(
            HandlerError::AllMissing {
                retry_after_secs: 30,
                requested: 1,
            }
            .code(),
            "bootstrap_upstream_empty"
        );
    }

    #[tokio::test]
    async fn all_missing_response_carries_retry_after_header() {
        let resp = HandlerError::AllMissing {
            retry_after_secs: 30,
            requested: 67,
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let h = resp.headers();
        assert_eq!(h.get("retry-after").unwrap(), "30");
        assert_eq!(
            h.get(GATEWAY_ERROR_CODE_HEADER).unwrap(),
            "bootstrap_upstream_empty"
        );
    }

    #[test]
    fn unwrap_envelope_data_strips_seed_wrapper() {
        let envelope = serde_json::json!({
            "_seed": { "version": "v1" },
            "data":  { "x": 1 }
        });
        let inner = unwrap_envelope_data(&envelope);
        assert_eq!(inner, serde_json::json!({ "x": 1 }));
    }

    #[test]
    fn unwrap_envelope_data_passes_through_raw_value() {
        let raw = serde_json::json!({ "x": 1 });
        let inner = unwrap_envelope_data(&raw);
        assert_eq!(inner, raw);
    }

    #[test]
    fn fast_keys_constant_round_trips_through_resolve() {
        let resolved = resolve_requested_keys(&q(Some("fast"), None)).unwrap();
        assert_eq!(resolved.len(), FAST_KEYS.len());
        for (i, k) in resolved.iter().enumerate() {
            assert_eq!(k, FAST_KEYS[i]);
        }
    }

    #[test]
    fn slow_keys_constant_round_trips_through_resolve() {
        let resolved = resolve_requested_keys(&q(Some("slow"), None)).unwrap();
        assert_eq!(resolved.len(), SLOW_KEYS.len());
        for (i, k) in resolved.iter().enumerate() {
            assert_eq!(k, SLOW_KEYS[i]);
        }
    }
}

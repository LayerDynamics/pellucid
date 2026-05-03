//! Common error type for every streams client.

use thiserror::Error;

/// Outcomes a streams client may return. The gateway's stage 11
/// (handler-error boundary) maps each variant to a fixed
/// HTTP-status + envelope-code so the webview can branch
/// deterministically.
#[derive(Clone, Debug, Error)]
pub enum StreamsError {
    /// Transport-level failure — connect refused, DNS lookup, TLS
    /// handshake, request timeout. Maps to **502 + `upstream_io`**.
    #[error("upstream io error: {0}")]
    Io(String),

    /// Upstream returned a non-2xx status. The status is preserved
    /// so the gateway can decide whether to surface it directly
    /// (for 4xx that mean "the *caller* asked for something
    /// invalid") or collapse it to **502 + `upstream_status`** (for
    /// 5xx that indicate the upstream is sick).
    #[error("upstream status {status}")]
    Status {
        /// HTTP status code returned by the upstream.
        status: u16,
    },

    /// Body present but not parseable as the expected schema. Maps
    /// to **502 + `upstream_parse`**.
    #[error("upstream parse error: {0}")]
    Parse(String),

    /// Upstream signalled "no such resource" — the streams client
    /// converts this into `Ok(None)` at the call site so the cache
    /// layer can record a negative sentinel. Reserved for sources
    /// (like aviationstack) that distinguish 404 from a transient
    /// 5xx.
    #[error("upstream returned no result for the requested key")]
    NotFound,
}

impl StreamsError {
    /// True iff the variant warrants a negative-sentinel cache entry.
    #[must_use]
    pub const fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound)
    }
}

impl From<reqwest::Error> for StreamsError {
    fn from(err: reqwest::Error) -> Self {
        if let Some(status) = err.status() {
            Self::Status {
                status: status.as_u16(),
            }
        } else {
            Self::Io(err.to_string())
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn is_not_found_only_true_for_not_found() {
        assert!(StreamsError::NotFound.is_not_found());
        assert!(!StreamsError::Io("x".into()).is_not_found());
        assert!(!StreamsError::Status { status: 503 }.is_not_found());
        assert!(!StreamsError::Parse("x".into()).is_not_found());
    }

    #[test]
    fn display_includes_underlying_detail() {
        assert!(StreamsError::Io("connect refused".into())
            .to_string()
            .contains("connect refused"));
        assert!(StreamsError::Status { status: 503 }
            .to_string()
            .contains("503"));
        assert!(StreamsError::Parse("missing field `x`".into())
            .to_string()
            .contains("missing field"));
    }
}

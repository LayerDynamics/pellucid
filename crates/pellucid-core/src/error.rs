//! Workspace error type. Every Pellucid library crate either re-exports
//! [`Error`] directly or maps its specific failures into one of these
//! variants so the gateway error mapper has a single match statement.

use thiserror::Error;

/// Result alias scoped to the Pellucid workspace.
pub type Result<T> = core::result::Result<T, Error>;

/// Top-level error variant. Variants intentionally describe **categories**
/// of failure rather than upstream-specific subtypes; specific crates
/// preserve detail in the `String` payload of the relevant variant.
#[derive(Debug, Error)]
pub enum Error {
    /// Envelope failed schema validation (missing field, wrong shape).
    #[error("invalid envelope shape: {0}")]
    InvalidEnvelope(String),

    /// Cache tier name or numeric value not recognized.
    #[error("invalid cache tier: {0}")]
    InvalidCacheTier(String),

    /// Encoded envelope payload exceeded the 5 MB cap from spec §7.4.
    #[error("payload exceeds max size: {actual} bytes > {max} bytes")]
    PayloadTooLarge {
        /// Observed encoded size in bytes.
        actual: usize,
        /// Configured maximum in bytes.
        max: usize,
    },

    /// JSON serialization or deserialization failed.
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),

    /// Underlying I/O failure surfaced from the standard library.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn invalid_envelope_carries_message() {
        let e = Error::InvalidEnvelope("missing _seed".into());
        assert!(e.to_string().contains("missing _seed"));
    }

    #[test]
    fn invalid_cache_tier_carries_message() {
        let e = Error::InvalidCacheTier("turbo".into());
        assert!(e.to_string().contains("turbo"));
    }

    #[test]
    fn payload_too_large_renders_actual_and_max() {
        let e = Error::PayloadTooLarge { actual: 10, max: 5 };
        let s = e.to_string();
        assert!(s.contains("10"));
        assert!(s.contains("5"));
    }

    #[test]
    fn from_serde_json_error() {
        let bad: core::result::Result<i32, serde_json::Error> = serde_json::from_str("not json");
        let pellucid_err: Error = bad.unwrap_err().into();
        assert!(matches!(pellucid_err, Error::Serde(_)));
    }

    #[test]
    fn from_io_error() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let pellucid_err: Error = io.into();
        assert!(matches!(pellucid_err, Error::Io(_)));
        assert!(pellucid_err.to_string().contains("missing"));
    }

    #[test]
    fn debug_renders_variant_name() {
        let e = Error::InvalidEnvelope("x".into());
        assert!(format!("{e:?}").contains("InvalidEnvelope"));
    }
}

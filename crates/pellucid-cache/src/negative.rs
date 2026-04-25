//! Negative-cache sentinel constants. The actual write/read of the
//! sentinel lives in [`crate::kv`]; this module exists so other crates
//! and tests can reference [`DEFAULT_NEGATIVE_TTL_MS`] without pulling
//! the kv module's full surface.
//!
//! SPEC-001 §7.3 — null upstream results record a row with
//! `is_negative = 1` and a short TTL (default 120 s). The TTL is
//! deliberately conservative so a flapping upstream cannot poison the
//! cache for an extended period.

/// Default TTL for a negative-sentinel row, in milliseconds.
pub const DEFAULT_NEGATIVE_TTL_MS: i64 = 120_000;

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_negative_ttl_is_120_seconds() {
        assert_eq!(DEFAULT_NEGATIVE_TTL_MS, 120_000);
    }
}

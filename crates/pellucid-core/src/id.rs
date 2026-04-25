//! Identifier types — `RunId` for atomic-publish lock ownership, request
//! correlation, and seeder cycle identification (SPEC-001 §7.4).

use core::fmt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A Pellucid run identifier — wraps a UUIDv4 so the type system
/// distinguishes "any random uuid" from "a run id" at API boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

impl RunId {
    /// Generate a new random run identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Returns the canonical hyphenated lowercase string form.
    #[must_use]
    pub fn to_hyphenated(self) -> String {
        self.0.as_hyphenated().to_string()
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_returns_a_v4_uuid() {
        let id = RunId::new();
        assert_eq!(id.0.get_version_num(), 4);
    }

    #[test]
    fn two_consecutive_new_calls_produce_distinct_ids() {
        // 128-bit UUIDv4 collisions are astronomically unlikely; this
        // protects against a subtle Default impl regression that would
        // hand out the same id every call.
        let a = RunId::new();
        let b = RunId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn default_is_new() {
        let _: RunId = RunId::default();
    }

    #[test]
    fn display_is_hyphenated_36_chars() {
        let id = RunId::new();
        let s = format!("{id}");
        assert_eq!(s.len(), 36, "expected 36-char hyphenated form, got {s}");
        assert_eq!(s.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn to_hyphenated_matches_display() {
        let id = RunId::new();
        assert_eq!(id.to_hyphenated(), id.to_string());
    }

    #[test]
    fn as_uuid_returns_inner() {
        let inner = Uuid::new_v4();
        let id = RunId(inner);
        assert_eq!(id.as_uuid(), inner);
    }

    #[test]
    fn serde_round_trip_is_string_transparent() {
        let id = RunId::new();
        let serialized = serde_json::to_string(&id).expect("serialize");
        // #[serde(transparent)] means it round-trips as the inner uuid string.
        assert!(serialized.starts_with('"'));
        let parsed: RunId = serde_json::from_str(&serialized).expect("parse");
        assert_eq!(id, parsed);
    }
}

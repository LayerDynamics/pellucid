//! Seed envelope shape — the typed contract every seeder writes
//! to `kv_envelope`.
//!
//! Mirrors the original WorldMonitor `_seed-utils.mjs` envelope:
//! `{ _seed: { fetched_at_ms, ttl_ms, source_version, record_count,
//! cascade_group, run_id }, data: <payload> }`. The cache layer
//! (`pellucid-cache`) reads this shape verbatim; the bootstrap
//! handler unwraps `data` before serving.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard size cap on serialised envelope payload — SPEC-001 §7.4
/// pins this at 5 MiB. Larger payloads are rejected before any
/// SQLite write.
pub const MAX_ENVELOPE_BYTES: usize = 5 * 1024 * 1024;

/// Seed metadata block. Stored verbatim under `_seed` in the
/// envelope JSON and replicated row-side into `seed_meta` on
/// canonical promotion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedMeta {
    /// Wall-clock ms when the upstream fetch returned. Set by
    /// the seeder, not the publisher.
    pub fetched_at_ms: i64,
    /// Time-to-live in milliseconds. The cache layer treats
    /// `fetched_at_ms + ttl_ms` as the expiry.
    pub ttl_ms: i64,
    /// Identifier the seeder uses to track schema drift (e.g.
    /// `"aviationstack-v1"`). Carried through `seed_meta` so
    /// dashboards can spot drift.
    pub source_version: String,
    /// Count of records inside `data`. Used by the relay's
    /// `/health` cascade to detect empty seeds.
    pub record_count: i64,
    /// Cascade group tag — multiple cache keys that should be
    /// considered one unit by `/health` (e.g.
    /// `"theater-posture"` for the seeder + its derived views).
    pub cascade_group: Option<String>,
    /// Per-run UUID. Set by [`crate::atomic_publish`] right
    /// before staging; never set by the seeder itself.
    pub run_id: String,
}

/// Full envelope as written to `kv_envelope`. Generic over the
/// `data` payload so each seeder can pass its own concrete
/// type; the cache layer round-trips it as `serde_json::Value`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedEnvelope<T = serde_json::Value> {
    /// Metadata block (see [`SeedMeta`]).
    #[serde(rename = "_seed")]
    pub seed: SeedMeta,
    /// Payload — the value the consumer panel renders.
    pub data: T,
}

impl SeedEnvelope<serde_json::Value> {
    /// Validate envelope shape per SPEC-001 §7.4:
    /// - `record_count >= 0`
    /// - `ttl_ms > 0`
    /// - `source_version` non-empty
    /// - `data` non-null
    /// - serialised size ≤ 5 MiB.
    ///
    /// Size check happens **last** — caller can pre-check the
    /// other invariants before paying for the JSON encode.
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if self.seed.ttl_ms <= 0 {
            return Err(EnvelopeError::InvalidTtl(self.seed.ttl_ms));
        }
        if self.seed.record_count < 0 {
            return Err(EnvelopeError::NegativeRecordCount(self.seed.record_count));
        }
        if self.seed.source_version.is_empty() {
            return Err(EnvelopeError::MissingSourceVersion);
        }
        if self.data.is_null() {
            return Err(EnvelopeError::NullData);
        }
        let encoded = serde_json::to_vec(self).map_err(EnvelopeError::Encode)?;
        if encoded.len() > MAX_ENVELOPE_BYTES {
            return Err(EnvelopeError::SizeExceeded {
                actual: encoded.len(),
                max: MAX_ENVELOPE_BYTES,
            });
        }
        Ok(())
    }

    /// Encode + size-check + return the serialised bytes. Used
    /// by [`crate::atomic_publish`] so the JSON encode happens
    /// once, not twice.
    pub fn validate_and_encode(&self) -> Result<String, EnvelopeError> {
        if self.seed.ttl_ms <= 0 {
            return Err(EnvelopeError::InvalidTtl(self.seed.ttl_ms));
        }
        if self.seed.record_count < 0 {
            return Err(EnvelopeError::NegativeRecordCount(self.seed.record_count));
        }
        if self.seed.source_version.is_empty() {
            return Err(EnvelopeError::MissingSourceVersion);
        }
        if self.data.is_null() {
            return Err(EnvelopeError::NullData);
        }
        let encoded = serde_json::to_string(self).map_err(EnvelopeError::Encode)?;
        if encoded.len() > MAX_ENVELOPE_BYTES {
            return Err(EnvelopeError::SizeExceeded {
                actual: encoded.len(),
                max: MAX_ENVELOPE_BYTES,
            });
        }
        Ok(encoded)
    }
}

/// Why an envelope failed validation. Surfaced through
/// [`crate::PublishError::Validation`] from `atomic_publish`.
#[derive(Debug, Error)]
pub enum EnvelopeError {
    /// `ttl_ms` was zero or negative.
    #[error("ttl_ms must be > 0; got {0}")]
    InvalidTtl(i64),
    /// `record_count` was negative.
    #[error("record_count must be ≥ 0; got {0}")]
    NegativeRecordCount(i64),
    /// `source_version` was empty.
    #[error("source_version must be non-empty")]
    MissingSourceVersion,
    /// `data` was JSON `null`.
    #[error("data must not be null")]
    NullData,
    /// Encoded payload exceeded the 5 MiB cap.
    #[error("envelope size {actual} exceeds cap of {max} bytes")]
    SizeExceeded {
        /// Actual encoded size in bytes.
        actual: usize,
        /// Configured cap (`MAX_ENVELOPE_BYTES`).
        max: usize,
    },
    /// JSON encoding failed.
    #[error("envelope encode failed: {0}")]
    Encode(serde_json::Error),
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn meta() -> SeedMeta {
        SeedMeta {
            fetched_at_ms: 1_746_226_800_000,
            ttl_ms: 60_000,
            source_version: "test-v1".into(),
            record_count: 3,
            cascade_group: None,
            run_id: "run-uuid".into(),
        }
    }

    fn envelope(data: serde_json::Value) -> SeedEnvelope {
        SeedEnvelope { seed: meta(), data }
    }

    #[test]
    fn well_formed_envelope_validates() {
        let env = envelope(serde_json::json!({"a": 1}));
        env.validate().unwrap();
    }

    #[test]
    fn ttl_zero_or_negative_rejected() {
        let mut env = envelope(serde_json::json!({"a": 1}));
        env.seed.ttl_ms = 0;
        assert!(matches!(env.validate(), Err(EnvelopeError::InvalidTtl(0))));
        env.seed.ttl_ms = -1;
        assert!(matches!(env.validate(), Err(EnvelopeError::InvalidTtl(-1))));
    }

    #[test]
    fn negative_record_count_rejected() {
        let mut env = envelope(serde_json::json!({"a": 1}));
        env.seed.record_count = -7;
        assert!(matches!(
            env.validate(),
            Err(EnvelopeError::NegativeRecordCount(-7))
        ));
    }

    #[test]
    fn empty_source_version_rejected() {
        let mut env = envelope(serde_json::json!({"a": 1}));
        env.seed.source_version = String::new();
        assert!(matches!(
            env.validate(),
            Err(EnvelopeError::MissingSourceVersion)
        ));
    }

    #[test]
    fn null_data_rejected() {
        let env = envelope(serde_json::Value::Null);
        assert!(matches!(env.validate(), Err(EnvelopeError::NullData)));
    }

    #[test]
    fn oversized_payload_rejected() {
        let big = serde_json::json!({ "blob": "x".repeat(MAX_ENVELOPE_BYTES) });
        let env = envelope(big);
        let err = env.validate().unwrap_err();
        assert!(
            matches!(err, EnvelopeError::SizeExceeded { actual, max }
                if actual > max && max == MAX_ENVELOPE_BYTES),
            "got {err:?}",
        );
    }

    #[test]
    fn validate_and_encode_returns_canonical_json() {
        let env = envelope(serde_json::json!({"a": 1}));
        let encoded = env.validate_and_encode().unwrap();
        assert!(encoded.contains("\"_seed\""));
        assert!(encoded.contains("\"data\""));
        assert!(encoded.contains("test-v1"));
        // Round-trips back into the typed shape.
        let back: SeedEnvelope = serde_json::from_str(&encoded).unwrap();
        assert_eq!(back, env);
    }

    #[test]
    fn cascade_group_serialises_when_present() {
        let mut env = envelope(serde_json::json!({"a": 1}));
        env.seed.cascade_group = Some("theater-posture".into());
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"cascade_group\":\"theater-posture\""));
    }
}

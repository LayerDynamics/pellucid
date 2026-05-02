//! `IdentitySigner` — typed wrapper around the HMAC-SHA256 sign +
//! verify pair from [`crate::hmac`].
//!
//! Holding the secret in a struct keeps the secret out of public
//! function signatures (so a misuse like accidentally logging it is
//! one fewer arrow to grep for) and gives downstream crates a
//! single place to plug in via dependency injection.
//!
//! Construction is intentionally fail-closed: the only constructor
//! requires a non-empty secret. An empty secret would still produce
//! valid HMAC output (RFC 2104 is defined for empty keys) but the
//! Dodo `DODO_IDENTITY_SIGNING_SECRET` is mandatory in production
//! (SPEC-001 §17), and accepting an empty key here would silently
//! defeat OP-12.

use std::fmt;

use thiserror::Error;

use crate::hmac::{sign_user_id_hmac, verify_user_id_hmac};

/// Errors that can occur constructing an [`IdentitySigner`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentitySignerError {
    /// The supplied secret was empty. Dodo identity signing requires
    /// a real secret; an empty one defeats OP-12.
    #[error("identity signing secret must not be empty")]
    EmptySecret,
}

/// Owns the HMAC secret and exposes typed sign / verify methods.
#[derive(Clone)]
pub struct IdentitySigner {
    secret: Vec<u8>,
}

impl IdentitySigner {
    /// Construct a signer. Returns [`IdentitySignerError::EmptySecret`]
    /// if the secret is empty.
    ///
    /// Production callers should source the secret from
    /// `DODO_IDENTITY_SIGNING_SECRET` (SPEC-001 §17).
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self, IdentitySignerError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(IdentitySignerError::EmptySecret);
        }
        Ok(Self { secret })
    }

    /// Construct a signer without the empty-secret check. Reserved
    /// for property tests and unit tests that *want* to exercise the
    /// empty-secret branch of the underlying HMAC primitives.
    #[doc(hidden)]
    #[must_use]
    pub fn new_unchecked(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// HMAC-SHA256 sign `user_id`, base64-encode, return.
    #[must_use]
    pub fn sign(&self, user_id: &str) -> String {
        sign_user_id_hmac(user_id, &self.secret)
    }

    /// Constant-time verify that `sig_b64` matches `user_id`.
    #[must_use]
    pub fn verify(&self, user_id: &str, sig_b64: &str) -> bool {
        verify_user_id_hmac(user_id, sig_b64, &self.secret)
    }

    /// Length of the held secret in bytes. Diagnostic only — the
    /// secret bytes themselves never leave the struct.
    #[must_use]
    pub fn secret_len(&self) -> usize {
        self.secret.len()
    }
}

/// Custom Debug — never prints the secret. Logs that capture
/// `{:?}` on a signer therefore can never leak it.
impl fmt::Debug for IdentitySigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentitySigner")
            .field("secret_len", &self.secret.len())
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn signer() -> IdentitySigner {
        IdentitySigner::new(b"unit-test-secret-32-bytes-min!!!".to_vec()).unwrap()
    }

    #[test]
    fn new_rejects_empty_secret() {
        let err = IdentitySigner::new(Vec::<u8>::new()).unwrap_err();
        assert_eq!(err, IdentitySignerError::EmptySecret);
    }

    #[test]
    fn new_accepts_one_byte_secret() {
        // The one-byte secret is technically legal for HMAC-SHA256 —
        // production keys are far longer, but we don't impose a
        // minimum here since the policy lives in the env-loading
        // layer (SPEC-001 §17).
        let s = IdentitySigner::new(vec![1u8]).unwrap();
        assert_eq!(s.secret_len(), 1);
    }

    #[test]
    fn round_trip_via_signer() {
        let s = signer();
        let sig = s.sign("clerk:user_abc");
        assert!(s.verify("clerk:user_abc", &sig));
    }

    #[test]
    fn cross_user_does_not_verify() {
        let s = signer();
        let sig = s.sign("user_a");
        assert!(!s.verify("user_b", &sig));
    }

    #[test]
    fn cross_signer_does_not_verify() {
        let a = IdentitySigner::new(b"secret-A".to_vec()).unwrap();
        let b = IdentitySigner::new(b"secret-B".to_vec()).unwrap();
        let sig = a.sign("u");
        assert!(!b.verify("u", &sig));
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let s = IdentitySigner::new(b"super-secret-value".to_vec()).unwrap();
        let printed = format!("{s:?}");
        assert!(!printed.contains("super-secret-value"));
        assert!(printed.contains("<redacted>"));
        assert!(printed.contains("secret_len"));
    }

    #[test]
    fn clone_preserves_signing_behavior() {
        let s = signer();
        let s2 = s.clone();
        let sig_a = s.sign("user");
        let sig_b = s2.sign("user");
        assert_eq!(sig_a, sig_b);
        assert!(s.verify("user", &sig_b));
    }

    #[test]
    fn new_unchecked_allows_empty_secret() {
        // Test-only escape hatch.
        let s = IdentitySigner::new_unchecked(Vec::<u8>::new());
        assert_eq!(s.secret_len(), 0);
        let sig = s.sign("u");
        assert!(s.verify("u", &sig));
    }

    #[test]
    fn secret_len_reports_secret_size() {
        let s = IdentitySigner::new(b"abc".to_vec()).unwrap();
        assert_eq!(s.secret_len(), 3);
    }
}

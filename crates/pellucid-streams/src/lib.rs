//! pellucid-streams — upstream HTTP clients for aviationstack, AIS,
//! OpenSky, RSS, Telegram, OREF, and the rest of the SPEC-001 §10
//! provider matrix.
//!
//! Each provider has its own module exposing typed `fetch_*`
//! functions that take an injected `reqwest::Client` + base URL so
//! integration tests can point them at a `wiremock` server. The
//! production binaries (`pellucid-edge-bin`, `pellucid-relay-bin`,
//! `pellucid-seeders-bin`) build a single client per process and
//! pass it through. Errors are wrapped in [`StreamsError`] so the
//! gateway's stage 11 can map them to consistent envelope shapes.

pub mod aviationstack;
pub mod error;

pub use aviationstack::{AviationstackClient, AviationstackConfig};
pub use error::StreamsError;

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty(), "version must not be empty");
        assert!(v.contains('.'), "expected semver with dot, got {v}");
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}

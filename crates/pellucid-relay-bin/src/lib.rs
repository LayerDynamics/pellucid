//! pellucid-relay-bin — library surface.
//!
//! The relay's behaviour is exposed as a thin library so the
//! integration tests (`tests/regression_c1.rs`) can drive the
//! validator without spawning the binary process itself, and so
//! T3.10 can layer the AIS / OpenSky / RSS / OREF tasks on top
//! of the same crate.

pub mod startup_check;

pub use startup_check::{
    ensure_safe_to_boot, BootDecision, StartupEnv, StartupError,
};

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
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }
}

//! pellucid-gateway
//!
//! 14-stage Tower middleware stack, router, ETag, CSP middleware.
//!
//! See `docs/specs/SPEC-001-pellucid-stack-rebuild.md` §11 for this crate's role
//! in the Pellucid workspace.

/// Returns the crate version string from `CARGO_PKG_VERSION`.
///
/// Used by the universal smoke test (per implementation plan §1.1) so every
/// crate has at least one passing unit test from the moment it is created.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
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

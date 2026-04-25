//! pellucid-sidecar-bin
//!
//! See `docs/specs/SPEC-001-pellucid-stack-rebuild.md` §11 for this binary's
//! role in the Pellucid workspace.
//!
//! Binary entry point — `println!` is the appropriate way to surface version
//! and startup banners on stdout, so we locally allow the `print_stdout` lint
//! that the workspace warns on for library code.

#![allow(clippy::print_stdout)]

/// Returns the crate version string from `CARGO_PKG_VERSION`.
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn main() {
    println!("{} {}", env!("CARGO_PKG_NAME"), version());
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
}

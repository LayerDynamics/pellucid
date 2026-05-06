//! C1 fix — relay startup gate.
//!
//! Original WorldMonitor `scripts/ais-relay.cjs:6444-6449`:
//!
//! ```js
//! function isAuthorizedRequest(req) {
//!   if (!RELAY_SHARED_SECRET) return true;   // ← bypass
//!   const provided = getRelaySecretFromRequest(req);
//!   if (!provided) return false;
//!   return safeTokenEquals(provided, RELAY_SHARED_SECRET);
//! }
//! ```
//!
//! A Railway / Fly deploy that lost the `RELAY_SHARED_SECRET` env
//! var (typo, rotation slip, fresh env) became an open proxy for
//! OpenSky quota, AIS stream, RSS proxy, and every seed endpoint
//! — silently. The C1 fix per SPEC-001 §17.8:
//!
//! 1. Hard-fail startup when `RELAY_SHARED_SECRET` is unset or
//!    empty AND `ALLOW_UNAUTHENTICATED_RELAY` is not exactly
//!    `"true"`.
//! 2. `ALLOW_UNAUTHENTICATED_RELAY=true` MUST NOT coexist with
//!    any production-hostname env var (`FLY_APP_NAME`,
//!    `RAILWAY_PROJECT_ID`, or `PELLUCID_PROD=true`). The flag
//!    is for dev only; coexistence indicates either a misconfig
//!    or an intentional bypass attempt — both are refused.
//!
//! This module is the standalone validator. The binary's `main`
//! calls [`ensure_safe_to_boot`] and bails on `Err`.
//!
//! Testability: the validator takes a [`StartupEnv`] snapshot
//! rather than reading `std::env::var` directly so unit tests can
//! drive every (secret, allow_unauth, prod_indicator) tuple
//! deterministically. The convenience helper
//! [`StartupEnv::from_process`] reads the live process env for
//! the binary's `main`.

use thiserror::Error;

/// Names of env vars consulted. Public so the integration test
/// can spawn the binary with the right keys without typos.
pub mod env_names {
    /// Shared secret the relay verifies on every inbound request.
    pub const RELAY_SHARED_SECRET: &str = "RELAY_SHARED_SECRET";
    /// Dev-only opt-out — `"true"` to skip the secret requirement.
    pub const ALLOW_UNAUTHENTICATED_RELAY: &str = "ALLOW_UNAUTHENTICATED_RELAY";
    /// Fly.io sets this on every machine.
    pub const FLY_APP_NAME: &str = "FLY_APP_NAME";
    /// Railway sets this on every service.
    pub const RAILWAY_PROJECT_ID: &str = "RAILWAY_PROJECT_ID";
    /// Pellucid's own explicit prod flag — also gates self-hosted
    /// production deploys that don't run on Fly / Railway.
    pub const PELLUCID_PROD: &str = "PELLUCID_PROD";
}

/// Snapshot of the env vars the validator consults.
///
/// Take a snapshot once at boot (via [`Self::from_process`]) and
/// pass it through. Re-reading the env between the snapshot and
/// the actual relay startup is a TOCTOU window; SPEC-001 §17.8
/// pins the snapshot pattern as the canonical shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupEnv {
    /// Raw value of `RELAY_SHARED_SECRET`. `None` = unset; the
    /// validator also rejects an empty `Some("")`.
    pub relay_shared_secret: Option<String>,
    /// `true` iff `ALLOW_UNAUTHENTICATED_RELAY == "true"` exactly.
    /// We deliberately do NOT accept `"True"` / `"1"` / `"yes"` —
    /// the strict match blocks accidental opt-ins from
    /// case-insensitive shells.
    pub allow_unauthenticated_relay: bool,
    /// `true` iff any of `FLY_APP_NAME`, `RAILWAY_PROJECT_ID`, or
    /// `PELLUCID_PROD == "true"` is set.
    pub prod_indicator: bool,
}

impl StartupEnv {
    /// Build from the live process env. Used by the binary's
    /// `main`; tests construct `StartupEnv` directly.
    ///
    /// This is the canonical env-reading boundary for the relay
    /// binary — every other code path reads from a [`StartupEnv`]
    /// snapshot rather than touching `std::env::var` directly,
    /// so the workspace's `disallowed_methods` lint stays useful
    /// elsewhere.
    #[must_use]
    #[allow(clippy::disallowed_methods)]
    pub fn from_process() -> Self {
        Self {
            relay_shared_secret: std::env::var(env_names::RELAY_SHARED_SECRET).ok(),
            allow_unauthenticated_relay: std::env::var(env_names::ALLOW_UNAUTHENTICATED_RELAY)
                .as_deref()
                == Ok("true"),
            prod_indicator: std::env::var(env_names::FLY_APP_NAME).is_ok()
                || std::env::var(env_names::RAILWAY_PROJECT_ID).is_ok()
                || std::env::var(env_names::PELLUCID_PROD).as_deref() == Ok("true"),
        }
    }

    /// `true` iff `relay_shared_secret` is `Some(non_empty)`.
    #[must_use]
    pub fn has_secret(&self) -> bool {
        self.relay_shared_secret
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    }
}

/// Why the relay refused to start.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StartupError {
    /// No secret in production. The original C1 bug.
    #[error(
        "RELAY_SHARED_SECRET unset (or empty) in production; \
         refusing to start (set the secret, or unset \
         FLY_APP_NAME / RAILWAY_PROJECT_ID / PELLUCID_PROD if \
         this really is a dev environment)"
    )]
    MissingSecretInProduction,

    /// Caller asked to bypass auth but the env says we're in
    /// production. The escape hatch refuses to coexist with prod
    /// indicators — that combination is either a misconfig or an
    /// intentional bypass attempt; both refused.
    #[error(
        "ALLOW_UNAUTHENTICATED_RELAY=true is set but a production \
         indicator (FLY_APP_NAME / RAILWAY_PROJECT_ID / PELLUCID_PROD) \
         is also present; refusing to start (the dev escape hatch \
         must not coexist with prod env vars)"
    )]
    UnauthRefusedInProduction,

    /// Neither path is satisfied. The plain "no secret, no opt-out"
    /// case the binary refuses even outside production so dev
    /// environments don't drift open.
    #[error(
        "RELAY_SHARED_SECRET is unset and ALLOW_UNAUTHENTICATED_RELAY \
         is not 'true'; refusing to start"
    )]
    MissingSecretNoBypass,
}

/// What the validator decided.
///
/// `Authorized` = a real secret is configured.
/// `UnauthDevMode` = no secret but the dev opt-out is set and we
/// are NOT in production. The `main` logs a loud warning and
/// continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootDecision {
    /// Normal authorized startup.
    Authorized,
    /// Dev-mode unauthenticated startup — log a warning.
    UnauthDevMode,
}

/// Run the validator. Returns the boot decision on success,
/// [`StartupError`] on refusal.
///
/// Implements SPEC-001 §17.8 truth table:
///
/// | secret             | allow_unauth | prod_indicator | outcome                       |
/// |--------------------|--------------|----------------|-------------------------------|
/// | Some(non-empty)    | any          | any            | Authorized                    |
/// | none / empty       | true         | false          | UnauthDevMode (warn)          |
/// | none / empty       | true         | true           | UnauthRefusedInProduction     |
/// | none / empty       | false        | true           | MissingSecretInProduction     |
/// | none / empty       | false        | false          | MissingSecretNoBypass         |
///
/// # Errors
/// Returns `StartupError` per the table above when the env tuple
/// fails the gate.
pub fn ensure_safe_to_boot(env: &StartupEnv) -> Result<BootDecision, StartupError> {
    if env.has_secret() {
        return Ok(BootDecision::Authorized);
    }
    match (env.allow_unauthenticated_relay, env.prod_indicator) {
        (true, false) => Ok(BootDecision::UnauthDevMode),
        (true, true) => Err(StartupError::UnauthRefusedInProduction),
        (false, true) => Err(StartupError::MissingSecretInProduction),
        (false, false) => Err(StartupError::MissingSecretNoBypass),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn env(secret: Option<&str>, allow_unauth: bool, prod: bool) -> StartupEnv {
        StartupEnv {
            relay_shared_secret: secret.map(str::to_string),
            allow_unauthenticated_relay: allow_unauth,
            prod_indicator: prod,
        }
    }

    #[test]
    fn nonempty_secret_authorizes_regardless_of_other_flags() {
        for allow in [false, true] {
            for prod in [false, true] {
                let result = ensure_safe_to_boot(&env(Some("s3cret"), allow, prod));
                assert_eq!(
                    result,
                    Ok(BootDecision::Authorized),
                    "(secret=Some, allow={allow}, prod={prod}) must authorize",
                );
            }
        }
    }

    #[test]
    fn empty_secret_does_not_authorize() {
        // An empty string is treated as "missing secret". The
        // legacy `process.env.RELAY_SHARED_SECRET || ""` collapse
        // is the bug we are fixing.
        let result = ensure_safe_to_boot(&env(Some(""), true, false));
        // Empty + allow_unauth + dev → UnauthDevMode (the
        // empty-string is equivalent to missing for our purposes).
        assert_eq!(result, Ok(BootDecision::UnauthDevMode));

        let result = ensure_safe_to_boot(&env(Some(""), false, false));
        assert_eq!(result, Err(StartupError::MissingSecretNoBypass));
    }

    #[test]
    fn missing_secret_in_dev_with_explicit_opt_in_warns_only() {
        let result = ensure_safe_to_boot(&env(None, true, false));
        assert_eq!(result, Ok(BootDecision::UnauthDevMode));
    }

    #[test]
    fn missing_secret_in_prod_with_opt_in_is_refused() {
        // The killer scenario: a dev sets ALLOW_UNAUTHENTICATED
        // somewhere, then deploys to Fly. The flag MUST refuse
        // to coexist with the prod indicator.
        let result = ensure_safe_to_boot(&env(None, true, true));
        assert_eq!(result, Err(StartupError::UnauthRefusedInProduction));
    }

    #[test]
    fn missing_secret_in_prod_without_opt_in_is_refused() {
        let result = ensure_safe_to_boot(&env(None, false, true));
        assert_eq!(result, Err(StartupError::MissingSecretInProduction));
    }

    #[test]
    fn missing_secret_in_dev_without_opt_in_is_refused() {
        // Even in dev, no secret + no opt-out means we refuse —
        // dev environments shouldn't drift open by accident.
        let result = ensure_safe_to_boot(&env(None, false, false));
        assert_eq!(result, Err(StartupError::MissingSecretNoBypass));
    }

    /// One row of the C1 truth table.
    type TruthRow = (
        Option<&'static str>,
        bool,
        bool,
        Result<BootDecision, StartupError>,
    );

    #[test]
    fn full_truth_table_exhaustive() {
        // Drive every combination of (secret-present, allow,
        // prod) and lock down the expected outcome. This is the
        // load-bearing C1 invariant — any reordering of the
        // match arms will break this test loudly.
        let cases: &[TruthRow] = &[
            (Some("s"), false, false, Ok(BootDecision::Authorized)),
            (Some("s"), false, true, Ok(BootDecision::Authorized)),
            (Some("s"), true, false, Ok(BootDecision::Authorized)),
            (Some("s"), true, true, Ok(BootDecision::Authorized)),
            (
                Some(""),
                false,
                false,
                Err(StartupError::MissingSecretNoBypass),
            ),
            (
                Some(""),
                false,
                true,
                Err(StartupError::MissingSecretInProduction),
            ),
            (Some(""), true, false, Ok(BootDecision::UnauthDevMode)),
            (
                Some(""),
                true,
                true,
                Err(StartupError::UnauthRefusedInProduction),
            ),
            (None, false, false, Err(StartupError::MissingSecretNoBypass)),
            (
                None,
                false,
                true,
                Err(StartupError::MissingSecretInProduction),
            ),
            (None, true, false, Ok(BootDecision::UnauthDevMode)),
            (
                None,
                true,
                true,
                Err(StartupError::UnauthRefusedInProduction),
            ),
        ];
        for (secret, allow, prod, expected) in cases {
            let got = ensure_safe_to_boot(&env(*secret, *allow, *prod));
            assert_eq!(
                got, *expected,
                "(secret={secret:?}, allow={allow}, prod={prod}) → expected {expected:?}, got {got:?}"
            );
        }
    }

    #[test]
    fn has_secret_treats_empty_as_missing() {
        let none = env(None, false, false);
        let empty = env(Some(""), false, false);
        let real = env(Some("s"), false, false);
        assert!(!none.has_secret());
        assert!(!empty.has_secret());
        assert!(real.has_secret());
    }

    #[test]
    fn allow_unauthenticated_strict_string_match_only() {
        // The from_process reader uses `as_deref() == Ok("true")`
        // — anything else (`"True"`, `"TRUE"`, `"1"`, `"yes"`,
        // ` true `) is NOT accepted. We exercise the constructed
        // shape directly here; the from_process branch is
        // covered separately when the integration test spawns
        // the binary with mismatched values.
        let strict = env(None, true, false);
        assert_eq!(
            ensure_safe_to_boot(&strict),
            Ok(BootDecision::UnauthDevMode)
        );
        let lenient_caller = env(None, false, false);
        assert_eq!(
            ensure_safe_to_boot(&lenient_caller),
            Err(StartupError::MissingSecretNoBypass)
        );
    }

    #[test]
    fn startup_error_messages_explain_remediation() {
        // The message users see in the deploy log MUST tell them
        // what to do.
        let m = StartupError::MissingSecretInProduction.to_string();
        assert!(m.contains("RELAY_SHARED_SECRET"));
        assert!(m.contains("FLY_APP_NAME"));

        let m = StartupError::UnauthRefusedInProduction.to_string();
        assert!(m.contains("ALLOW_UNAUTHENTICATED_RELAY"));
        assert!(m.contains("dev escape hatch"));

        let m = StartupError::MissingSecretNoBypass.to_string();
        assert!(m.contains("RELAY_SHARED_SECRET"));
    }
}

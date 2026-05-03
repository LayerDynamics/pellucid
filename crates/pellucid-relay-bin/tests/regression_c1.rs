#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! C1 regression test — locks down SPEC-001 §17.8 / §24.
//!
//! The original WorldMonitor relay's `isAuthorizedRequest`
//! (`scripts/ais-relay.cjs:6444-6449`) returned `true`
//! unconditionally when `RELAY_SHARED_SECRET` was unset. A Fly /
//! Railway deploy that lost the secret env (typo, rotation slip,
//! fresh env) silently became an open proxy for OpenSky quota,
//! AIS stream, RSS proxy, and every seed endpoint.
//!
//! This test boots the **real** binary as a child process with
//! curated env tuples and asserts the C1 gate is wired:
//!  - production env without secret → exit non-zero.
//!  - dev env with no secret + no opt-in → exit non-zero.
//!  - dev env with explicit `ALLOW_UNAUTHENTICATED_RELAY=true` →
//!    exit zero (warning printed).
//!  - any env with a real secret → exit zero (banner printed).
//!  - `ALLOW_UNAUTHENTICATED_RELAY=true` PLUS any prod indicator
//!    → exit non-zero (escape hatch refuses to coexist).
//!
//! ## Fix-fail manual procedure
//!
//! 1. Open `crates/pellucid-relay-bin/src/main.rs`.
//! 2. Replace the `match ensure_safe_to_boot(...)` block with a
//!    plain `println!("authorized")` + early return — i.e. revert
//!    to the legacy "if the secret is missing, we just continue"
//!    behaviour.
//! 3. Run: `cargo nextest run -p pellucid-relay-bin --test regression_c1`.
//! 4. The `*_in_production_*` tests fail: the binary now exits 0
//!    even though no secret was set in prod.
//! 5. Restore main.rs → green again.

use std::process::{Command, Output, Stdio};

use pellucid_relay_bin::startup_check::env_names;

const EXIT_REFUSED: i32 = 78;

/// Resolve the path to the just-built binary. We ask cargo to
/// expose it via the `CARGO_BIN_EXE_<name>` env var that
/// integration tests inherit automatically.
fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_pellucid-relay-bin")
}

/// Spawn the binary with EXACTLY the env vars in `env_pairs`.
/// We pre-clear every env var the gate consults so a stray one
/// in the test runner's env (e.g. `FLY_APP_NAME` in CI) cannot
/// poison the result.
fn spawn(env_pairs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(binary_path());
    // Wipe every env var the validator looks at, then set only
    // the ones this case wants. We do NOT use `env_clear()`
    // because the binary still needs basic process env
    // (PATH, PWD, …) to start.
    for var in [
        env_names::RELAY_SHARED_SECRET,
        env_names::ALLOW_UNAUTHENTICATED_RELAY,
        env_names::FLY_APP_NAME,
        env_names::RAILWAY_PROJECT_ID,
        env_names::PELLUCID_PROD,
    ] {
        cmd.env_remove(var);
    }
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn relay binary")
}

fn assert_authorized_exit(out: &Output, label: &str) {
    assert!(
        out.status.success(),
        "{label}: expected exit 0, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

fn assert_refused_exit(out: &Output, label: &str) {
    assert_eq!(
        out.status.code(),
        Some(EXIT_REFUSED),
        "{label}: expected exit code {EXIT_REFUSED}, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn nonempty_secret_dev_authorizes() {
    let out = spawn(&[(env_names::RELAY_SHARED_SECRET, "real-secret-xyz")]);
    assert_authorized_exit(&out, "secret-only dev");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("authorized startup"), "stdout: {stdout}");
}

#[test]
fn nonempty_secret_in_production_authorizes() {
    let out = spawn(&[
        (env_names::RELAY_SHARED_SECRET, "real-secret-xyz"),
        (env_names::FLY_APP_NAME, "pellucid-relay"),
    ]);
    assert_authorized_exit(&out, "secret + FLY_APP_NAME");

    let out = spawn(&[
        (env_names::RELAY_SHARED_SECRET, "real-secret-xyz"),
        (env_names::RAILWAY_PROJECT_ID, "abc123"),
    ]);
    assert_authorized_exit(&out, "secret + RAILWAY_PROJECT_ID");

    let out = spawn(&[
        (env_names::RELAY_SHARED_SECRET, "real-secret-xyz"),
        (env_names::PELLUCID_PROD, "true"),
    ]);
    assert_authorized_exit(&out, "secret + PELLUCID_PROD");
}

#[test]
fn no_secret_in_production_refuses_with_exit_78() {
    // The killer scenario the C1 fix exists to catch.
    let out = spawn(&[(env_names::FLY_APP_NAME, "pellucid-relay")]);
    assert_refused_exit(&out, "no secret + FLY_APP_NAME");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("RELAY_SHARED_SECRET"),
        "stderr should explain the missing var: {stderr}",
    );
}

#[test]
fn no_secret_in_railway_refuses() {
    let out = spawn(&[(env_names::RAILWAY_PROJECT_ID, "abc123")]);
    assert_refused_exit(&out, "no secret + RAILWAY_PROJECT_ID");
}

#[test]
fn no_secret_with_pellucid_prod_refuses() {
    let out = spawn(&[(env_names::PELLUCID_PROD, "true")]);
    assert_refused_exit(&out, "no secret + PELLUCID_PROD=true");
}

#[test]
fn no_secret_no_opt_in_dev_refuses() {
    // Even in dev the gate refuses — dev shouldn't drift open.
    let out = spawn(&[]);
    assert_refused_exit(&out, "no secret, no opt-in, no prod");
}

#[test]
fn allow_unauthenticated_in_dev_authorizes_with_warning() {
    let out = spawn(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "true")]);
    assert_authorized_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=true (dev)");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("WARNING") && stderr.contains("dev mode"),
        "stderr should warn loudly: {stderr}"
    );
}

#[test]
fn allow_unauthenticated_with_fly_app_refuses() {
    // The escape hatch + production indicator combination MUST
    // refuse.
    let out = spawn(&[
        (env_names::ALLOW_UNAUTHENTICATED_RELAY, "true"),
        (env_names::FLY_APP_NAME, "pellucid-relay"),
    ]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED + FLY_APP_NAME");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ALLOW_UNAUTHENTICATED_RELAY"),
        "stderr should call out the conflicting flag: {stderr}",
    );
}

#[test]
fn allow_unauthenticated_with_railway_refuses() {
    let out = spawn(&[
        (env_names::ALLOW_UNAUTHENTICATED_RELAY, "true"),
        (env_names::RAILWAY_PROJECT_ID, "abc"),
    ]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED + RAILWAY_PROJECT_ID");
}

#[test]
fn allow_unauthenticated_with_pellucid_prod_refuses() {
    let out = spawn(&[
        (env_names::ALLOW_UNAUTHENTICATED_RELAY, "true"),
        (env_names::PELLUCID_PROD, "true"),
    ]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED + PELLUCID_PROD=true");
}

#[test]
fn allow_unauthenticated_strict_string_match_in_main() {
    // `ALLOW_UNAUTHENTICATED_RELAY=True` (capitalised) must NOT
    // satisfy the opt-out — strict equality only. The binary's
    // from_process reader uses `as_deref() == Ok("true")`.
    let out = spawn(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "True")]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=True (capitalised)");

    let out = spawn(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "1")]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=1");

    let out = spawn(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "yes")]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=yes");
}

#[test]
fn empty_secret_in_production_refuses() {
    // The original `process.env.RELAY_SHARED_SECRET || ""` shape:
    // an empty string MUST be treated as missing.
    let out = spawn(&[
        (env_names::RELAY_SHARED_SECRET, ""),
        (env_names::FLY_APP_NAME, "pellucid-relay"),
    ]);
    assert_refused_exit(&out, "empty secret + FLY_APP_NAME");
}

#[test]
fn pellucid_prod_false_does_not_count_as_production() {
    // Only `"true"` exact match should trip the prod indicator
    // for PELLUCID_PROD. `"false"` (or anything else) means
    // dev — combined with no secret + no opt-in we still refuse,
    // but with a different error code than the production path.
    let out = spawn(&[(env_names::PELLUCID_PROD, "false")]);
    // Still refused (no secret, no opt-in) but the error message
    // should be the no-bypass shape, not the in-production shape.
    assert_refused_exit(&out, "PELLUCID_PROD=false, no secret, no opt-in");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("FLY_APP_NAME"),
        "PELLUCID_PROD=false should not surface a production-path error: {stderr}",
    );
}

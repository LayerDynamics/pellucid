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

use std::io::BufReader;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use pellucid_relay_bin::startup_check::env_names;

const EXIT_REFUSED: i32 = 78;

/// Resolve the path to the just-built binary. We ask cargo to
/// expose it via the `CARGO_BIN_EXE_<name>` env var that
/// integration tests inherit automatically.
fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_pellucid-relay-bin")
}

/// Pick an unused loopback port. The binary picks a real
/// listener port post-gate; without a free port the bind
/// would race + the test would flake.
fn ephemeral_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// Build a `Command` with the gate env tuple curated to
/// exactly what the test wants. Common to both spawn helpers.
fn build_command(env_pairs: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(binary_path());
    for var in [
        env_names::RELAY_SHARED_SECRET,
        env_names::ALLOW_UNAUTHENTICATED_RELAY,
        env_names::FLY_APP_NAME,
        env_names::RAILWAY_PROJECT_ID,
        env_names::PELLUCID_PROD,
        "PELLUCID_RELAY_LISTEN_ADDR",
        "PELLUCID_DB_URL",
    ] {
        cmd.env_remove(var);
    }
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    cmd
}

/// Spawn the binary, block until it exits, and capture
/// stdout/stderr. Used for refused-boot cases — the gate
/// rejects + the process exits with `EXIT_REFUSED` before
/// touching any I/O, so blocking on `output()` is safe.
fn spawn_blocking(env_pairs: &[(&str, &str)]) -> Output {
    build_command(env_pairs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn relay binary")
}

/// Outcome of a non-blocking authorized-boot probe — the
/// binary now actually serves traffic past the gate, so we
/// can't `output()`-block forever. Instead we read both
/// pipes until the banner appears, kill the child, and
/// drain the rest.
struct AuthorizedProbe {
    /// Captured stdout (banner + any post-banner log lines
    /// that arrived before we killed the child).
    stdout: String,
    /// Captured stderr (warning banner + tracing output).
    stderr: String,
}

/// Spawn the binary on a free loopback port + in-memory DB,
/// wait for an authorized banner OR a refusal exit, then
/// return what we captured. Kills the child if it stayed
/// alive past the banner.
fn spawn_authorized_probe(env_pairs: &[(&str, &str)]) -> AuthorizedProbe {
    let port = ephemeral_port();
    let listen = format!("127.0.0.1:{port}");
    let mut owned: Vec<(String, String)> = env_pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    owned.push(("PELLUCID_RELAY_LISTEN_ADDR".into(), listen));
    owned.push(("PELLUCID_DB_URL".into(), "sqlite::memory:".into()));
    let pairs: Vec<(&str, &str)> =
        owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut child = build_command(&pairs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn relay binary");

    let stdout_pipe = child.stdout.take().unwrap();
    let stderr_pipe = child.stderr.take().unwrap();

    // Drain stdout + stderr concurrently in worker threads —
    // both pipes have OS buffer limits + a stuck reader
    // deadlocks the child.
    let stdout_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout_pipe);
        let mut acc = String::new();
        let _ = std::io::Read::read_to_string(&mut reader, &mut acc);
        acc
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr_pipe);
        let mut acc = String::new();
        let _ = std::io::Read::read_to_string(&mut reader, &mut acc);
        acc
    });

    // Give the child up to 5s to either print its banner +
    // start serving (loop exits on first try_wait that says
    // "still running") or exit early (e.g. the gate refused).
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            // Process exited on its own — drain pipes + return.
            let stdout = stdout_handle.join().unwrap();
            let stderr = stderr_handle.join().unwrap();
            return AuthorizedProbe { stdout, stderr };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Still running — kill it. The pipe readers will see EOF
    // once the kernel closes the descriptors.
    let _ = child.kill();
    let _ = child.wait();
    let stdout = stdout_handle.join().unwrap();
    let stderr = stderr_handle.join().unwrap();
    AuthorizedProbe { stdout, stderr }
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
    let probe = spawn_authorized_probe(&[(env_names::RELAY_SHARED_SECRET, "real-secret-xyz")]);
    assert!(
        probe.stdout.contains("authorized startup"),
        "secret-only dev: missing banner.\nstdout: {}\nstderr: {}",
        probe.stdout,
        probe.stderr,
    );
}

#[test]
fn nonempty_secret_in_production_authorizes() {
    for (label, prod_var) in [
        ("secret + FLY_APP_NAME", env_names::FLY_APP_NAME),
        ("secret + RAILWAY_PROJECT_ID", env_names::RAILWAY_PROJECT_ID),
        ("secret + PELLUCID_PROD", env_names::PELLUCID_PROD),
    ] {
        let probe = spawn_authorized_probe(&[
            (env_names::RELAY_SHARED_SECRET, "real-secret-xyz"),
            (prod_var, "pellucid-relay"),
        ]);
        assert!(
            probe.stdout.contains("authorized startup"),
            "{label}: missing banner.\nstdout: {}\nstderr: {}",
            probe.stdout,
            probe.stderr,
        );
    }
}

#[test]
fn no_secret_in_production_refuses_with_exit_78() {
    // The killer scenario the C1 fix exists to catch.
    let out = spawn_blocking(&[(env_names::FLY_APP_NAME, "pellucid-relay")]);
    assert_refused_exit(&out, "no secret + FLY_APP_NAME");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("RELAY_SHARED_SECRET"),
        "stderr should explain the missing var: {stderr}",
    );
}

#[test]
fn no_secret_in_railway_refuses() {
    let out = spawn_blocking(&[(env_names::RAILWAY_PROJECT_ID, "abc123")]);
    assert_refused_exit(&out, "no secret + RAILWAY_PROJECT_ID");
}

#[test]
fn no_secret_with_pellucid_prod_refuses() {
    let out = spawn_blocking(&[(env_names::PELLUCID_PROD, "true")]);
    assert_refused_exit(&out, "no secret + PELLUCID_PROD=true");
}

#[test]
fn no_secret_no_opt_in_dev_refuses() {
    // Even in dev the gate refuses — dev shouldn't drift open.
    let out = spawn_blocking(&[]);
    assert_refused_exit(&out, "no secret, no opt-in, no prod");
}

#[test]
fn allow_unauthenticated_in_dev_authorizes_with_warning() {
    let probe =
        spawn_authorized_probe(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "true")]);
    assert!(
        probe.stderr.contains("WARNING") && probe.stderr.contains("dev mode"),
        "stderr should warn loudly.\nstdout: {}\nstderr: {}",
        probe.stdout,
        probe.stderr,
    );
}

#[test]
fn allow_unauthenticated_with_fly_app_refuses() {
    // The escape hatch + production indicator combination MUST
    // refuse.
    let out = spawn_blocking(&[
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
    let out = spawn_blocking(&[
        (env_names::ALLOW_UNAUTHENTICATED_RELAY, "true"),
        (env_names::RAILWAY_PROJECT_ID, "abc"),
    ]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED + RAILWAY_PROJECT_ID");
}

#[test]
fn allow_unauthenticated_with_pellucid_prod_refuses() {
    let out = spawn_blocking(&[
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
    let out = spawn_blocking(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "True")]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=True (capitalised)");

    let out = spawn_blocking(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "1")]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=1");

    let out = spawn_blocking(&[(env_names::ALLOW_UNAUTHENTICATED_RELAY, "yes")]);
    assert_refused_exit(&out, "ALLOW_UNAUTHENTICATED_RELAY=yes");
}

#[test]
fn empty_secret_in_production_refuses() {
    // The original `process.env.RELAY_SHARED_SECRET || ""` shape:
    // an empty string MUST be treated as missing.
    let out = spawn_blocking(&[
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
    let out = spawn_blocking(&[(env_names::PELLUCID_PROD, "false")]);
    // Still refused (no secret, no opt-in) but the error message
    // should be the no-bypass shape, not the in-production shape.
    assert_refused_exit(&out, "PELLUCID_PROD=false, no secret, no opt-in");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("FLY_APP_NAME"),
        "PELLUCID_PROD=false should not surface a production-path error: {stderr}",
    );
}

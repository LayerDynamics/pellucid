//! M0 Gate regression test — proves the full host → sidecar live path.
//!
//! Walks the SPEC-001 §28 M0 exit criterion end-to-end:
//!   1. Build (or reuse) the real `pellucid-sidecar-bin` binary.
//!   2. Spawn it through `SidecarSupervisor::spawn_with_env` with the
//!      same `PELLUCID_SIDECAR_TOKEN` the host's seed mints.
//!   3. Attach the supervisor to a `LocalApiState` exactly the way
//!      `app::setup_main_window` does.
//!   4. Hit `/api/echo` from a real HTTP client using the seed bearer.
//!      Expected: 200, payload echoed back.
//!   5. Drive a `TokenRotator::rotate_now`, persist to vault, forward
//!      the rotation through `LocalApiState::forward_token_rotation_to_sidecar`.
//!   6. Confirm the sidecar accepts BOTH the new bearer (current) and
//!      the previous bearer (overlap window) — H1 contract.
//!   7. Confirm an unrelated random token still 401s.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use pellucid_tauri::{
    resolve_sidecar_binary_path, Clock, InMemoryVault, LocalApiState, ManualClock, SidecarHandle,
    SidecarSupervisor, TokenRotator, Variant, Vault, DEFAULT_OVERLAP_MS,
    DEFAULT_ROTATION_INTERVAL_MS,
};

const SEED_TOKEN: &str = "m0gate-seed-token";

fn workspace_root() -> PathBuf {
    // Cargo guarantees `CARGO_MANIFEST_DIR` is the *crate* dir during
    // test compilation; the workspace root is its parent's parent.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .expect("workspace root for crates/pellucid-tauri")
}

fn ensure_sidecar_binary() -> PathBuf {
    let target_debug = workspace_root().join("target").join("debug");
    if let Ok(p) = resolve_sidecar_binary_path(&target_debug) {
        return p;
    }
    // Build the sidecar in-place. Use the same target dir cargo
    // would have used for the test itself so we do not pollute a
    // different tree.
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "pellucid-sidecar-bin", "--quiet"])
        .current_dir(workspace_root())
        .status()
        .expect("invoke cargo build for pellucid-sidecar-bin");
    assert!(status.success(), "cargo build of sidecar bin failed");
    resolve_sidecar_binary_path(&target_debug).expect("sidecar binary present after build")
}

async fn echo(port: u16, bearer: Option<&str>) -> reqwest::StatusCode {
    let mut req =
        reqwest::Client::new().get(format!("http://127.0.0.1:{port}/api/echo?message=ping"));
    if let Some(b) = bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    req.send().await.expect("/api/echo round-trip").status()
}

#[tokio::test]
async fn m0_gate_round_trip_via_real_sidecar() {
    let bin = ensure_sidecar_binary();

    // Build LocalApiState the way app::setup_main_window does.
    let sidecar_handle = SidecarHandle::from_port(0);
    let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
    let state = LocalApiState::new(sidecar_handle, vault, Variant::Base);

    // Spawn the real binary with the seed token in its env.
    let supervisor = SidecarSupervisor::spawn_with_env(
        &bin,
        &[],
        &[("PELLUCID_SIDECAR_TOKEN".to_string(), SEED_TOKEN.to_string())],
    )
    .await
    .expect("spawn sidecar binary");
    let port = supervisor.handle().port();
    state.set_sidecar_port(port);
    state.attach_sidecar_supervisor(Arc::new(supervisor));

    // /api/echo round-trip with the seed bearer.
    let status = echo(port, Some(SEED_TOKEN)).await;
    assert_eq!(status.as_u16(), 200, "seed bearer must return 200");

    // Anonymous request must 401.
    let status = echo(port, None).await;
    assert_eq!(status.as_u16(), 401, "anonymous /api/echo must 401");

    // Drive a rotation through the rotator + state, exactly the way
    // the production tokio task does it.
    let clock = Arc::new(ManualClock::new());
    let rotator = Arc::new(TokenRotator::with_schedule(
        SEED_TOKEN.to_string(),
        clock.clone() as Arc<dyn Clock>,
        DEFAULT_ROTATION_INTERVAL_MS,
        DEFAULT_OVERLAP_MS,
    ));
    state.attach_rotator(rotator.clone());
    let outcome = rotator.rotate_now().expect("rotation");
    state
        .persist_rotation(&outcome)
        .await
        .expect("persist rotation");
    state
        .forward_token_rotation_to_sidecar(&outcome.new_token, Some(outcome.retired_token.as_str()))
        .await
        .expect("forward TOKEN_ROTATED to sidecar");

    // Give the sidecar a moment to ingest the stdin line.
    for _ in 0..40 {
        let s = echo(port, Some(&outcome.new_token)).await;
        if s.as_u16() == 200 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // New token accepted.
    assert_eq!(
        echo(port, Some(&outcome.new_token)).await.as_u16(),
        200,
        "post-rotation new bearer must return 200"
    );
    // Previous (seed) token still accepted inside the overlap.
    assert_eq!(
        echo(port, Some(SEED_TOKEN)).await.as_u16(),
        200,
        "previous bearer must still be accepted inside overlap"
    );
    // Random token rejected.
    assert_eq!(
        echo(port, Some("never-issued-token")).await.as_u16(),
        401,
        "unknown bearer must 401"
    );

    // Tear down — detaching the supervisor drops the only remaining
    // Arc (state held one, we held nothing) and the child exits via
    // stdin EOF.
    state.detach_sidecar_supervisor();
}

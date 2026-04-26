//! H1 regression test — rotation cadence + 30 s overlap.
//!
//! Locks down the SPEC-001 §10.3 contract so a future patch that drops
//! rotation, shrinks the overlap, or stops persisting tokens trips a
//! red CI signal. Manual fix-fail procedure (per the task plan):
//!
//! ```bash
//! # 1. Comment out `state.attach_rotator(r.clone());` in this file.
//! # 2. Run: cargo nextest run -p pellucid-tauri --test regression_h1.
//! # 3. Test fails — rotation never reaches LocalApiState.
//! # 4. Restore the line — test passes.
//! ```

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pellucid_tauri::{
    Clock, InMemoryVault, LocalApiState, ManualClock, RotationOutcome, SidecarHandle,
    TokenRotator, Variant, Vault, DEFAULT_OVERLAP_MS, DEFAULT_ROTATION_INTERVAL_MS,
};

fn boot_state(initial_token: &str) -> (LocalApiState, Arc<TokenRotator>, Arc<ManualClock>) {
    let sidecar = SidecarHandle::from_port(46_222);
    let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
    let state = LocalApiState::new(sidecar, vault, Variant::Base);
    let clock = Arc::new(ManualClock::new());
    let rotator = Arc::new(TokenRotator::with_schedule(
        initial_token.to_string(),
        clock.clone() as Arc<dyn Clock>,
        DEFAULT_ROTATION_INTERVAL_MS,
        DEFAULT_OVERLAP_MS,
    ));
    state.attach_rotator(rotator.clone());
    (state, rotator, clock)
}

#[tokio::test]
async fn rotation_produces_a_distinct_token_after_5_minute_tick() {
    // SPEC §10.3: rotation cadence is 5 minutes.
    let (state, rotator, clock) = boot_state("v-initial");
    let captured_first = state.local_api_token().expect("seed token visible");

    // Advance the clock past the rotation interval and trigger one.
    clock.advance(DEFAULT_ROTATION_INTERVAL_MS);
    let outcome = rotator.rotate_now().unwrap();
    state.persist_rotation(&outcome).await.unwrap();

    let captured_second = state.local_api_token().expect("post-rotation token");
    assert_ne!(
        captured_first, captured_second,
        "after 5 minutes the IPC layer must serve a new token"
    );
    assert_eq!(captured_second, outcome.new_token);
}

#[tokio::test]
async fn sidecar_accepts_both_tokens_during_overlap_then_drops_previous() {
    // SPEC §10.3: the sidecar accepts current OR previous for 30 s.
    let (state, rotator, clock) = boot_state("v1");
    let v1 = state.local_api_token().unwrap();
    let outcome = rotator.rotate_now().unwrap();
    state.persist_rotation(&outcome).await.unwrap();
    let v2 = state.local_api_token().unwrap();
    assert_ne!(v1, v2);

    // Inside the overlap window both tokens are accepted.
    clock.advance(DEFAULT_OVERLAP_MS - 1);
    assert!(state.accepts_token(&v2), "current must be accepted");
    assert!(state.accepts_token(&v1), "previous must be accepted inside overlap");

    // After the overlap elapses the previous token must be rejected.
    clock.advance(2);
    assert!(state.accepts_token(&v2), "current still accepted post-overlap");
    assert!(
        !state.accepts_token(&v1),
        "previous must be dropped past 30 s — H1 regression"
    );
    assert!(!state.accepts_token("never-issued"));
}

#[tokio::test]
async fn persist_rotation_writes_into_vault_so_restart_path_is_warm() {
    // After process restart the boot path reads the consolidated blob;
    // the rotation history (current + previous) must already be there.
    let (state, rotator, _clock) = boot_state("v1");
    let outcome = rotator.rotate_now().unwrap();
    state.persist_rotation(&outcome).await.unwrap();

    let blob = state.vault().read().await.unwrap();
    assert_eq!(blob.sidecar_token.as_deref(), Some(outcome.new_token.as_str()));
    assert_eq!(blob.sidecar_token_previous.as_deref(), Some("v1"));
}

#[tokio::test]
async fn three_rotations_under_compressed_clock_yield_three_distinct_tokens() {
    // Advance the clock through three rotation windows and assert no
    // token is ever re-used. This guards against accidental state
    // sharing or a "rotate" that resets to the original seed.
    let (state, rotator, clock) = boot_state("seed");
    let mut seen = std::collections::HashSet::new();
    seen.insert(state.local_api_token().unwrap());

    for _ in 0..3 {
        clock.advance(DEFAULT_ROTATION_INTERVAL_MS);
        let outcome: RotationOutcome = rotator.rotate_now().unwrap();
        state.persist_rotation(&outcome).await.unwrap();
        assert!(
            seen.insert(state.local_api_token().unwrap()),
            "rotation produced a duplicate token"
        );
    }
    assert_eq!(seen.len(), 4, "seed + 3 rotations should be 4 unique tokens");
}

#[tokio::test]
async fn refresh_secrets_returns_rotation_state_post_rotation() {
    // The webview calls `refresh_secrets` to learn the new bearer.
    // The bundle must reflect the rotator's view, not the old cached
    // vault contents.
    let (state, rotator, clock) = boot_state("v1");
    let pre = state.refresh_secrets().await.unwrap();
    assert_eq!(pre.sidecar_token.as_deref(), Some("v1"));
    assert!(pre.sidecar_token_previous.is_none());

    clock.advance(DEFAULT_ROTATION_INTERVAL_MS);
    let outcome = rotator.rotate_now().unwrap();
    state.persist_rotation(&outcome).await.unwrap();

    let post = state.refresh_secrets().await.unwrap();
    assert_eq!(post.sidecar_token.as_deref(), Some(outcome.new_token.as_str()));
    assert_eq!(post.sidecar_token_previous.as_deref(), Some("v1"));
}

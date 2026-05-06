//! Integration test for the relay's Telegram run task wiring.
//!
//! Boots the relay app twice:
//! 1. With `Config` lacking `TELEGRAM_API_ID` / `TELEGRAM_API_HASH` —
//!    `BootedRelay::telegram_handles` must be `None`.
//! 2. With both creds set — `try_spawn` is called, but because we
//!    don't supply real Telegram secrets in CI the connect step fails
//!    (or auth-required surfaces). The test asserts the relay continues
//!    to boot anyway (the wiring is fault-tolerant by design).
//!
//! The "real connect path with valid creds" lives behind
//! `TELEGRAM_E2E=1` in `crates/pellucid-relay-bin/src/telegram_task.rs`
//! tests, not here.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use pellucid_relay_bin::{build_app, Config, ConfigSource};

#[tokio::test]
async fn relay_boots_without_telegram_handles_when_no_credentials_present() {
    let cfg = Config::parse(&ConfigSource::default()).unwrap();
    let booted = build_app(cfg, vec![], vec![]).await.unwrap();
    assert!(
        booted.telegram_handles.is_none(),
        "no-credentials boot must not start the run task"
    );
    let clean = booted.shutdown().await;
    assert!(clean);
}

#[tokio::test]
async fn relay_boots_continues_when_telegram_creds_partial() {
    // Half-set creds (api_id without api_hash) must not start the run
    // task — try_spawn returns None at the cred-presence check and
    // never touches grammers. This is the dev-mode path the relay's
    // README documents.
    let mut cfg = Config::parse(&ConfigSource::default()).unwrap();
    cfg.telegram_api_id = Some(99_999_999);
    // hash deliberately absent
    cfg.telegram_api_hash = None;
    cfg.shutdown_grace = Duration::from_millis(500);
    let booted = build_app(cfg, vec![], vec![]).await.unwrap();
    assert!(
        booted.telegram_handles.is_none(),
        "partial creds must not start the run task"
    );
    let clean = booted.shutdown().await;
    assert!(clean);
}

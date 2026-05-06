//! Integration test for the sidecar's extended stdin protocol.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use pellucid_sidecar::{process_stdin_loop, TokenSet};
use pellucid_streams::telegram::session::{IpcSessionStore, SessionEvent, SessionStore};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn telegram_session_updated_line_mutates_store_and_emits_event() {
    let tokens = TokenSet::new("initial-token".into());
    let store = Arc::new(IpcSessionStore::new());
    let mut rx = store.subscribe();

    let payload = BASE64_STANDARD.encode([0xAA_u8, 0xBB, 0xCC]);
    let line = format!("TELEGRAM_SESSION_UPDATED {payload}\nSHUTDOWN\n");
    let reader = tokio::io::BufReader::new(line.as_bytes());

    process_stdin_loop(reader, &tokens, &store).await;

    let _ = rx.changed().await;
    assert_eq!(*rx.borrow_and_update(), SessionEvent::Updated);
    assert_eq!(store.load().await.unwrap(), Some(vec![0xAA_u8, 0xBB, 0xCC]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn telegram_session_cleared_line_clears_store() {
    let tokens = TokenSet::new("initial-token".into());
    let store = Arc::new(IpcSessionStore::with_bytes(b"existing".to_vec()));
    let mut rx = store.subscribe();

    let line = "TELEGRAM_SESSION_CLEARED\nSHUTDOWN\n".to_string();
    let reader = tokio::io::BufReader::new(line.as_bytes());

    process_stdin_loop(reader, &tokens, &store).await;

    let _ = rx.changed().await;
    assert_eq!(*rx.borrow_and_update(), SessionEvent::Cleared);
    assert!(store.load().await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_line_does_not_break_loop() {
    let tokens = TokenSet::new("initial-token".into());
    let store = Arc::new(IpcSessionStore::new());

    let payload = BASE64_STANDARD.encode([1_u8, 2]);
    let stream =
        format!("UNKNOWN_DIRECTIVE foo bar\nTELEGRAM_SESSION_UPDATED {payload}\nSHUTDOWN\n");
    let reader = tokio::io::BufReader::new(stream.as_bytes());

    process_stdin_loop(reader, &tokens, &store).await;

    assert_eq!(store.load().await.unwrap(), Some(vec![1_u8, 2]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_base64_payload_is_logged_but_does_not_break_loop() {
    let tokens = TokenSet::new("initial-token".into());
    let store = Arc::new(IpcSessionStore::new());

    let stream = "TELEGRAM_SESSION_UPDATED not-base64!!\nSHUTDOWN\n".to_string();
    let reader = tokio::io::BufReader::new(stream.as_bytes());

    process_stdin_loop(reader, &tokens, &store).await;

    assert!(store.load().await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_rotated_continues_to_work_alongside_telegram_lines() {
    // Sanity: the original TOKEN_ROTATED branch is preserved in the
    // refactored `process_stdin_loop`.
    let tokens = TokenSet::new("old".into());
    let store = Arc::new(IpcSessionStore::new());

    let stream = "TOKEN_ROTATED new prev\nSHUTDOWN\n".to_string();
    let reader = tokio::io::BufReader::new(stream.as_bytes());

    process_stdin_loop(reader, &tokens, &store).await;

    assert_eq!(tokens.current(), "new");
    assert_eq!(tokens.previous().as_deref(), Some("prev"));
}

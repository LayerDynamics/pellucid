//! pellucid-sidecar-bin — desktop API binary entry point.
//!
//! Boots the Axum server on a random `127.0.0.1:<port>` socket, prints
//! `PORT=<n>` on stdout for the host's `SidecarSupervisor`, and reads a
//! line-oriented control protocol from stdin so the host can push
//! token-rotation events into the running process without restarting.
//!
//! Stdin protocol (one command per line):
//! - `TOKEN_ROTATED <new_current> <previous>` — replace the token pair.
//! - `TOKEN_ROTATED <new_current> -` — replace current, clear previous.
//! - `TELEGRAM_SESSION_UPDATED <base64>` — host pushed new MTProto
//!   session bytes (T4.5.0). The sidecar's `IpcSessionStore` updates
//!   in-memory and emits `Updated` to subscribers.
//! - `TELEGRAM_SESSION_CLEARED` — host cleared the MTProto session
//!   (user logged out). Sidecar's run task drains gracefully.
//! - `SHUTDOWN` — graceful exit.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Write;
#[cfg(feature = "telegram")]
use std::sync::Arc;

use pellucid_sidecar::{process_stdin_loop, serve_on_random_port, TokenSet, STDOUT_PORT_PREFIX};
#[cfg(feature = "telegram")]
use pellucid_telegram::session::IpcSessionStore;
use tokio::io::BufReader;
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let initial = read_initial_token_from_env();
    let tokens = TokenSet::new(initial);

    let handle = serve_on_random_port(tokens.clone()).await?;
    {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{STDOUT_PORT_PREFIX}{}", handle.port)?;
        out.flush()?;
    }
    tracing::info!(target: "pellucid::sidecar", port = handle.port, "sidecar listening");

    #[cfg(feature = "telegram")]
    let session_store = Arc::new(IpcSessionStore::new());

    let stdin = tokio::io::stdin();
    process_stdin_loop(
        BufReader::new(stdin),
        &tokens,
        #[cfg(feature = "telegram")]
        &session_store,
    )
    .await;

    handle.task.abort();
    Ok(())
}

fn read_initial_token_from_env() -> String {
    if let Some(value) = std::env::var_os("PELLUCID_SIDECAR_TOKEN") {
        if let Ok(s) = value.into_string() {
            if !s.is_empty() {
                return s;
            }
        }
    }
    tracing::warn!(
        target: "pellucid::sidecar",
        "PELLUCID_SIDECAR_TOKEN not set, generating ephemeral token"
    );
    let mut buf = [0u8; 32];
    match read_dev_urandom(&mut buf) {
        Ok(()) => buf
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(""),
        Err(()) => "fallback-insecure-do-not-ship".to_string(),
    }
}

/// Read entropy from `/dev/urandom`. Used as the fallback when the
/// host did not provide a `PELLUCID_SIDECAR_TOKEN`. The host's
/// rotation loop will replace this token within seconds, so this is a
/// short-lived bootstrap path only.
fn read_dev_urandom(buf: &mut [u8]) -> Result<(), ()> {
    use std::io::Read;
    match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)) {
        Ok(()) => Ok(()),
        Err(_) => Err(()),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("PELLUCID_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower=warn,axum=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

// T4.5.0 — `apply_token_rotated` moved into
// `pellucid_sidecar::stdin_protocol`. The token-rotation behavior
// previously asserted here is now covered by
// `tests/stdin_telegram_protocol.rs::token_rotated_continues_to_work_alongside_telegram_lines`
// and the existing `tests/token_rotation.rs` integration test.

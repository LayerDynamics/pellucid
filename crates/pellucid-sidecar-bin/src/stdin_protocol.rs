//! Sidecar stdin control protocol.
//!
//! Lives in the library half (not `main.rs`) so integration tests can
//! drive the parser with canned line streams without forking a child
//! process. The binary entry point in `src/main.rs` calls
//! [`process_stdin_loop`] once at boot.
//!
//! Recognised lines (one per `\n`):
//! - `TOKEN_ROTATED <new_current> <previous>` — rotate the bearer
//!   token pair. `<previous>` may be `-` to clear it.
//! - `TELEGRAM_SESSION_UPDATED <base64>` — host pushed new MTProto
//!   session bytes (T4.5.0). Decoded into the [`IpcSessionStore`];
//!   subscribers see `SessionEvent::Updated`.
//! - `TELEGRAM_SESSION_CLEARED` — host cleared the MTProto session
//!   (logout). Subscribers see `SessionEvent::Cleared`.
//! - `SHUTDOWN` — break the loop and let the binary exit.
//!
//! Unrecognised lines are logged at `warn` and ignored.

use std::sync::Arc;

use pellucid_streams::telegram::session::IpcSessionStore;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::auth::TokenSet;

/// Drive the stdin control loop. Returns when `SHUTDOWN` is read or
/// EOF is reached. Pure (apart from `tracing` calls) so integration
/// tests can feed canned line streams.
pub async fn process_stdin_loop<R: AsyncBufRead + Unpin>(
    reader: R,
    tokens: &TokenSet,
    session_store: &Arc<IpcSessionStore>,
) {
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "SHUTDOWN" {
            tracing::info!(target: "pellucid::sidecar", "received SHUTDOWN, exiting");
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("TOKEN_ROTATED ") {
            apply_token_rotated(tokens, rest);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("TELEGRAM_SESSION_UPDATED ") {
            apply_telegram_session_updated(session_store, rest);
            continue;
        }
        if trimmed == "TELEGRAM_SESSION_CLEARED" {
            session_store.apply_ipc_cleared();
            tracing::info!(
                target: "pellucid::sidecar",
                "telegram session cleared via stdin"
            );
            continue;
        }
        tracing::warn!(
            target: "pellucid::sidecar",
            "unrecognised control line: {trimmed:?}"
        );
    }
}

fn apply_token_rotated(tokens: &TokenSet, rest: &str) {
    let mut parts = rest.split_whitespace();
    let Some(new_current) = parts.next() else {
        tracing::warn!(target: "pellucid::sidecar", "TOKEN_ROTATED missing new current");
        return;
    };
    let previous = parts.next();
    let previous = match previous {
        Some("-") | None => None,
        Some(other) => Some(other.to_string()),
    };
    tokens.set_pair(new_current.to_string(), previous);
    tracing::info!(target: "pellucid::sidecar", "token pair updated via stdin");
}

fn apply_telegram_session_updated(store: &IpcSessionStore, base64_payload: &str) {
    match store.apply_ipc_updated(base64_payload) {
        Ok(()) => {
            tracing::info!(
                target: "pellucid::sidecar",
                "telegram session updated via stdin"
            );
        }
        Err(err) => {
            tracing::warn!(
                target: "pellucid::sidecar",
                error = %err,
                "ignored TELEGRAM_SESSION_UPDATED with invalid payload"
            );
        }
    }
}

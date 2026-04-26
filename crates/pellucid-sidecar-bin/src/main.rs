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
//! - `SHUTDOWN` — graceful exit.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Write;

use pellucid_sidecar::{serve_on_random_port, TokenSet, STDOUT_PORT_PREFIX};
use tokio::io::{AsyncBufReadExt, BufReader};
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

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
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
            apply_token_rotated(&tokens, rest);
            continue;
        }
        tracing::warn!(
            target: "pellucid::sidecar",
            "unrecognised control line: {trimmed:?}"
        );
    }

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
    match std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
    {
        Ok(()) => Ok(()),
        Err(_) => Err(()),
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

fn init_tracing() {
    let filter = EnvFilter::try_from_env("PELLUCID_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower=warn,axum=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn apply_token_rotated_with_dash_clears_previous() {
        let t = TokenSet::new("c".into());
        apply_token_rotated(&t, "fresh -");
        assert_eq!(t.current(), "fresh");
        assert!(t.previous().is_none());
    }

    #[test]
    fn apply_token_rotated_with_two_args_records_pair() {
        let t = TokenSet::new("c".into());
        apply_token_rotated(&t, "fresh stale");
        assert_eq!(t.current(), "fresh");
        assert_eq!(t.previous().as_deref(), Some("stale"));
    }

    #[test]
    fn apply_token_rotated_ignores_empty_payload() {
        let t = TokenSet::new("c".into());
        apply_token_rotated(&t, "");
        assert_eq!(t.current(), "c");
    }
}

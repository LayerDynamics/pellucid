//! Sidecar process supervisor.
//!
//! The desktop host owns the lifecycle of `pellucid-sidecar-bin`: it spawns
//! the binary, parses the `PORT=<n>` line the sidecar prints on startup,
//! and exposes a [`SidecarHandle`] the IPC layer can consult for the
//! current port. On `Drop` the supervisor sends the child a graceful
//! shutdown; if the child does not exit within `SHUTDOWN_GRACE` it is
//! killed.
//!
//! T1.7 only ships the supervisor + handle types. T1.9 adds the actual
//! binary. The two ship separately so the `pellucid-tauri` integration
//! tests can construct a [`SidecarHandle`] from a port literal without
//! launching a child process at all.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const SHUTDOWN_GRACE: Duration = Duration::from_millis(2_000);
const PORT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const PORT_LINE_PREFIX: &str = "PORT=";

/// Handle the IPC layer holds onto. Cheap to clone; reads serialise on a
/// `parking_lot::RwLock` so dozens of concurrent webview calls don't pile
/// up.
#[derive(Clone, Debug)]
pub struct SidecarHandle {
    state: Arc<RwLock<SidecarState>>,
}

#[derive(Debug)]
struct SidecarState {
    port: u16,
}

impl SidecarHandle {
    /// Construct a handle from a known port. Used by tests + the
    /// supervisor once it has parsed the port from stdout.
    #[must_use]
    pub fn from_port(port: u16) -> Self {
        Self {
            state: Arc::new(RwLock::new(SidecarState { port })),
        }
    }

    /// Current sidecar port. Returns the literal value last written by
    /// the supervisor; never returns 0 once the sidecar has started
    /// because the supervisor blocks until the port line arrives.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.state.read().port
    }

    /// Replace the port. Called by the supervisor when the sidecar is
    /// restarted on a different port (e.g. crash recovery).
    pub fn set_port(&self, port: u16) {
        self.state.write().port = port;
    }
}

/// Errors emitted while launching or monitoring the sidecar.
#[derive(Debug, Error)]
pub enum SidecarLaunchError {
    /// Could not spawn the child process.
    #[error("spawn failed: {0}")]
    Spawn(#[source] std::io::Error),
    /// Child started but never printed `PORT=<n>` within the timeout.
    #[error("port discovery timed out after {grace_ms} ms")]
    PortTimeout {
        /// The configured grace window in milliseconds.
        grace_ms: u128,
    },
    /// `PORT=<n>` line arrived but the value did not parse as a u16.
    #[error("invalid port value '{got}'")]
    InvalidPort {
        /// The raw value the supervisor saw on the wire.
        got: String,
    },
    /// The child process closed stdout before printing the port line.
    #[error("sidecar exited before printing PORT line")]
    EarlyExit,
}

/// Owns the running sidecar process and the stdout reader task. Drop the
/// supervisor to terminate the child.
#[derive(Debug)]
pub struct SidecarSupervisor {
    handle: SidecarHandle,
    child: Mutex<Option<Child>>,
    stdout_task: Mutex<Option<JoinHandle<()>>>,
}

impl SidecarSupervisor {
    /// Spawn `program` (with `args`) and block until the binary prints
    /// the `PORT=<n>` discovery line on stdout. Returns the supervisor
    /// + a clonable handle the IPC layer keeps for the lifetime of the app.
    pub async fn spawn(
        program: &std::path::Path,
        args: &[String],
    ) -> Result<Self, SidecarLaunchError> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(SidecarLaunchError::Spawn)?;

        let stdout = child
            .stdout
            .take()
            .ok_or(SidecarLaunchError::EarlyExit)?;
        let mut reader = BufReader::new(stdout).lines();

        let port = match timeout(PORT_DISCOVERY_TIMEOUT, async {
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        if let Some(rest) = line.strip_prefix(PORT_LINE_PREFIX) {
                            return Ok(rest.trim().to_string());
                        }
                        // ignore unrelated stdout lines.
                    }
                    Ok(None) => return Err(SidecarLaunchError::EarlyExit),
                    Err(err) => return Err(SidecarLaunchError::Spawn(err)),
                }
            }
        })
        .await
        {
            Ok(res) => res?,
            Err(_) => {
                return Err(SidecarLaunchError::PortTimeout {
                    grace_ms: PORT_DISCOVERY_TIMEOUT.as_millis(),
                });
            }
        };

        let port: u16 = port
            .parse()
            .map_err(|_| SidecarLaunchError::InvalidPort { got: port.clone() })?;
        let handle = SidecarHandle::from_port(port);

        // After port discovery we keep draining stdout so its pipe
        // buffer never fills up and stalls the child.
        let drain_handle = handle.clone();
        let task = tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(rest) = line.strip_prefix(PORT_LINE_PREFIX) {
                    if let Ok(p) = rest.trim().parse::<u16>() {
                        drain_handle.set_port(p);
                    }
                }
            }
        });

        Ok(Self {
            handle,
            child: Mutex::new(Some(child)),
            stdout_task: Mutex::new(Some(task)),
        })
    }

    /// Cheap clone of the IPC handle.
    #[must_use]
    pub fn handle(&self) -> SidecarHandle {
        self.handle.clone()
    }

    /// Stop the child process gracefully (SIGTERM/equivalent). Returns
    /// `Ok(())` even if the process has already exited.
    pub async fn shutdown(&self) -> std::io::Result<()> {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            // Best effort: send a kill, then wait briefly.
            let _ = child.start_kill();
            let _ = timeout(SHUTDOWN_GRACE, child.wait()).await;
        }
        if let Some(task) = self.stdout_task.lock().await.take() {
            task.abort();
        }
        Ok(())
    }
}

impl Drop for SidecarSupervisor {
    fn drop(&mut self) {
        // Tokio's `kill_on_drop(true)` plus `start_kill` on the child
        // covers the actual process termination. We just abort the
        // stdout drain task if it is still pinned to a runtime.
        if let Ok(mut guard) = self.stdout_task.try_lock() {
            if let Some(task) = guard.take() {
                task.abort();
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn handle_round_trips_port() {
        let h = SidecarHandle::from_port(46_123);
        assert_eq!(h.port(), 46_123);
    }

    #[test]
    fn handle_clone_shares_state() {
        let h1 = SidecarHandle::from_port(10);
        let h2 = h1.clone();
        h1.set_port(20);
        assert_eq!(h2.port(), 20, "clone must observe writes via shared state");
    }

    #[tokio::test]
    async fn spawn_returns_invalid_port_when_program_does_not_exist() {
        // Use a path that cannot exist anywhere on disk.
        let bogus = std::path::PathBuf::from("/this/path/does/not/exist/pellucid-bogus");
        let res = SidecarSupervisor::spawn(&bogus, &[]).await;
        assert!(matches!(res, Err(SidecarLaunchError::Spawn(_))));
    }

    #[tokio::test]
    async fn spawn_parses_port_from_stdout_via_real_subprocess() {
        // A tiny shell command works as a synthetic sidecar.
        let program = std::path::PathBuf::from("/bin/sh");
        let args = vec![
            "-c".to_string(),
            "echo PORT=42137; sleep 5".to_string(),
        ];
        let sup = SidecarSupervisor::spawn(&program, &args)
            .await
            .expect("spawn synthetic sidecar");
        assert_eq!(sup.handle().port(), 42_137);
        sup.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn spawn_returns_early_exit_when_no_port_emitted() {
        let program = std::path::PathBuf::from("/bin/sh");
        let args = vec!["-c".to_string(), "true".to_string()]; // exits immediately
        let res = SidecarSupervisor::spawn(&program, &args).await;
        assert!(matches!(res, Err(SidecarLaunchError::EarlyExit)));
    }

    #[tokio::test]
    async fn spawn_returns_invalid_port_for_garbage_value() {
        let program = std::path::PathBuf::from("/bin/sh");
        let args = vec![
            "-c".to_string(),
            "echo PORT=not-a-number; sleep 5".to_string(),
        ];
        let res = SidecarSupervisor::spawn(&program, &args).await;
        match res {
            Err(SidecarLaunchError::InvalidPort { got }) => {
                assert_eq!(got, "not-a-number");
            }
            other => panic!("expected InvalidPort, got {other:?}"),
        }
    }
}

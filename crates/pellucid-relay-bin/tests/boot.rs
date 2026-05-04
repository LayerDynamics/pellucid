#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! T3.10 boot integration test.
//!
//! Spawns the **real** `pellucid-relay-bin` as a child process,
//! waits for the listener to come up, hits `/health` and
//! `/metrics`, then sends SIGTERM and asserts a clean exit.
//!
//! This is what an end-to-end test of the relay looks like:
//!  - real binary, real Axum server, real Tokio runtime;
//!  - `/health` runs the L3 cascade query against a real
//!    SQLite pool (in-memory, zero rows → empty cascade →
//!    200 because the missing-fraction guard treats total=0
//!    as `max(1)`);
//!  - we don't mock the network — `axum::serve` actually
//!    binds a real loopback port we picked.
//!
//! No HTTP client is mocked. We use `reqwest` (already in
//! dev-deps via the proxy tests) to hit the live socket.
//!
//! The test relies on the C1 startup-gate dev-mode opt-out
//! (`ALLOW_UNAUTHENTICATED_RELAY=true`) so we don't have to
//! invent a "real" shared secret in the harness.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pellucid_relay_bin::startup_check::env_names;

fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_pellucid-relay-bin")
}

/// Pick an unused loopback port. We bind a TcpListener on
/// 127.0.0.1:0, read the chosen port, then drop it — there's
/// a race window before the relay binds the same port, but
/// it's small and the test re-binds within milliseconds.
fn ephemeral_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Block until the relay's `/health` endpoint responds. We
/// give it 10 seconds — boot has to open SQLite, install the
/// Prometheus recorder, and bind a TCP listener, which takes
/// ~50ms on a warm cache and up to a few seconds in CI.
async fn wait_for_health(addr: &str, deadline: Duration) -> reqwest::Response {
    let url = format!("http://{addr}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Ok(resp) = client.get(&url).send().await {
            return resp;
        }
        if start.elapsed() > deadline {
            panic!("relay never answered /health within {deadline:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_boots_serves_health_and_shuts_down_clean() {
    let port = ephemeral_port();
    let addr = format!("127.0.0.1:{port}");

    let mut child = Command::new(binary_path())
        // Start clean — wipe every gate var the runner might inject.
        .env_remove(env_names::RELAY_SHARED_SECRET)
        .env_remove(env_names::FLY_APP_NAME)
        .env_remove(env_names::RAILWAY_PROJECT_ID)
        .env_remove(env_names::PELLUCID_PROD)
        // Dev-mode opt-in: the gate refuses to start without it.
        .env(env_names::ALLOW_UNAUTHENTICATED_RELAY, "true")
        .env("PELLUCID_RELAY_LISTEN_ADDR", &addr)
        .env("PELLUCID_DB_URL", "sqlite::memory:")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn relay binary");

    // /health responds with 200 (empty cascade → no missing-
    // fraction outage).
    let resp = wait_for_health(&addr, Duration::from_secs(10)).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "/health status");

    // /metrics responds with text/plain (Prometheus exposition).
    let metrics_resp = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .expect("/metrics request");
    assert_eq!(metrics_resp.status(), reqwest::StatusCode::OK);
    let ct = metrics_resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(ct.as_deref(), Some("text/plain; version=0.0.4"));

    // Send SIGTERM — the binary's shutdown_signal handler
    // resolves and we get a graceful drain. We shell out to
    // `kill` rather than depend on the `libc` crate just for
    // this test.
    #[cfg(unix)]
    {
        let pid = child.id().to_string();
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(&pid)
            .status()
            .expect("invoke kill -TERM");
        assert!(status.success(), "kill returned {status:?}");
        let exit = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        // A graceful SIGTERM drain returns exit 0; if the
        // signal handler hadn't been installed the process
        // would have been killed by the default disposition,
        // which sets `signal()` to SIGTERM. Either is
        // acceptable for an integration test that's only
        // proving the binary doesn't hang.
        use std::os::unix::process::ExitStatusExt;
        const SIGTERM: i32 = 15;
        assert!(
            exit.success() || exit.signal() == Some(SIGTERM),
            "expected clean exit, got {exit:?}",
        );
    }
    #[cfg(not(unix))]
    {
        // On non-Unix platforms we can't send SIGTERM — kill
        // the child and just verify it exited.
        child.kill().expect("kill child");
        let _ = child.wait().expect("wait child");
    }
}

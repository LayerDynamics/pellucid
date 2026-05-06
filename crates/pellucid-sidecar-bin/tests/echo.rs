//! Integration test for the sidecar binary — boots
//! `pellucid-sidecar-bin` on a random port, parses `PORT=<n>` from
//! stdout, and drives the `/api/echo` endpoint over a real HTTP socket.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

const TOKEN: &str = "test-token-1234";

struct SpawnedSidecar {
    child: Child,
    port: u16,
}

async fn spawn_sidecar() -> SpawnedSidecar {
    let bin = env!("CARGO_BIN_EXE_pellucid-sidecar-bin");
    let mut cmd = Command::new(bin);
    cmd.env("PELLUCID_SIDECAR_TOKEN", TOKEN)
        .env("PELLUCID_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = cmd.spawn().expect("spawn sidecar binary");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut lines = BufReader::new(stdout).lines();

    let port = timeout(Duration::from_secs(5), async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("read stdout line")
                .expect("stdout closed before PORT=");
            if let Some(rest) = line.strip_prefix("PORT=") {
                return rest.trim().parse::<u16>().expect("parse port");
            }
        }
    })
    .await
    .expect("PORT line within 5s");

    SpawnedSidecar { child, port }
}

async fn shutdown(mut s: SpawnedSidecar) {
    use tokio::io::AsyncWriteExt;
    if let Some(mut stdin) = s.child.stdin.take() {
        let _ = stdin.write_all(b"SHUTDOWN\n").await;
        let _ = stdin.flush().await;
    }
    let _ = timeout(Duration::from_secs(2), s.child.wait()).await;
}

#[tokio::test]
async fn echo_get_with_valid_bearer_returns_200_and_message_round_trip() {
    let sidecar = spawn_sidecar().await;
    let url = format!("http://127.0.0.1:{}/api/echo?message=ping", sidecar.port);
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .expect("http request");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("parse json");
    assert_eq!(body["message"].as_str(), Some("ping"));
    shutdown(sidecar).await;
}

#[tokio::test]
async fn echo_post_with_valid_bearer_round_trips_payload() {
    let sidecar = spawn_sidecar().await;
    let url = format!("http://127.0.0.1:{}/api/echo", sidecar.port);
    let body = serde_json::json!({
        "message": "from-test",
        "payload": { "n": 7, "list": [1, 2, 3] },
    });
    let resp = reqwest::Client::new()
        .post(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .json(&body)
        .send()
        .await
        .expect("http request");
    assert_eq!(resp.status().as_u16(), 200);
    let parsed: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(parsed["message"].as_str(), Some("from-test"));
    assert_eq!(parsed["payload"]["n"].as_i64(), Some(7));
    shutdown(sidecar).await;
}

#[tokio::test]
async fn echo_without_bearer_returns_401() {
    let sidecar = spawn_sidecar().await;
    let url = format!("http://127.0.0.1:{}/api/echo", sidecar.port);
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .expect("http request");
    assert_eq!(resp.status().as_u16(), 401);
    shutdown(sidecar).await;
}

#[tokio::test]
async fn echo_with_wrong_bearer_returns_401() {
    let sidecar = spawn_sidecar().await;
    let url = format!("http://127.0.0.1:{}/api/echo", sidecar.port);
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", "Bearer not-the-token")
        .send()
        .await
        .expect("http request");
    assert_eq!(resp.status().as_u16(), 401);
    shutdown(sidecar).await;
}

#[tokio::test]
async fn token_rotated_stdin_command_swaps_accepted_tokens() {
    use tokio::io::AsyncWriteExt;
    let mut sidecar = spawn_sidecar().await;
    let url = format!("http://127.0.0.1:{}/api/echo", sidecar.port);

    // Initial token works.
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    // Push a TOKEN_ROTATED event over stdin: new=v2, previous=initial.
    {
        let stdin = sidecar.child.stdin.as_mut().expect("stdin");
        stdin
            .write_all(format!("TOKEN_ROTATED v2 {TOKEN}\n").as_bytes())
            .await
            .unwrap();
        stdin.flush().await.unwrap();
    }
    // Tiny pause to let the sidecar process the line.
    tokio::time::sleep(Duration::from_millis(80)).await;

    // New token must be accepted.
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", "Bearer v2")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    // Old token still accepted (overlap).
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    // Unknown token rejected.
    let resp = reqwest::Client::new()
        .get(&url)
        .header("authorization", "Bearer nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);

    shutdown(sidecar).await;
}

#[tokio::test]
async fn healthz_does_not_require_bearer() {
    let sidecar = spawn_sidecar().await;
    let url = format!("http://127.0.0.1:{}/healthz", sidecar.port);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body.as_str(), "ok");
    shutdown(sidecar).await;
}

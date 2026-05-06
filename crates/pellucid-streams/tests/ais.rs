#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! AIS WebSocket client integration tests (T3.1).
//!
//! Wires the production `AisClient` against a hand-rolled local
//! WebSocket server (a `tokio::net::TcpListener` + tungstenite
//! upgrade) so we can drive a deterministic frame stream without
//! depending on the public aisstream.io endpoint.
//!
//! The local server:
//!   1. Accepts the WS upgrade.
//!   2. Reads the first text frame (the `AisSubscribe` payload)
//!      and stashes it for assertion.
//!   3. Pushes a fixed sequence of [`AisEnvelope`]-shaped frames
//!      to the client.
//!
//! The client subscribes via [`AisClient::subscribe`] and the
//! test asserts every envelope arrives byte-faithfully on the
//! broadcast channel.

use std::sync::Arc;
use std::time::Duration;

use futures::sink::SinkExt as _;
use futures::stream::StreamExt as _;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

use pellucid_streams::ais::{AisClient, Backoff};
use pellucid_streams::types::{AisEnvelope, AisSubscribe};

/// Spin up a local WS server on an ephemeral port that:
/// - accepts one client,
/// - records the first text frame (the subscribe message),
/// - replays `frames_to_push` to the client,
/// - then closes.
///
/// Returns `(ws_url, captured_subscribe)`. The captured handle is
/// populated as soon as the test client sends its handshake.
async fn spawn_ws_server(frames_to_push: Vec<String>) -> (String, Arc<Mutex<Option<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let captured_for_task = captured.clone();
    tokio::spawn(async move {
        let (stream, _peer) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut write, mut read) = ws.split();
        // Capture the first text frame (the subscribe handshake).
        if let Some(Ok(Message::Text(text))) = read.next().await {
            *captured_for_task.lock().await = Some(text.to_string());
        }
        // Push every frame, then close.
        for frame in frames_to_push {
            if write.send(Message::Text(frame)).await.is_err() {
                break;
            }
        }
        let _ = write.send(Message::Close(None)).await;
    });
    (format!("ws://127.0.0.1:{port}"), captured)
}

fn position_report_frame(mmsi: u32) -> String {
    serde_json::json!({
        "MessageType": "PositionReport",
        "MetaData": {
            "MMSI": mmsi,
            "ShipName": format!("VESSEL-{mmsi}"),
            "latitude": 30.0 + (mmsi as f64) * 0.001,
            "longitude": 32.0,
            "time_utc": "2026-05-02T12:00:00.000+0000"
        },
        "Message": {
            "PositionReport": {
                "Sog": 12.4,
                "Cog": 245.0
            }
        }
    })
    .to_string()
}

#[tokio::test]
async fn client_receives_replayed_frames_in_order() {
    let frames = vec![
        position_report_frame(367_000_001),
        position_report_frame(367_000_002),
        position_report_frame(367_000_003),
    ];
    let (ws_url, captured) = spawn_ws_server(frames.clone()).await;

    let client = AisClient::new(&ws_url, "integration-test-key", 32).with_backoff(Backoff {
        initial_ms: 50,
        max_ms: 200,
        reset_after_ms: 10_000,
    });
    let mut rx = client.subscribe();
    // Spawn the run loop; it will exit when every consumer drops.
    let runner = tokio::spawn(client.run());

    let mut received = Vec::new();
    for _ in 0..frames.len() {
        match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Ok(env)) => received.push(env),
            other => panic!("recv failed: {other:?}"),
        }
    }
    assert_eq!(received.len(), 3);
    let mmsi: Vec<u32> = received.iter().map(|e| e.metadata.mmsi).collect();
    assert_eq!(mmsi, vec![367_000_001, 367_000_002, 367_000_003]);

    // Drop receiver so the run loop exits on the next reconnect.
    drop(rx);
    let _ = tokio::time::timeout(Duration::from_secs(2), runner).await;

    // Assert the subscribe handshake the server saw matches what
    // we configured.
    let captured_text = captured.lock().await.clone();
    let captured_text = captured_text.expect("subscribe handshake must arrive");
    let parsed: AisSubscribe = serde_json::from_str(&captured_text).unwrap();
    assert_eq!(parsed.api_key, "integration-test-key");
    assert_eq!(parsed.bounding_boxes.len(), 1);
    assert_eq!(parsed.bounding_boxes[0][0], [-90.0, -180.0]);
    assert_eq!(parsed.bounding_boxes[0][1], [90.0, 180.0]);
}

#[tokio::test]
async fn client_decodes_envelope_metadata_byte_faithfully() {
    let frames = vec![position_report_frame(367_999_999)];
    let (ws_url, _captured) = spawn_ws_server(frames).await;

    let client = AisClient::new(&ws_url, "k", 32).with_backoff(Backoff {
        initial_ms: 50,
        max_ms: 200,
        reset_after_ms: 10_000,
    });
    let mut rx = client.subscribe();
    let runner = tokio::spawn(client.run());

    let env: AisEnvelope = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(env.message_type, "PositionReport");
    assert_eq!(env.metadata.mmsi, 367_999_999);
    assert!(env.metadata.ship_name.unwrap().starts_with("VESSEL-"));
    let sog = env.message.pointer("/PositionReport/Sog").unwrap();
    assert!((sog.as_f64().unwrap() - 12.4).abs() < 1e-9);

    drop(rx);
    let _ = tokio::time::timeout(Duration::from_secs(2), runner).await;
}

#[tokio::test]
async fn malformed_frame_is_skipped_session_continues() {
    // First frame is junk → must NOT tear down the session.
    // Second frame is good → must arrive.
    let frames = vec![
        "not even json".to_string(),
        position_report_frame(367_111_111),
    ];
    let (ws_url, _captured) = spawn_ws_server(frames).await;

    let client = AisClient::new(&ws_url, "k", 32).with_backoff(Backoff {
        initial_ms: 50,
        max_ms: 200,
        reset_after_ms: 10_000,
    });
    let mut rx = client.subscribe();
    let runner = tokio::spawn(client.run());

    // Only one decoded envelope should arrive (the malformed
    // frame is silently dropped).
    let env = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(env.metadata.mmsi, 367_111_111);

    drop(rx);
    let _ = tokio::time::timeout(Duration::from_secs(2), runner).await;
}

#[tokio::test]
async fn fan_out_to_multiple_subscribers() {
    let frames = vec![position_report_frame(367_222_222)];
    let (ws_url, _captured) = spawn_ws_server(frames).await;

    let client = AisClient::new(&ws_url, "k", 32).with_backoff(Backoff {
        initial_ms: 50,
        max_ms: 200,
        reset_after_ms: 10_000,
    });
    let mut a = client.subscribe();
    let mut b = client.subscribe();
    let runner = tokio::spawn(client.run());

    let env_a = tokio::time::timeout(Duration::from_secs(2), a.recv())
        .await
        .unwrap()
        .unwrap();
    let env_b = tokio::time::timeout(Duration::from_secs(2), b.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(env_a, env_b);

    drop(a);
    drop(b);
    let _ = tokio::time::timeout(Duration::from_secs(2), runner).await;
}

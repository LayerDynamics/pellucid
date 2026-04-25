//! Smoke test for T0.5 — proves that the Rust dev-tooling stack
//! (`proptest`, `wiremock`, `tokio` test runtime) is wired correctly so
//! every subsequent crate that needs them can rely on the workspace
//! dependency aliases.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn wiremock_can_bind_and_serve() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;

    let url = format!("{}/ping", server.uri());
    let body = reqwest::get(&url)
        .await
        .expect("request")
        .text()
        .await
        .expect("body");

    assert_eq!(body, "pong");
    assert!(server.uri().starts_with("http://"));
}

proptest! {
    /// Property: FNV-style fold over arbitrary byte slices is deterministic.
    /// This is a sanity check that proptest itself is wired and the workspace
    /// allows compiling property-test bodies. Real FNV lives in `pellucid-core`
    /// proper at T1.1.
    #[test]
    fn deterministic_fold_smoke(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        let a = bytes.iter().fold(0u64, |acc, &b| acc.wrapping_mul(31).wrapping_add(u64::from(b)));
        let b = bytes.iter().fold(0u64, |acc, &b| acc.wrapping_mul(31).wrapping_add(u64::from(b)));
        prop_assert_eq!(a, b);
    }
}

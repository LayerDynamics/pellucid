//! `/metrics` Prometheus exposition.
//!
//! The relay records counters via the `metrics` crate's
//! global recorder (the seeders' M3-fix counters fire here,
//! the proxy middleware emits per-route counters, the AIS
//! task increments connect / disconnect counters). This
//! module wires a [`PrometheusBuilder`] and mounts the
//! `/metrics` Axum route that scrapers hit.

use std::sync::OnceLock;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use thiserror::Error;

/// Process-wide cache of the installed handle. Multiple test
/// fixtures share one process, and the global recorder can
/// only be installed once — caching the handle lets every
/// caller render the same metrics.
static INSTALLED_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// HTTP path the metrics endpoint binds.
pub const METRICS_PATH: &str = "/metrics";

/// Errors building the recorder can surface.
#[derive(Debug, Error)]
pub enum MetricsError {
    /// The exporter recorder failed to install.
    #[error("install metrics recorder: {0}")]
    Install(String),
}

/// State passed to the `/metrics` handler — owns the
/// `PrometheusHandle` that renders on every request.
#[derive(Clone, Debug)]
pub struct MetricsState {
    /// Handle into the recorder. Cloning is cheap.
    pub handle: PrometheusHandle,
}

/// Install the Prometheus recorder + return its handle.
///
/// Uses `PrometheusBuilder::install_recorder()` which only
/// builds + installs the in-process recorder (no TCP
/// listener). We expose the rendered exposition via our own
/// Axum route on the relay's main port.
///
/// The handle is cached in `INSTALLED_HANDLE` so that every
/// caller in the process renders the same registry. The
/// global `metrics::Recorder` slot can only be written once,
/// so once the first caller installs successfully every
/// subsequent caller must reuse that handle (otherwise a
/// detached `build_recorder()` returns a handle that doesn't
/// see counters fired against the global recorder).
///
/// # Errors
/// See [`MetricsError`].
pub fn install_recorder() -> Result<PrometheusHandle, MetricsError> {
    let handle = INSTALLED_HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .unwrap_or_else(|_already_installed| {
                // The global slot was claimed by something else
                // (e.g. another test harness installed its own
                // recorder before us). Fall back to a detached
                // recorder so the caller still gets a renderable
                // handle — counters fired against the global
                // recorder won't be visible to this handle, but
                // that's the price of losing the install race.
                PrometheusBuilder::new().build_recorder().handle()
            })
    });
    Ok(handle.clone())
}

/// Mount the `/metrics` route.
pub fn metrics_router(state: MetricsState) -> Router {
    Router::new()
        .route(METRICS_PATH, get(metrics_handler))
        .with_state(state)
}

async fn metrics_handler(State(state): State<MetricsState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.handle.render(),
    )
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn install_recorder_returns_renderable_handle() {
        // Ignore the duplicate-install error — the global
        // recorder is process-wide and tests share it.
        let handle = install_recorder().unwrap();
        ::metrics::counter!("pellucid_test_counter").increment(7);
        let rendered = handle.render();
        assert!(
            rendered.contains("pellucid_test_counter"),
            "expected counter in rendered exposition; got: {rendered}",
        );
    }

    #[tokio::test]
    async fn metrics_route_returns_text_plain_body() {
        let handle = install_recorder().unwrap();
        ::metrics::counter!("pellucid_route_test").increment(3);
        let app = metrics_router(MetricsState { handle });
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(METRICS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string());
        assert_eq!(ct.as_deref(), Some("text/plain; version=0.0.4"));
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let s = String::from_utf8(body.to_vec()).unwrap();
        assert!(s.contains("pellucid_route_test"));
    }
}

//! pellucid-workers — async task workers spawned by `pellucid-relay-bin`.
//!
//! Three worker loops, parity with `worldmonitor/scripts/{scenario-worker,
//! process-deep-forecast-tasks, process-simulation-tasks}.mjs`:
//!
//! - [`scenario`] — supply-chain scenario engine. Computes the impact of a
//!   pre-defined disruption template across countries / HS2 sectors using
//!   chokepoint exposure data cached in Redis.
//! - [`deep_forecast`] — long-running forecast generation tasks. The
//!   computation itself is supplied by a caller-provided
//!   [`deep_forecast::DeepForecastDriver`] trait implementation; this crate
//!   owns the queue plumbing only (Upstash Redis BLMOVE → driver → SETEX
//!   result). The driver lives close to the LLM stack so this crate stays
//!   free of `pellucid-ml`.
//! - [`simulation`] — simulation tasks. Same shape as `deep_forecast`.
//!
//! All three workers share a single Upstash Redis REST client
//! ([`redis::RedisClient`]) and a single error type ([`WorkerError`]).
//!
//! # Example
//! ```no_run
//! use pellucid_workers::{redis::RedisClient, scenario};
//!
//! # async fn ex() -> Result<(), pellucid_workers::WorkerError> {
//! let redis = RedisClient::from_env()?;
//! scenario::run_worker(&redis, scenario::WorkerOptions::default()).await?;
//! # Ok(()) }
//! ```

pub mod deep_forecast;
pub mod error;
pub mod redis;
pub mod scenario;
pub mod simulation;

pub use error::WorkerError;

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty(), "version must not be empty");
        assert!(v.contains('.'), "expected semver with dot, got {v}");
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}

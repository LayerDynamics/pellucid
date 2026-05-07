//! Shared error type for every worker loop.

use thiserror::Error;

/// Failure modes for the workers crate.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// `UPSTASH_REDIS_REST_URL` / `UPSTASH_REDIS_REST_TOKEN` missing or empty.
    #[error("redis credentials missing: {0}")]
    MissingCredentials(&'static str),

    /// Upstream HTTP failure (transport-level).
    #[error("redis transport: {0}")]
    Transport(#[from] reqwest::Error),

    /// Upstash REST API returned non-2xx.
    #[error("redis http {status}: {body}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Truncated response body for diagnostics.
        body: String,
    },

    /// Redis returned a result whose JSON shape was unexpected.
    #[error("redis decode: {0}")]
    Decode(String),

    /// Worker payload failed validation.
    #[error("invalid job: {0}")]
    InvalidJob(String),

    /// Driver-supplied error (e.g. LLM call failure inside a deep_forecast
    /// driver). The worker uses this to write a `failed` result row and
    /// move on to the next job rather than abort the loop.
    #[error("driver: {0}")]
    Driver(String),
}

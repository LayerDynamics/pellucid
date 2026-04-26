//! 14-stage Tower middleware pipeline.
//!
//! Each `pub mod` in this directory implements one stage from
//! SPEC-001 §8.3. Stages are exposed as middleware functions
//! (`async fn(State, Request, Next) -> Response`) so the
//! `axum::middleware::from_fn_with_state` adapter can compose them
//! without writing custom `Layer` / `Service` trait implementations.

pub mod api_key;
pub mod cache_control;
pub mod clerk_session;
pub mod cors;
pub mod endpoint_rate;
pub mod entitlement;
pub mod etag;
pub mod global_rate;
pub mod handler_boundary;
pub mod header_merge;
pub mod origin;
pub mod preflight;
pub mod tier_gate;

pub use api_key::api_key;
pub use cache_control::cache_control;
pub use clerk_session::clerk_session;
pub use cors::cors_merge;
pub use endpoint_rate::endpoint_rate;
pub use entitlement::entitlement;
pub use etag::etag;
pub use global_rate::global_rate;
pub use handler_boundary::handler_boundary;
pub use header_merge::header_merge;
pub use origin::origin_allow_list;
pub use preflight::options_preflight;
pub use tier_gate::tier_gate;

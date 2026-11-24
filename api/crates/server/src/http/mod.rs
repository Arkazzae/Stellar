//! Everything between the socket and the handlers: the pipeline every request
//! goes through, the way JSON and problems are written, and the types a group
//! of endpoints is built from.

mod body;
mod client_gate;
mod cors;
mod error;
mod forwarded;
mod json;
mod metrics;
mod pipeline;
mod problem;
mod rate_limit;
mod routes;
pub mod trace;

pub use body::{MAX_REQUEST_BODY_BYTES, read_all};
pub use client_gate::{ClientVersion, KIND_HEADER, VERSION_HEADER};
pub use error::{ApiError, ApiResult};
pub use forwarded::ClientInfo;
pub use json::{Json, json_response};
pub use metrics::Metrics;
pub use pipeline::build;
pub use problem::Problem;
pub use rate_limit::{AttemptLimiter, Lease, RateLimitPolicy, RateLimiter};
pub use routes::{Endpoints, Rules};

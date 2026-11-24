//! Everything between the socket and the handlers: the pipeline every request
//! goes through, the way JSON and problems are written, and the types a group
//! of endpoints is built from.

mod body;
mod client_gate;
mod cors;
mod error;
mod extract;
pub mod file;
mod forwarded;
mod http_date;
mod json;
mod metrics;
pub mod multipart;
mod path;
mod pipeline;
mod problem;
mod query_values;
mod rate_limit;
mod routes;
mod session_cookie;
pub mod stream;
pub mod trace;

pub use body::{MAX_REQUEST_BODY_BYTES, read_all};
pub use client_gate::{ClientVersion, KIND_HEADER, VERSION_HEADER};
pub use error::{ApiError, ApiResult};
pub use extract::{MaybeCaller, Origin, Path, Query};
pub use forwarded::ClientInfo;
pub use json::{Json, json_response};
pub use metrics::Metrics;
pub use pipeline::build;
pub use problem::Problem;
pub use query_values::{QueryValues, parse_int, parse_long};
pub use rate_limit::{AttemptLimiter, Lease, RateLimitPolicy, RateLimiter};
pub use routes::{DEFAULT_JSON_BODY_BYTES, Endpoints, Rules};
pub use session_cookie::{keep_out_of_caches, with_cookie, with_session_cookie};

//! The HTTP client every outbound call of the API goes through.
//!
//! HTTP/1.1 over TCP or rustls with a bounded connection pool. The client
//! keeps no cookies and reads no proxy settings from the environment.
//! Answers compressed with `gzip` or `deflate` are decoded unless the
//! request opts out.
//! What each request states for itself is the method, the URL, the headers,
//! a body, two timeouts, a ceiling on the size of the answer, how many
//! redirects are followed, which addresses may be connected to and whether
//! compression is offered.
//!
//! Nothing here logs a URL's path or query or a header value, and no error
//! shows one: keys and tokens travel there.
//!
//! ```no_run
//! # async fn demo() -> stellarr_outbound::Result<()> {
//! use stellarr_outbound::{AddressPolicy, Client, Request};
//!
//! let client = Client::new();
//! let answer = client
//!     .send_collected(
//!         Request::get("https://example.com/")?
//!             .max_redirects(3)
//!             .policy(AddressPolicy::PublicOnly),
//!     )
//!     .await?;
//! assert!(answer.status.is_success());
//! # Ok(())
//! # }
//! ```

mod address;
mod body;
mod client;
mod connect;
mod encoding;
mod error;
mod pool;
pub mod tls;
mod url;

/// The `http` types a caller needs to build a request and read an answer,
/// so that it does not have to depend on the crate itself.
pub use http::header::{self, HeaderMap, HeaderName, HeaderValue};
pub use http::{Method, StatusCode};

pub use address::{AddressPolicy, is_loopback, is_public};
pub use body::{Body, ResponseBody};
pub use client::{Client, Collected, Request, Response};
pub use connect::{Resolve, SystemResolver, TableResolver};
pub use error::{Error, Result};
pub use url::{Scheme, Url};

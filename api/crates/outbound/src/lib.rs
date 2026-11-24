//! The HTTP client every outbound call of the API goes through.
//!
//! Small on purpose: HTTP/1.1 over TCP or rustls, one connection per
//! request, no cookies, no proxy from the environment, no compression yet.
//! What each request states for itself is the method, the URL, the headers,
//! a body, two timeouts, a ceiling on the size of the answer, how many
//! redirects are followed and which addresses may be connected to.
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
pub mod tls;
mod url;

pub use address::{AddressPolicy, is_loopback, is_public};
pub use body::{Body, ResponseBody};
pub use client::{Client, Collected, Request, Response};
pub use connect::{Resolve, SystemResolver, TableResolver};
pub use error::{Error, Result};
pub use url::{Scheme, Url};

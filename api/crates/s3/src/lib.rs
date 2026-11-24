//! The S3 calls Stellarr makes, as requests to send and answers to read.
//!
//! Nothing here touches the network. A [`Client`] turns a call into a
//! [`Prepared`] request, signed with Signature Version 4, for the application's
//! own HTTP client to send; the `read_*` functions turn the status, headers and
//! body that come back into a result or a classified [`Error`].
//!
//! The calls are the ones the application makes: list buckets and create one,
//! put, get (whole or a range), head and delete an object, list a bucket page
//! by page, and a multipart upload for big files. Addressing is path-style, the
//! form MinIO and R2 accept without wildcard DNS.
//!
//! # How a body is signed
//!
//! S3 wants to know whether the payload is covered by the signature. [`Payload`]
//! offers the three ways, and each call that carries object bytes takes one:
//!
//! * [`Payload::Hashed`] signs the SHA-256 of the bytes. It needs the hash
//!   before the first byte goes out, so it suits what is already in memory and
//!   a file that is read twice.
//! * [`Payload::Unsigned`] signs `UNSIGNED-PAYLOAD`. Integrity is left to TLS,
//!   which is why it is refused for an `http` endpoint. It is the right choice
//!   for streaming a file to R2 or to any `https` store.
//! * [`Payload::Chunked`] sends the body as `aws-chunked`, each chunk carrying a
//!   signature that covers the one before. It protects a body of known length
//!   without reading it twice, and is the way to stream to a plain `http` store.
//!
//! # What the caller has to do
//!
//! Send exactly the headers in [`Prepared::headers`] plus `Host` as in
//! [`Prepared::host`], the URL as it is (no re-encoding or normalising of the
//! path), and the body as [`Prepared::body`] says. Do not add `x-amz-*` headers.

mod client;
mod encode;
mod error;
mod read;
mod sign;
mod time;
pub mod xml;

pub use client::{
    ByteRange, Client, Config, ConfigError, ListRequest, MAX_PARTS, MIN_PART_SIZE,
    MULTIPART_THRESHOLD, Method, Part, Payload, Prepared, RequestBody, RequestError, UploadPlan,
};
pub use error::{Error, ErrorKind, MAX_ERROR_BODY};
pub use read::{
    GetHead, ListPage, MAX_LIST_BODY, ObjectHead, ObjectInfo, parse_content_range,
    read_abort_multipart_upload, read_complete_multipart_upload, read_create_bucket,
    read_create_multipart_upload, read_delete_object, read_get_object_head, read_head_object,
    read_list_buckets, read_list_objects, read_put_object, read_upload_part,
};
pub use sign::{ChunkError, ChunkSigner, Credentials, MIN_CHUNK, chunked_length, sha256};
pub use time::Timestamp;

/// Response headers, looked up by name without regard to case.
pub trait Headers {
    fn get(&self, name: &str) -> Option<&str>;

    /// Every header as a name and a value.
    fn pairs(&self) -> Vec<(&str, &str)>;
}

impl<N: AsRef<str>, V: AsRef<str>> Headers for [(N, V)] {
    fn get(&self, name: &str) -> Option<&str> {
        self.iter()
            .find(|(n, _)| n.as_ref().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_ref())
    }

    fn pairs(&self) -> Vec<(&str, &str)> {
        self.iter().map(|(n, v)| (n.as_ref(), v.as_ref())).collect()
    }
}

impl<N: AsRef<str>, V: AsRef<str>> Headers for Vec<(N, V)> {
    fn get(&self, name: &str) -> Option<&str> {
        Headers::get(self.as_slice(), name)
    }

    fn pairs(&self) -> Vec<(&str, &str)> {
        self.as_slice().pairs()
    }
}

/// Pieces of the signing machinery the proof tests reach.
#[doc(hidden)]
pub mod internals {
    pub use crate::encode::{decode, encode_component, encode_path, query_strings};
    pub use crate::sign::{Request, Scope, Signed, hex, sign, signing_key};
}

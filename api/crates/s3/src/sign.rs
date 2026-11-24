//! AWS Signature Version 4 for request headers, and the chunk signatures of a
//! streamed body.
//!
//! Query-string signing is left out on purpose: the application never hands a
//! signed link to anyone.

use aws_lc_rs::{digest, hmac};

pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The value of `x-amz-content-sha256` for a body that is not hashed.
pub const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

/// The value of `x-amz-content-sha256` for a body sent in signed chunks.
pub const STREAMING_PAYLOAD: &str = "STREAMING-AWS4-HMAC-SHA256-PAYLOAD";

/// SHA-256 of nothing, the payload hash of every request without a body.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// An access key and its secret. The secret never reaches `Debug` output.
#[derive(Clone)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .field("secret_key", &"<hidden>")
            .finish()
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(digest::digest(&digest::SHA256, data).as_ref());
    out
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        out.push(char::from(b"0123456789abcdef"[usize::from(byte & 15)]));
    }
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut out = [0u8; 32];
    out.copy_from_slice(hmac::sign(&key, data).as_ref());
    out
}

/// The key derived for one day, region and service, so the secret itself never
/// signs a request.
pub fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let date_key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, service.as_bytes());
    hmac_sha256(&service_key, b"aws4_request")
}

/// Everything about a request that the signature covers, except the secret.
pub struct Request<'a> {
    pub method: &'a str,
    /// The path exactly as it is sent, already percent-encoded.
    pub path: &'a str,
    /// The query exactly as it is sent, already encoded and sorted.
    pub query: &'a str,
    /// Every header to sign, `host` included. Names in any case.
    pub headers: &'a [(&'a str, &'a str)],
    /// The hex hash, or one of the magic values, as sent in `x-amz-content-sha256`.
    pub payload_hash: &'a str,
}

pub struct Scope<'a> {
    pub amz_date: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

impl Scope<'_> {
    fn date(&self) -> &str {
        self.amz_date.get(..8).unwrap_or_default()
    }

    fn text(&self) -> String {
        format!(
            "{}/{}/{}/aws4_request",
            self.date(),
            self.region,
            self.service
        )
    }
}

/// What signing produced. The canonical request and the string to sign are kept
/// so a test can compare them with the published text.
#[derive(Debug)]
pub struct Signed {
    pub authorization: String,
    pub signature: String,
    pub signed_headers: String,
    pub canonical_request: String,
    pub string_to_sign: String,
    signing_key: [u8; 32],
}

pub fn sign(credentials: &Credentials, scope: &Scope<'_>, request: &Request<'_>) -> Signed {
    let (canonical_headers, signed_headers) = canonical_headers(request.headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method,
        request.path,
        request.query,
        canonical_headers,
        signed_headers,
        request.payload_hash
    );
    let string_to_sign = format!(
        "{ALGORITHM}\n{}\n{}\n{}",
        scope.amz_date,
        scope.text(),
        hex(&sha256(canonical_request.as_bytes()))
    );
    let key = signing_key(
        &credentials.secret_key,
        scope.date(),
        scope.region,
        scope.service,
    );
    let signature = hex(&hmac_sha256(&key, string_to_sign.as_bytes()));
    let authorization = format!(
        "{ALGORITHM} Credential={}/{}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key,
        scope.text()
    );
    Signed {
        authorization,
        signature,
        signed_headers,
        canonical_request,
        string_to_sign,
        signing_key: key,
    }
}

/// Lower-case names, values trimmed with inner runs of blanks folded to one
/// space, sorted by name; a name given twice has its values joined with commas.
fn canonical_headers(headers: &[(&str, &str)]) -> (String, String) {
    let mut lines: Vec<(String, String)> = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        let value = fold_blanks(value);
        match lines.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, joined)) => {
                joined.push(',');
                joined.push_str(&value);
            }
            None => lines.push((name, value)),
        }
    }
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    let mut canonical = String::new();
    let mut names = String::new();
    for (name, value) in &lines {
        canonical.push_str(name);
        canonical.push(':');
        canonical.push_str(value);
        canonical.push('\n');
        if !names.is_empty() {
            names.push(';');
        }
        names.push_str(name);
    }
    (canonical, names)
}

fn fold_blanks(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_blank = false;
    for ch in value.trim_matches([' ', '\t']).chars() {
        if ch == ' ' || ch == '\t' {
            pending_blank = true;
        } else {
            if pending_blank {
                out.push(' ');
                pending_blank = false;
            }
            out.push(ch);
        }
    }
    out
}

/// The smallest chunk S3 accepts, except for the last one.
pub const MIN_CHUNK: usize = 8 * 1024;

/// A problem with the way chunks were fed to a [`ChunkSigner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkError {
    /// More bytes than the length announced in the headers.
    TooMuch,
    /// A chunk other than the last is not exactly the chunk size, or the chunk is empty.
    WrongSize,
    /// The body ended before the announced length was reached.
    TooLittle,
    /// Chunks after the closing one.
    Finished,
}

/// Signs a body sent as `aws-chunked`: every chunk carries a signature that
/// covers the signature of the chunk before it, so a body is protected without
/// being read twice or held in memory.
pub struct ChunkSigner {
    key: [u8; 32],
    amz_date: String,
    scope: String,
    previous: String,
    chunk_size: usize,
    remaining: u64,
    finished: bool,
}

impl std::fmt::Debug for ChunkSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkSigner")
            .field("chunk_size", &self.chunk_size)
            .field("remaining", &self.remaining)
            .finish_non_exhaustive()
    }
}

impl ChunkSigner {
    /// Starts a body whose headers were signed as `seed`. Requests come with
    /// their signer already made; this is public for the proof tests only.
    #[doc(hidden)]
    pub fn new(seed: &Signed, scope: &Scope<'_>, decoded_length: u64, chunk_size: usize) -> Self {
        Self {
            key: seed.signing_key,
            amz_date: scope.amz_date.to_owned(),
            scope: scope.text(),
            previous: seed.signature.clone(),
            chunk_size,
            remaining: decoded_length,
            finished: false,
        }
    }

    /// The chunk size the content length was computed for.
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Bytes of the object still to be fed in.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Appends the next chunk of the object to `out`. Every chunk but the last
    /// must be exactly [`chunk_size`](Self::chunk_size) bytes.
    pub fn chunk(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<(), ChunkError> {
        if self.finished {
            return Err(ChunkError::Finished);
        }
        let length = data.len() as u64;
        if length > self.remaining {
            return Err(ChunkError::TooMuch);
        }
        if data.is_empty() || (length != self.chunk_size as u64 && length != self.remaining) {
            return Err(ChunkError::WrongSize);
        }
        self.remaining -= length;
        self.write(data, out);
        Ok(())
    }

    /// Appends the closing zero-length chunk. Only valid once the whole object
    /// has been fed in.
    pub fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), ChunkError> {
        if self.finished {
            return Err(ChunkError::Finished);
        }
        if self.remaining != 0 {
            return Err(ChunkError::TooLittle);
        }
        self.write(&[], out);
        self.finished = true;
        Ok(())
    }

    fn write(&mut self, data: &[u8], out: &mut Vec<u8>) {
        let string_to_sign = format!(
            "{ALGORITHM}-PAYLOAD\n{}\n{}\n{}\n{EMPTY_SHA256}\n{}",
            self.amz_date,
            self.scope,
            self.previous,
            hex(&sha256(data))
        );
        let signature = hex(&hmac_sha256(&self.key, string_to_sign.as_bytes()));
        out.extend_from_slice(
            format!("{:x};chunk-signature={signature}\r\n", data.len()).as_bytes(),
        );
        out.extend_from_slice(data);
        out.extend_from_slice(b"\r\n");
        self.previous = signature;
    }
}

/// The `Content-Length` of a body sent in chunks of `chunk_size`, signature
/// framing included.
pub fn chunked_length(decoded: u64, chunk_size: usize) -> u64 {
    // Frame: "<hex length>;chunk-signature=<64 hex>\r\n" + data + "\r\n".
    fn overhead(data_length: u64) -> u64 {
        let digits = if data_length == 0 {
            1
        } else {
            u64::from((64 - data_length.leading_zeros()).div_ceil(4))
        };
        digits + ";chunk-signature=".len() as u64 + 64 + 2 + 2
    }
    let size = chunk_size as u64;
    let full = decoded / size;
    let rest = decoded % size;
    let mut total = decoded + full * overhead(size) + overhead(0);
    if rest > 0 {
        total += overhead(rest);
    }
    total
}

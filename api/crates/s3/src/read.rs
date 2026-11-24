//! Reading the answers.
//!
//! Every function takes the status, the headers and the body that came back.
//! A refused call becomes an [`Error`] classified from the status and the error
//! document. The caller should read at most [`MAX_ERROR_BODY`] bytes of the body
//! of a refusal and [`MAX_LIST_BODY`] of a listing; whatever is longer is not
//! an answer worth parsing.

use crate::Headers;
use crate::client::ByteRange;
use crate::encode;
use crate::error::{Error, ErrorKind, MAX_ERROR_BODY};
use crate::xml::{self, Node};

/// The most of a listing page worth reading: far above a thousand keys of the
/// longest length S3 allows.
pub const MAX_LIST_BODY: usize = xml::MAX_DOCUMENT;

fn success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Parses a body that should be the document `root_name`. A document named
/// `Error` is a refusal even under a success status: some calls answer `200`
/// first and fail afterwards.
fn document<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
    root_name: &str,
) -> Result<Node, Error> {
    if !success(status) {
        return Err(Error::from_response(
            status,
            headers,
            &body[..body.len().min(MAX_ERROR_BODY)],
        ));
    }
    let root = xml::parse(body)
        .map_err(|e| Error::malformed(&format!("the answer is not a readable document: {e}")))?;
    if root.name == "Error" {
        return Err(Error::from_response(status, headers, body));
    }
    if root.name != root_name {
        return Err(Error::malformed(
            "the answer is a different document than the call expects",
        ));
    }
    Ok(root)
}

fn nothing<H: Headers + ?Sized>(status: u16, headers: &H, body: &[u8]) -> Result<(), Error> {
    if success(status) {
        Ok(())
    } else {
        Err(Error::from_response(
            status,
            headers,
            &body[..body.len().min(MAX_ERROR_BODY)],
        ))
    }
}

/// `GET /`: the names of the buckets.
pub fn read_list_buckets<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<Vec<String>, Error> {
    let root = document(status, headers, body, "ListAllMyBucketsResult")?;
    // A store with no buckets sends no `Buckets` element, or an empty one.
    Ok(root
        .child("Buckets")
        .map(|buckets| {
            buckets
                .children_named("Bucket")
                .filter_map(|b| b.child_text("Name"))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

/// `PUT /bucket`. A bucket that is already ours comes back as
/// [`ErrorKind::AlreadyOwned`], which a caller that only wants it to exist can accept.
pub fn read_create_bucket<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<(), Error> {
    nothing(status, headers, body)
}

/// `PUT /bucket/key`: the `ETag` the store gave the object, when it said.
pub fn read_put_object<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<Option<String>, Error> {
    nothing(status, headers, body)?;
    Ok(headers.get("etag").map(str::to_owned))
}

/// `DELETE /bucket/key`. An object that is not there is already deleted, so
/// that is success; a missing bucket is not.
pub fn read_delete_object<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<(), Error> {
    match nothing(status, headers, body) {
        Err(error) if error.kind == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// What a `HEAD` says about an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectHead {
    pub length: u64,
    pub etag: Option<String>,
    /// User metadata: the `x-amz-meta-` suffix in lower case and its value.
    pub metadata: Vec<(String, String)>,
}

impl ObjectHead {
    /// The value of user metadata `name`, found without regard to case.
    pub fn metadata_value(&self, name: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

const MAX_METADATA_HEADERS: usize = 64;
const MAX_METADATA_VALUE: usize = 4096;

/// `HEAD /bucket/key`. A refusal has no body, so it is classified from the
/// status alone: a `404` is [`ErrorKind::NotFound`] whether the key or the
/// bucket is missing.
pub fn read_head_object<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
) -> Result<ObjectHead, Error> {
    nothing(status, headers, &[])?;
    let length = parse_length(headers)
        .ok_or_else(|| Error::malformed("the answer has no usable content-length"))?;
    let mut metadata = Vec::new();
    for (name, value) in headers.pairs() {
        let Some(suffix) = name
            .get(..11)
            .filter(|p| p.eq_ignore_ascii_case("x-amz-meta-"))
            .map(|_| &name[11..])
        else {
            continue;
        };
        if metadata.len() >= MAX_METADATA_HEADERS || value.len() > MAX_METADATA_VALUE {
            return Err(Error::malformed("too much user metadata"));
        }
        metadata.push((suffix.to_ascii_lowercase(), value.trim().to_owned()));
    }
    Ok(ObjectHead {
        length,
        etag: headers.get("etag").map(str::to_owned),
        metadata,
    })
}

/// The head of a `GET` answer, enough to start streaming the body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetHead {
    /// Bytes in this answer: the slice for a range, the object otherwise.
    pub length: u64,
    /// Bytes in the whole object.
    pub total_length: u64,
    /// The slice that is being sent, or `None` when it is the whole object.
    /// A store that ignores `Range` answers `200`, which shows up as `None`
    /// for a call that asked for a range.
    pub range: Option<ByteRange>,
    pub etag: Option<String>,
}

/// `GET /bucket/key`, from the status and headers alone, so the body can be
/// streamed. For a refusal, call [`Error::from_response`] with the body if the
/// code is wanted; the status already gives [`ErrorKind::NotFound`] for `404`
/// and [`ErrorKind::RangeNotSatisfiable`] for `416`.
pub fn read_get_object_head<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
) -> Result<GetHead, Error> {
    nothing(status, headers, &[])?;
    let etag = headers.get("etag").map(str::to_owned);
    match status {
        200 => {
            let length = parse_length(headers)
                .ok_or_else(|| Error::malformed("the answer has no usable content-length"))?;
            Ok(GetHead {
                length,
                total_length: length,
                range: None,
                etag,
            })
        }
        206 => {
            let (range, total) = headers
                .get("content-range")
                .and_then(parse_content_range)
                .ok_or_else(|| {
                    Error::malformed("a partial answer without a usable content-range")
                })?;
            let served = range.to.map_or(0, |to| to - range.from + 1);
            // Trust the range over the length only if they agree.
            if parse_length(headers).is_some_and(|length| length != served) {
                return Err(Error::malformed(
                    "content-length and content-range disagree",
                ));
            }
            Ok(GetHead {
                length: served,
                total_length: total,
                range: Some(range),
                etag,
            })
        }
        _ => Err(Error::malformed(
            "a success status other than 200 or 206 for a read",
        )),
    }
}

fn parse_length<H: Headers + ?Sized>(headers: &H) -> Option<u64> {
    let text = headers.get("content-length")?.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Reads `bytes from-to/total`. A range outside its object, or one the store
/// could not size (`*`), is refused.
pub fn parse_content_range(value: &str) -> Option<(ByteRange, u64)> {
    let unit = value.get(..6)?;
    if !unit.eq_ignore_ascii_case("bytes ") {
        return None;
    }
    let payload = &value[6..];
    let (span, total) = payload.split_once('/')?;
    let (from, to) = span.split_once('-')?;
    let number = |text: &str| -> Option<u64> {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let (from, to, total) = (number(from)?, number(to)?, number(total)?);
    (to >= from && total > to).then_some((ByteRange { from, to: Some(to) }, total))
}

/// One object of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub key: String,
    pub size: u64,
}

/// One page of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPage {
    pub objects: Vec<ObjectInfo>,
    /// The token for the next page, when there is one. A truncated page
    /// without a token is refused rather than followed forever.
    pub next_token: Option<String>,
}

/// `GET /bucket?list-type=2`, requested with `encoding-type=url`.
pub fn read_list_objects<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<ListPage, Error> {
    let root = document(status, headers, body, "ListBucketResult")?;
    let url_encoded = match root.child_text("EncodingType").map(str::trim) {
        None | Some("") => false,
        Some("url") => true,
        Some(_) => {
            return Err(Error::malformed(
                "the listing uses an encoding that was not asked for",
            ));
        }
    };
    let mut objects = Vec::new();
    for entry in root.children_named("Contents") {
        let raw = entry
            .child_text("Key")
            .ok_or_else(|| Error::malformed("a listed object has no key"))?;
        let key = if url_encoded {
            encode::decode(raw, true)
                .ok_or_else(|| Error::malformed("a listed key is not valid URL encoding"))?
        } else {
            raw.to_owned()
        };
        if key.is_empty() {
            return Err(Error::malformed("a listed object has an empty key"));
        }
        let size = entry
            .child_text("Size")
            .map(str::trim)
            .and_then(|size| {
                (!size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| size.parse::<u64>().ok())
                    .flatten()
            })
            .ok_or_else(|| Error::malformed("a listed object has no usable size"))?;
        objects.push(ObjectInfo { key, size });
    }
    let truncated = match root.child_text("IsTruncated").map(str::trim) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(Error::malformed("IsTruncated is neither true nor false")),
    };
    let next_token = root
        .child_text("NextContinuationToken")
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    if truncated && next_token.is_none() {
        return Err(Error::malformed(
            "a truncated listing without a continuation token",
        ));
    }
    Ok(ListPage {
        objects,
        next_token: if truncated { next_token } else { None },
    })
}

/// `POST /bucket/key?uploads`: the id of the new upload.
pub fn read_create_multipart_upload<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<String, Error> {
    let root = document(status, headers, body, "InitiateMultipartUploadResult")?;
    match root.child_text("UploadId").map(str::trim) {
        Some(id) if !id.is_empty() && id.len() <= 1024 => Ok(id.to_owned()),
        _ => Err(Error::malformed("the answer has no usable upload id")),
    }
}

/// `PUT ...?partNumber=N&uploadId=...`: the `ETag` of the part, which
/// completing the upload needs.
pub fn read_upload_part<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<String, Error> {
    nothing(status, headers, body)?;
    match headers.get("etag").map(str::trim) {
        Some(etag) if !etag.is_empty() && etag.len() <= 256 => Ok(etag.to_owned()),
        _ => Err(Error::malformed("the answer to a part has no etag")),
    }
}

/// `POST /bucket/key?uploadId=...`: the `ETag` of the finished object. A
/// `200` whose body is an error document is a failure, and one with no body at
/// all is not a success either.
pub fn read_complete_multipart_upload<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<String, Error> {
    let root = document(status, headers, body, "CompleteMultipartUploadResult")?;
    match root.child_text("ETag").map(str::trim) {
        Some(etag) if !etag.is_empty() && etag.len() <= 256 => Ok(etag.to_owned()),
        _ => Err(Error::malformed("the answer has no etag")),
    }
}

/// `DELETE ...?uploadId=...`. An upload that is gone comes back as
/// [`ErrorKind::UploadNotFound`], which a cleanup can ignore.
pub fn read_abort_multipart_upload<H: Headers + ?Sized>(
    status: u16,
    headers: &H,
    body: &[u8],
) -> Result<(), Error> {
    nothing(status, headers, body)
}

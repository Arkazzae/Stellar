//! Reading answers: the cases the application's storage tests cover, and what
//! a hostile or broken store can send.

use stellarr_s3::{
    ByteRange, ErrorKind, MAX_ERROR_BODY, parse_content_range, read_abort_multipart_upload,
    read_complete_multipart_upload, read_create_bucket, read_create_multipart_upload,
    read_delete_object, read_get_object_head, read_head_object, read_list_buckets,
    read_list_objects, read_put_object, read_upload_part,
};

type H = [(&'static str, &'static str)];
const NONE: &H = &[];

fn error(code: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{code}</Code><Message>m</Message><RequestId>4442587FB7D0A2F9</RequestId></Error>"
    )
}

// --- Content-Range ---

#[test]
fn reads_the_range_actually_served() {
    for (text, from, to, total) in [
        ("bytes 0-99/100", 0, 99, 100),
        ("bytes 100-199/4096", 100, 199, 4096),
        ("BYTES 5-5/6", 5, 5, 6),
    ] {
        let (range, got_total) = parse_content_range(text).unwrap();
        assert_eq!(range, ByteRange { from, to: Some(to) }, "{text}");
        assert_eq!(got_total, total);
    }
}

#[test]
fn rejects_malformed_content_ranges() {
    for text in [
        "",
        "items 0-9/10",
        "bytes 10-9/100",
        "bytes 0-100/100",
        "bytes */100",
        "bytes 0-9/*",
        "bytes 0-9",
        "bytes -5/10",
        "bytes 0--9/10",
        "bytes 0-9/10/11",
        "bytes a-b/c",
        "bytes 0-9/-1",
        "bytes +0-9/10",
        "bytes 0-99999999999999999999/99999999999999999999",
        "bytes  0-9/10",
        "bytes\u{e9}0-9/10",
        "byt",
        "bytes ",
    ] {
        assert_eq!(parse_content_range(text), None, "{text:?}");
    }
}

// --- GET ---

#[test]
fn a_whole_object_and_a_slice() {
    let whole =
        read_get_object_head(200, &[("Content-Length", "10"), ("ETag", "\"x\"")][..]).unwrap();
    assert_eq!(
        (whole.length, whole.total_length, whole.range),
        (10, 10, None)
    );

    // A range past the end is clamped by the store: bytes=7-100 of ten bytes.
    let slice = read_get_object_head(
        206,
        &[("content-length", "3"), ("content-range", "bytes 7-9/10")][..],
    )
    .unwrap();
    assert_eq!(
        (slice.length, slice.total_length, slice.range),
        (
            3,
            10,
            Some(ByteRange {
                from: 7,
                to: Some(9)
            })
        )
    );
}

#[test]
fn a_partial_answer_must_be_consistent() {
    for headers in [
        &[("content-length", "3")][..],
        &[("content-length", "3"), ("content-range", "bytes */10")][..],
        &[("content-length", "4"), ("content-range", "bytes 7-9/10")][..],
        &[("content-length", "3"), ("content-range", "bytes 7-10/10")][..],
    ] {
        assert_eq!(
            read_get_object_head(206, headers).unwrap_err().kind,
            ErrorKind::Malformed,
            "{headers:?}"
        );
    }
    assert_eq!(
        read_get_object_head(200, NONE).unwrap_err().kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        read_get_object_head(200, &[("content-length", "-1")][..])
            .unwrap_err()
            .kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        read_get_object_head(200, &[("content-length", "1e3")][..])
            .unwrap_err()
            .kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        read_get_object_head(204, NONE).unwrap_err().kind,
        ErrorKind::Malformed
    );
}

#[test]
fn a_refusal_becomes_the_error_every_store_raises() {
    for (status, kind) in [
        (404, ErrorKind::NotFound),
        (416, ErrorKind::RangeNotSatisfiable),
        (500, ErrorKind::Unavailable),
        (403, ErrorKind::AccessDenied),
    ] {
        assert_eq!(
            read_get_object_head(status, NONE).unwrap_err().kind,
            kind,
            "{status}"
        );
    }
}

// --- HEAD ---

#[test]
fn head_reads_the_length_and_user_metadata_whatever_the_case() {
    let head = read_head_object(
        200,
        &[
            ("Content-Length", "15"),
            ("ETag", "\"abc\""),
            ("X-Amz-Meta-Sha256", " abcdef "),
            ("x-amz-meta-Other", "1"),
            ("x-amz-request-id", "r"),
        ][..],
    )
    .unwrap();
    assert_eq!(head.length, 15);
    assert_eq!(head.metadata_value("sha256"), Some("abcdef"));
    assert_eq!(head.metadata_value("SHA256"), Some("abcdef"));
    assert_eq!(head.metadata_value("other"), Some("1"));
    assert_eq!(head.metadata_value("request-id"), None);
}

#[test]
fn head_failures_come_from_the_status_alone() {
    assert_eq!(
        read_head_object(404, NONE).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        read_head_object(403, NONE).unwrap_err().kind,
        ErrorKind::AccessDenied
    );
    assert_eq!(
        read_head_object(200, NONE).unwrap_err().kind,
        ErrorKind::Malformed
    );
}

#[test]
fn head_bounds_user_metadata() {
    let big = "x".repeat(5000);
    let many: Vec<(String, String)> =
        std::iter::once(("content-length".to_owned(), "1".to_owned()))
            .chain((0..100).map(|i| (format!("x-amz-meta-k{i}"), "v".into())))
            .collect();
    assert_eq!(
        read_head_object(200, &many).unwrap_err().kind,
        ErrorKind::Malformed
    );
    let long = vec![
        ("content-length".to_owned(), "1".to_owned()),
        ("x-amz-meta-k".to_owned(), big),
    ];
    assert_eq!(
        read_head_object(200, &long).unwrap_err().kind,
        ErrorKind::Malformed
    );
}

// --- PUT, DELETE, buckets ---

#[test]
fn put_returns_the_etag_and_refusals_are_classified() {
    assert_eq!(
        read_put_object(200, &[("ETag", "\"e\"")][..], b"")
            .unwrap()
            .as_deref(),
        Some("\"e\"")
    );
    assert_eq!(read_put_object(200, NONE, b"").unwrap(), None);
    let denied = read_put_object(403, NONE, error("AccessDenied").as_bytes()).unwrap_err();
    assert_eq!(denied.kind, ErrorKind::AccessDenied);
    assert_eq!(denied.code.as_deref(), Some("AccessDenied"));
    assert_eq!(denied.request_id.as_deref(), Some("4442587FB7D0A2F9"));
    assert_eq!(
        read_put_object(412, NONE, error("PreconditionFailed").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::PreconditionFailed
    );
}

#[test]
fn deleting_what_is_not_there_is_not_an_error() {
    read_delete_object(204, NONE, b"").unwrap();
    read_delete_object(200, NONE, b"").unwrap();
    read_delete_object(404, NONE, error("NoSuchKey").as_bytes()).unwrap();
    read_delete_object(404, NONE, b"").unwrap();
    assert_eq!(
        read_delete_object(404, NONE, error("NoSuchBucket").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::BucketNotFound
    );
    assert_eq!(
        read_delete_object(403, NONE, b"").unwrap_err().kind,
        ErrorKind::AccessDenied
    );
}

#[test]
fn creating_a_bucket() {
    read_create_bucket(200, NONE, b"").unwrap();
    assert_eq!(
        read_create_bucket(409, NONE, error("BucketAlreadyOwnedByYou").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::AlreadyOwned
    );
    assert_eq!(
        read_create_bucket(409, NONE, error("BucketAlreadyExists").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
}

#[test]
fn listing_buckets_including_a_fresh_store_with_none() {
    let some = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Owner><ID>o</ID></Owner><Buckets><Bucket><Name>uploads</Name><CreationDate>2013-05-24T00:00:00.000Z</CreationDate></Bucket><Bucket><Name>backups</Name></Bucket></Buckets></ListAllMyBucketsResult>";
    assert_eq!(
        read_list_buckets(200, NONE, some.as_bytes()).unwrap(),
        ["uploads", "backups"]
    );
    for empty in [
        "<ListAllMyBucketsResult><Owner><ID>o</ID></Owner><Buckets/></ListAllMyBucketsResult>",
        "<ListAllMyBucketsResult><Owner><ID>o</ID></Owner></ListAllMyBucketsResult>",
        "<ListAllMyBucketsResult><Buckets></Buckets></ListAllMyBucketsResult>",
    ] {
        assert!(
            read_list_buckets(200, NONE, empty.as_bytes())
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        read_list_buckets(403, NONE, error("AccessDenied").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::AccessDenied
    );
    assert_eq!(
        read_list_buckets(200, NONE, b"<Other/>").unwrap_err().kind,
        ErrorKind::Malformed
    );
}

// --- listings ---

fn page(entries: &str, tail: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>b</Name><Prefix></Prefix><EncodingType>url</EncodingType>{tail}{entries}</ListBucketResult>"
    )
}

fn entry(key: &str, size: &str) -> String {
    format!(
        "<Contents><Key>{key}</Key><LastModified>2013-05-24T00:00:00.000Z</LastModified><ETag>\"e\"</ETag><Size>{size}</Size><StorageClass>STANDARD</StorageClass></Contents>"
    )
}

#[test]
fn a_listing_names_every_object_by_key_and_size() {
    let body = page(
        &(entry("a/first.bin", "3")
            + &entry("b/2026/11/second.bin", "10")
            + &entry("top.bin", "0")),
        "<KeyCount>3</KeyCount><IsTruncated>false</IsTruncated>",
    );
    let got = read_list_objects(200, NONE, body.as_bytes()).unwrap();
    assert_eq!(got.next_token, None);
    let listed: Vec<(&str, u64)> = got
        .objects
        .iter()
        .map(|o| (o.key.as_str(), o.size))
        .collect();
    assert_eq!(
        listed,
        [
            ("a/first.bin", 3),
            ("b/2026/11/second.bin", 10),
            ("top.bin", 0)
        ]
    );
}

#[test]
fn keys_come_back_url_decoded_with_plus_as_space() {
    let body = page(
        &(entry("a+b%2Bc/%C3%BC%20x%25.png", "1") + &entry("tab%09x", "1") + &entry("%2F%2F", "1")),
        "<IsTruncated>false</IsTruncated>",
    );
    let keys: Vec<String> = read_list_objects(200, NONE, body.as_bytes())
        .unwrap()
        .objects
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(keys, ["a b+c/ü x%.png", "tab\tx", "//"]);
}

#[test]
fn a_store_that_does_not_encode_is_read_as_it_is() {
    let body = "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>a+b%2B</Key><Size>1</Size></Contents></ListBucketResult>";
    assert_eq!(
        read_list_objects(200, NONE, body.as_bytes())
            .unwrap()
            .objects[0]
            .key,
        "a+b%2B"
    );
}

#[test]
fn a_truncated_listing_hands_over_its_token() {
    let body = page(
        &entry("k", "1"),
        "<IsTruncated>true</IsTruncated><NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken>",
    );
    assert_eq!(
        read_list_objects(200, NONE, body.as_bytes())
            .unwrap()
            .next_token
            .as_deref(),
        Some("1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=")
    );
    // A token on the last page is ignored.
    let last = page(
        &entry("k", "1"),
        "<IsTruncated>false</IsTruncated><NextContinuationToken>t</NextContinuationToken>",
    );
    assert_eq!(
        read_list_objects(200, NONE, last.as_bytes())
            .unwrap()
            .next_token,
        None
    );
}

#[test]
fn an_empty_page_is_empty() {
    let body = page("", "<KeyCount>0</KeyCount><IsTruncated>false</IsTruncated>");
    assert!(
        read_list_objects(200, NONE, body.as_bytes())
            .unwrap()
            .objects
            .is_empty()
    );
}

#[test]
fn hostile_listings_are_refused() {
    let truncated_without_token = page(&entry("k", "1"), "<IsTruncated>true</IsTruncated>");
    let empty_token = page(
        &entry("k", "1"),
        "<IsTruncated>true</IsTruncated><NextContinuationToken></NextContinuationToken>",
    );
    let cases: Vec<(&str, String)> = vec![
        ("truncated without a token", truncated_without_token),
        ("truncated with an empty token", empty_token),
        (
            "a size past u64",
            page(&entry("k", "18446744073709551616"), ""),
        ),
        ("a negative size", page(&entry("k", "-1"), "")),
        ("a size with a unit", page(&entry("k", "1KB"), "")),
        ("no size", page("<Contents><Key>k</Key></Contents>", "")),
        ("no key", page("<Contents><Size>1</Size></Contents>", "")),
        ("an empty key", page(&entry("", "1"), "")),
        ("a bad escape in a key", page(&entry("a%zz", "1"), "")),
        (
            "a key that decodes to bad UTF-8",
            page(&entry("%ff", "1"), ""),
        ),
        (
            "IsTruncated nonsense",
            page(&entry("k", "1"), "<IsTruncated>maybe</IsTruncated>"),
        ),
        (
            "an encoding that was not asked for",
            page(&entry("k", "1"), "").replace(
                "<EncodingType>url</EncodingType>",
                "<EncodingType>base64</EncodingType>",
            ),
        ),
        (
            "an entity declaration",
            format!(
                "<!DOCTYPE x [<!ENTITY a \"aaaaaaaaaa\">]>{}",
                page(&entry("&a;", "1"), "")
            ),
        ),
        ("the wrong document", "<ListAllMyBucketsResult/>".to_owned()),
        (
            "html from a proxy",
            "<html><body>Bad Gateway</body></html>".to_owned(),
        ),
        ("plain text", "502 Bad Gateway".to_owned()),
        ("nothing", String::new()),
    ];
    for (what, body) in cases {
        let error = read_list_objects(200, NONE, body.as_bytes()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Malformed, "{what}");
    }
}

#[test]
fn an_error_document_under_a_success_status_is_an_error() {
    let error = read_list_objects(200, NONE, error("InternalError").as_bytes()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert!(error.is_retryable());
}

#[test]
fn every_truncation_of_a_listing_is_an_error_and_none_panics() {
    let body = page(
        &(entry("a%20b", "3") + &entry("c", "4")),
        "<IsTruncated>true</IsTruncated><NextContinuationToken>t</NextContinuationToken>",
    );
    for cut in 0..body.len() {
        assert!(
            read_list_objects(200, NONE, &body.as_bytes()[..cut]).is_err(),
            "cut at {cut}"
        );
    }
    assert!(read_list_objects(200, NONE, body.as_bytes()).is_ok());
}

#[test]
fn oversized_listings_are_refused_without_being_read() {
    let big = vec![b' '; stellarr_s3::MAX_LIST_BODY + 1];
    assert_eq!(
        read_list_objects(200, NONE, &big).unwrap_err().kind,
        ErrorKind::Malformed
    );
    // A page of a thousand of the longest keys is well within bounds.
    let key = "%C3%A9".repeat(300);
    let entries: String = (0..1000).map(|_| entry(&key, "1")).collect();
    let body = page(&entries, "<IsTruncated>false</IsTruncated>");
    assert_eq!(
        read_list_objects(200, NONE, body.as_bytes())
            .unwrap()
            .objects
            .len(),
        1000
    );
}

#[test]
fn billions_of_elements_are_refused() {
    let body = page(&"<Contents/>".repeat(250_000), "");
    assert_eq!(
        read_list_objects(200, NONE, body.as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::Malformed
    );
}

// --- multipart ---

#[test]
fn starting_a_multipart_upload() {
    let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>b</Bucket><Key>k</Key><UploadId>VXBsb2FkIElE</UploadId></InitiateMultipartUploadResult>";
    assert_eq!(
        read_create_multipart_upload(200, NONE, body.as_bytes()).unwrap(),
        "VXBsb2FkIElE"
    );
    for bad in [
        "<InitiateMultipartUploadResult><UploadId></UploadId></InitiateMultipartUploadResult>",
        "<InitiateMultipartUploadResult/>",
        "<Other/>",
    ] {
        assert_eq!(
            read_create_multipart_upload(200, NONE, bad.as_bytes())
                .unwrap_err()
                .kind,
            ErrorKind::Malformed
        );
    }
    let long = format!(
        "<InitiateMultipartUploadResult><UploadId>{}</UploadId></InitiateMultipartUploadResult>",
        "u".repeat(2000)
    );
    assert_eq!(
        read_create_multipart_upload(200, NONE, long.as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        read_create_multipart_upload(404, NONE, error("NoSuchBucket").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::BucketNotFound
    );
}

#[test]
fn a_part_needs_its_etag() {
    assert_eq!(
        read_upload_part(200, &[("ETag", "\"p1\"")][..], b"").unwrap(),
        "\"p1\""
    );
    assert_eq!(
        read_upload_part(200, NONE, b"").unwrap_err().kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        read_upload_part(200, &[("ETag", "")][..], b"")
            .unwrap_err()
            .kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        read_upload_part(404, NONE, error("NoSuchUpload").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::UploadNotFound
    );
    assert_eq!(
        read_upload_part(503, NONE, error("SlowDown").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::Throttled
    );
}

#[test]
fn completing_an_upload_reads_the_body_before_trusting_the_status() {
    let ok = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>l</Location><Bucket>b</Bucket><Key>k</Key><ETag>\"3858f62230ac3c915f300c664312c11f-9\"</ETag></CompleteMultipartUploadResult>";
    assert_eq!(
        read_complete_multipart_upload(200, NONE, ok.as_bytes()).unwrap(),
        "\"3858f62230ac3c915f300c664312c11f-9\""
    );

    // S3 may send 200 and fail afterwards; the failure is in the body.
    let failed =
        read_complete_multipart_upload(200, NONE, error("InternalError").as_bytes()).unwrap_err();
    assert_eq!(failed.kind, ErrorKind::Unavailable);
    assert_eq!(failed.status, 200);
    assert_eq!(
        read_complete_multipart_upload(200, NONE, error("InvalidPart").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::Rejected
    );

    // Whitespace keeps the connection alive before the document starts.
    let padded = format!("  \n \n{ok}");
    assert!(read_complete_multipart_upload(200, NONE, padded.as_bytes()).is_ok());

    // No body, a half body or the wrong document is not a success.
    for bad in [
        "",
        "   ",
        "<CompleteMultipartUploadResult><ETag>x",
        "<CompleteMultipartUploadResult/>",
        "<Other/>",
    ] {
        assert_eq!(
            read_complete_multipart_upload(200, NONE, bad.as_bytes())
                .unwrap_err()
                .kind,
            ErrorKind::Malformed,
            "{bad:?}"
        );
    }
    assert_eq!(
        read_complete_multipart_upload(400, NONE, error("EntityTooSmall").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::Rejected
    );
}

#[test]
fn aborting_an_upload() {
    read_abort_multipart_upload(204, NONE, b"").unwrap();
    assert_eq!(
        read_abort_multipart_upload(404, NONE, error("NoSuchUpload").as_bytes())
            .unwrap_err()
            .kind,
        ErrorKind::UploadNotFound
    );
}

// --- refusals in general ---

#[test]
fn a_refusal_body_is_only_read_up_to_its_bound() {
    // What lies beyond the bound is not read; a document that does not end
    // inside it is not an error document, and the status alone classifies.
    let long = format!(
        "<Error><Code>SlowDown</Code><Message>{}</Message></Error>",
        "y".repeat(MAX_ERROR_BODY)
    );
    let refused = read_put_object(404, NONE, long.as_bytes()).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::NotFound);
    assert_eq!(refused.code, None);

    // One that ends inside it is read even when the body goes on.
    let mut body = error("SlowDown").into_bytes();
    body.extend(vec![b' '; MAX_ERROR_BODY + 10]);
    assert_eq!(
        read_put_object(503, NONE, &body)
            .unwrap_err()
            .code
            .as_deref(),
        Some("SlowDown")
    );
}

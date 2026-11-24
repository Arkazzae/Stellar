//! Building requests: what is refused, and the shape of an upload.

use stellarr_s3::{
    ByteRange, Client, Config, ConfigError, Credentials, ListRequest, MIN_PART_SIZE, Part, Payload,
    RequestBody, RequestError, Timestamp, UploadPlan,
};

fn credentials() -> Credentials {
    Credentials {
        access_key: "AK".into(),
        secret_key: "SK".into(),
    }
}

fn client(endpoint: &str) -> Client {
    Client::new(Config::new(endpoint, "us-east-1", "uploads", credentials()).unwrap())
}

fn now() -> Timestamp {
    Timestamp::from_unix(1_369_353_600).unwrap()
}

#[test]
fn endpoints_are_read_strictly() {
    let ok = |endpoint: &str| Config::new(endpoint, "us-east-1", "b", credentials());
    for good in [
        "http://minio:9000",
        "https://s3.example.test",
        "https://abc.r2.cloudflarestorage.com",
        "https://host.test/",
        "https://host.test/prefix/store",
        "http://127.0.0.1:9000",
        "http://[::1]:9000",
        "HTTPS://Host.TEST:443",
    ] {
        ok(good).unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    for bad in [
        "",
        "minio:9000",
        "ftp://host",
        "https://",
        "https://user:pw@host",
        "https://host?x=1",
        "https://host#f",
        "https://host/p?x",
        "https://host:0",
        "https://host:65536",
        "https://host:abc",
        "https://host:",
        "https://-host",
        "https://ho st",
        "https://host/a%2Fb",
        "https://host/a//b",
        "https://host/../x",
        "https://host/./x",
        "https://[::1",
        "https://[]:80",
        "https://host\\x",
        "https://hö.st",
    ] {
        assert!(matches!(ok(bad), Err(ConfigError::Endpoint(_))), "{bad:?}");
    }
    assert_eq!(
        Config::new("https://h", "", "b", credentials()).unwrap_err(),
        ConfigError::Region
    );
    assert_eq!(
        Config::new("https://h", "us east", "b", credentials()).unwrap_err(),
        ConfigError::Region
    );
    assert_eq!(
        Config::new("https://h", "auto", "", credentials()).unwrap_err(),
        ConfigError::Bucket
    );
    let blank = Credentials {
        access_key: "".into(),
        secret_key: "s".into(),
    };
    assert_eq!(
        Config::new("https://h", "auto", "b", blank).unwrap_err(),
        ConfigError::Credentials
    );
    let control = Credentials {
        access_key: "a\nb".into(),
        secret_key: "s".into(),
    };
    assert_eq!(
        Config::new("https://h", "auto", "b", control).unwrap_err(),
        ConfigError::Credentials
    );
}

#[test]
fn a_path_prefix_is_part_of_every_address() {
    let prepared = client("https://host.test:8443/s3/")
        .get_object("a b", None, now())
        .unwrap();
    assert_eq!(prepared.url, "https://host.test:8443/s3/uploads/a%20b");
    assert_eq!(prepared.host, "host.test:8443");
    assert_eq!(
        client("http://host.test:80")
            .head_object("k", now())
            .unwrap()
            .host,
        "host.test"
    );
}

#[test]
fn the_secret_stays_out_of_debug_output() {
    let text = format!(
        "{:?}",
        Credentials {
            access_key: "AK".into(),
            secret_key: "very-secret".into()
        }
    );
    assert!(!text.contains("very-secret"));
}

#[test]
fn keys_that_s3_cannot_hold_are_refused() {
    let c = client("https://h.test");
    assert!(matches!(
        c.get_object("", None, now()),
        Err(RequestError::BadKey(_))
    ));
    assert!(matches!(
        c.delete_object(&"k".repeat(1025), now()),
        Err(RequestError::BadKey(_))
    ));
    assert!(c.delete_object(&"k".repeat(1024), now()).is_ok());
    // 1024 bytes, not characters: 513 two-byte letters are too long.
    assert!(matches!(
        c.head_object(&"é".repeat(513), now()),
        Err(RequestError::BadKey(_))
    ));
}

#[test]
fn ranges() {
    let c = client("https://h.test");
    let range = |from, to| {
        c.get_object("k", Some(ByteRange { from, to }), now())
            .map(|p| p.header("range").map(str::to_owned))
    };
    assert_eq!(range(0, Some(0)), Ok(Some("bytes=0-0".into())));
    assert_eq!(range(7, None), Ok(Some("bytes=7-".into())));
    assert_eq!(
        range(u64::MAX, Some(u64::MAX)),
        Ok(Some(format!("bytes={0}-{0}", u64::MAX)))
    );
    assert_eq!(range(5, Some(4)), Err(RequestError::BadRange));
    assert_eq!(
        c.get_object("k", None, now()).unwrap().header("range"),
        None
    );
}

#[test]
fn an_unsigned_payload_never_goes_over_plain_http() {
    let plain = client("http://minio.test:9000");
    assert_eq!(
        plain
            .put_object("k", 10, None, &[], Payload::Unsigned, now())
            .unwrap_err(),
        RequestError::UnsignedOverPlainHttp
    );
    assert_eq!(
        plain
            .upload_part("k", "id", 1, 10, Payload::Unsigned, now())
            .unwrap_err(),
        RequestError::UnsignedOverPlainHttp
    );
    assert!(
        plain
            .put_object(
                "k",
                10,
                None,
                &[],
                Payload::Chunked { chunk_size: 8192 },
                now()
            )
            .is_ok()
    );
    assert!(
        plain
            .put_object("k", 10, None, &[], Payload::hash_of(b"0123456789"), now())
            .is_ok()
    );
    assert!(
        client("https://h.test")
            .put_object("k", 10, None, &[], Payload::Unsigned, now())
            .is_ok()
    );
}

#[test]
fn chunk_sizes_are_bounded() {
    let c = client("https://h.test");
    for size in [0, 1, 8191, 16 * 1024 * 1024 + 1] {
        assert_eq!(
            c.put_object(
                "k",
                1,
                None,
                &[],
                Payload::Chunked { chunk_size: size },
                now()
            )
            .unwrap_err(),
            RequestError::BadChunkSize
        );
    }
    assert!(
        c.put_object(
            "k",
            1,
            None,
            &[],
            Payload::Chunked { chunk_size: 8192 },
            now()
        )
        .is_ok()
    );
}

#[test]
fn metadata_and_content_types_are_checked() {
    let c = client("https://h.test");
    let put = |ct: Option<&str>, meta: &[(&str, &str)]| {
        c.put_object("k", 1, ct, meta, Payload::Unsigned, now())
    };
    assert!(put(Some("application/zip"), &[("sha256", "abc")]).is_ok());
    let sent = put(None, &[("Sha256", "abc")]).unwrap();
    assert_eq!(sent.header("x-amz-meta-sha256"), Some("abc"));
    assert_eq!(
        sent.header("content-type"),
        Some("application/octet-stream")
    );
    for bad_meta in [
        vec![("", "v")],
        vec![("a b", "v")],
        vec![("a_b", "v")],
        vec![("k", "")],
        vec![("k", "line\nbreak")],
        vec![("k", "é")],
        vec![("k", " lead")],
        vec![("k", "two  spaces")],
        vec![("k", "v"), ("K", "w")],
        vec![("k", &"v".repeat(1025))],
    ] {
        assert!(
            matches!(put(None, &bad_meta), Err(RequestError::BadHeader(_))),
            "{bad_meta:?}"
        );
    }
    let sum: Vec<(String, String)> = (0..5)
        .map(|i| (format!("k{i}"), "v".repeat(1000)))
        .collect();
    let sum: Vec<(&str, &str)> = sum.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    assert!(matches!(put(None, &sum), Err(RequestError::BadHeader(_))));
    assert!(matches!(
        put(Some("a\r\nb: c"), &[]),
        Err(RequestError::BadHeader(_))
    ));
}

#[test]
fn upload_parts_and_completion_are_checked() {
    let c = client("https://h.test");
    assert!(matches!(
        c.upload_part("k", "", 1, 1, Payload::Unsigned, now()),
        Err(RequestError::BadUploadId)
    ));
    assert!(matches!(
        c.upload_part("k", &"u".repeat(1025), 1, 1, Payload::Unsigned, now()),
        Err(RequestError::BadUploadId)
    ));
    assert!(matches!(
        c.upload_part("k", "id", 0, 1, Payload::Unsigned, now()),
        Err(RequestError::BadPart(_))
    ));
    assert!(matches!(
        c.upload_part("k", "id", 10_001, 1, Payload::Unsigned, now()),
        Err(RequestError::BadPart(_))
    ));
    assert!(
        c.upload_part("k", "id", 10_000, 1, Payload::Unsigned, now())
            .is_ok()
    );

    let part = |number, etag: &str| Part {
        number,
        etag: etag.into(),
    };
    assert!(matches!(
        c.complete_multipart_upload("k", "id", &[], now()),
        Err(RequestError::BadPart(_))
    ));
    for bad in [
        vec![part(0, "\"a\"")],
        vec![part(2, "\"a\""), part(1, "\"b\"")],
        vec![part(1, "\"a\""), part(1, "\"b\"")],
        vec![part(10_001, "\"a\"")],
        vec![part(1, "")],
        vec![part(1, "a\nb")],
        vec![part(1, &"e".repeat(257))],
    ] {
        assert!(
            matches!(
                c.complete_multipart_upload("k", "id", &bad, now()),
                Err(RequestError::BadPart(_))
            ),
            "{bad:?}"
        );
    }
    let done = c
        .complete_multipart_upload("k", "id", &[part(1, "\"a<b\"")], now())
        .unwrap();
    let RequestBody::Bytes(body) = done.body else {
        panic!("a completion carries bytes")
    };
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("<ETag>&quot;a&lt;b&quot;</ETag>"), "{text}");
}

#[test]
fn the_query_is_one_sorted_string_and_tokens_survive_it() {
    let c = client("https://h.test");
    let list = c
        .list_objects(
            &ListRequest {
                max_keys: Some(1),
                continuation_token: Some("a+b/c=="),
                prefix: None,
            },
            now(),
        )
        .unwrap();
    assert_eq!(
        list.url,
        "https://h.test/uploads?continuation-token=a%2Bb%2Fc%3D%3D&encoding-type=url&list-type=2&max-keys=1"
    );
    assert_eq!(
        c.create_multipart_upload("k", None, &[], now())
            .unwrap()
            .url,
        "https://h.test/uploads/k?uploads"
    );
}

#[test]
fn a_request_that_has_a_body_states_its_length_and_one_without_does_not() {
    let c = client("https://h.test");
    assert_eq!(
        c.create_bucket(now()).unwrap().header("content-length"),
        Some("0")
    );
    assert_eq!(
        c.get_object("k", None, now())
            .unwrap()
            .header("content-length"),
        None
    );
    assert_eq!(
        c.delete_object("k", now())
            .unwrap()
            .header("content-length"),
        None
    );
    let put = c
        .put_object("k", 5, None, &[], Payload::Unsigned, now())
        .unwrap();
    assert_eq!(put.header("content-length"), Some("5"));
    assert!(matches!(put.body, RequestBody::Object { length: 5 }));
    assert_eq!(put.header("x-amz-content-sha256"), Some("UNSIGNED-PAYLOAD"));
}

#[test]
fn a_chunked_request_announces_both_lengths() {
    let c = client("http://minio.test:9000");
    let put = c
        .put_object(
            "k",
            66_560,
            None,
            &[],
            Payload::Chunked { chunk_size: 65_536 },
            now(),
        )
        .unwrap();
    assert_eq!(put.header("content-length"), Some("66824"));
    assert_eq!(put.header("x-amz-decoded-content-length"), Some("66560"));
    assert_eq!(put.header("content-encoding"), Some("aws-chunked"));
    assert_eq!(
        put.header("x-amz-content-sha256"),
        Some("STREAMING-AWS4-HMAC-SHA256-PAYLOAD")
    );
}

#[test]
fn a_signature_depends_on_the_secret_the_region_and_the_clock() {
    let make = |secret: &str, region: &str| {
        Client::new(
            Config::new(
                "https://h.test",
                region,
                "b",
                Credentials {
                    access_key: "AK".into(),
                    secret_key: secret.into(),
                },
            )
            .unwrap(),
        )
        .head_object("k", now())
        .unwrap()
        .header("authorization")
        .unwrap()
        .to_owned()
    };
    let base = make("s1", "us-east-1");
    assert_ne!(base, make("s2", "us-east-1"));
    assert_ne!(base, make("s1", "auto"));
    assert_eq!(base, make("s1", "us-east-1"));
    let later = Client::new(
        Config::new(
            "https://h.test",
            "us-east-1",
            "b",
            Credentials {
                access_key: "AK".into(),
                secret_key: "s1".into(),
            },
        )
        .unwrap(),
    )
    .head_object("k", Timestamp::from_unix(1_369_353_601).unwrap())
    .unwrap();
    assert_ne!(later.header("authorization").unwrap(), base);
}

#[test]
fn upload_plans() {
    const MIB: u64 = 1024 * 1024;
    assert_eq!(UploadPlan::new(0, 64 * MIB).unwrap(), UploadPlan::Single);
    assert_eq!(
        UploadPlan::new(16 * MIB - 1, 64 * MIB).unwrap(),
        UploadPlan::Single
    );
    // A file at the threshold goes up in parts, even if one part holds it all.
    assert_eq!(
        UploadPlan::new(16 * MIB, 64 * MIB).unwrap(),
        UploadPlan::Multipart {
            part_size: 64 * MIB,
            parts: 1
        }
    );
    let plan = UploadPlan::new(200 * MIB, 64 * MIB).unwrap();
    assert_eq!(
        plan,
        UploadPlan::Multipart {
            part_size: 64 * MIB,
            parts: 4
        }
    );
    assert_eq!(plan.part_span(1, 200 * MIB), Some((0, 64 * MIB)));
    assert_eq!(plan.part_span(4, 200 * MIB), Some((192 * MIB, 8 * MIB)));
    assert_eq!(plan.part_span(0, 200 * MIB), None);
    assert_eq!(plan.part_span(5, 200 * MIB), None);
    assert_eq!(UploadPlan::Single.part_span(1, 5), None);

    // The part size grows so that 10 000 parts always suffice.
    let huge = 2_000_000 * MIB;
    let UploadPlan::Multipart { part_size, parts } = UploadPlan::new(huge, 64 * MIB).unwrap()
    else {
        panic!()
    };
    assert!(parts <= 10_000 && part_size >= 64 * MIB && u64::from(parts) * part_size >= huge);

    assert!(UploadPlan::new(1, MIN_PART_SIZE - 1).is_err());
    assert!(UploadPlan::new(1, 6 * 1024 * MIB).is_err());
    assert!(UploadPlan::new(60_000_000 * MIB, 64 * MIB).is_err());
}

#[test]
fn every_part_but_the_last_is_the_same_size() {
    const MIB: u64 = 1024 * 1024;
    let length = 1000 * MIB + 17;
    let plan = UploadPlan::new(length, 64 * MIB).unwrap();
    let UploadPlan::Multipart { parts, .. } = plan else {
        panic!()
    };
    let mut covered = 0;
    for n in 1..=parts {
        let (start, len) = plan.part_span(n, length).unwrap();
        assert_eq!(start, covered);
        if n < parts {
            assert_eq!(len, 64 * MIB);
        }
        covered += len;
    }
    assert_eq!(covered, length);
}

#[test]
fn only_the_if_absent_calls_carry_the_condition_and_they_sign_it() {
    let c = client("https://store.test");
    let hash = Payload::hash_of(b"x");
    let plain = c.put_object("k", 1, None, &[], hash, now()).unwrap();
    let guarded = c
        .put_object_if_absent("k", 1, None, &[], hash, now())
        .unwrap();
    assert_eq!(plain.header("if-none-match"), None);
    assert_eq!(guarded.header("if-none-match"), Some("*"));
    assert!(
        guarded
            .header("authorization")
            .unwrap()
            .contains("SignedHeaders=content-length;content-type;host;if-none-match;")
    );

    let parts = [Part {
        number: 1,
        etag: "\"e\"".into(),
    }];
    let plain = c
        .complete_multipart_upload("k", "id", &parts, now())
        .unwrap();
    let guarded = c
        .complete_multipart_upload_if_absent("k", "id", &parts, now())
        .unwrap();
    assert_eq!(plain.header("if-none-match"), None);
    assert_eq!(guarded.header("if-none-match"), Some("*"));
    assert!(
        guarded
            .header("authorization")
            .unwrap()
            .contains("if-none-match")
    );
    // The same refusals as the plain calls.
    assert!(
        c.complete_multipart_upload_if_absent("k", "", &parts, now())
            .is_err()
    );
    assert!(
        c.put_object_if_absent("", 1, None, &[], hash, now())
            .is_err()
    );
}

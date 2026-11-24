//! The bucket store against a fake that speaks the protocol: the contract
//! every store keeps, and how this store behaves on the wire, under failures
//! and with a store that answers oddly.

#[macro_use]
mod support;

use std::sync::Arc;

use stellarr_outbound::Client;
use stellarr_storage::{
    BucketStore, BucketTuning, ByteRange, Error, ObjectStorage, StorageOptions, StorageProvider,
    StoreFault,
};
use support::contract::Env;
use support::fake_s3::{ACCESS_KEY, FakeS3, MIN_PART, SECRET_KEY};
use support::{pattern, put, read};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BUCKET: &str = "uploads";

fn options(endpoint: &str) -> StorageOptions {
    StorageOptions {
        provider: StorageProvider::S3,
        endpoint: endpoint.to_owned(),
        access_key: ACCESS_KEY.into(),
        secret_key: SECRET_KEY.into(),
        bucket: BUCKET.into(),
    }
}

fn tuning() -> BucketTuning {
    BucketTuning {
        part_size: MIN_PART as u64,
        ..BucketTuning::default()
    }
}

fn store_for(fake: &FakeS3) -> BucketStore {
    BucketStore::with_tuning(&options(&fake.endpoint), Client::new(), tuning()).unwrap()
}

async fn prepared() -> (FakeS3, BucketStore) {
    let fake = FakeS3::start().await;
    let store = store_for(&fake);
    store.prepare().await.unwrap();
    (fake, store)
}

async fn env() -> Env {
    let (fake, store) = prepared().await;
    let state = fake.state.clone();
    Env {
        store: Arc::new(store),
        exclusive_put: false,
        boundary: MIN_PART,
        longest_key: 1024,
        leftovers: Box::new(move || state.lock().unwrap().open_upload_ids()),
        guard: Box::new(fake),
    }
}

contract_tests!(env().await);

#[tokio::test]
async fn requests_are_path_style_signed_and_carry_what_the_store_needs() {
    let (fake, store) = prepared().await;
    fake.clear_log();
    put(&store, "audio/2026/11/a b+c.webm", b"opus")
        .await
        .unwrap();
    let log = fake.requests();
    assert_eq!(
        log.iter().map(|r| r.method.as_str()).collect::<Vec<_>>(),
        ["HEAD", "PUT"]
    );
    for request in &log {
        // The fake verifies every signature; a 403 would be here.
        assert_ne!(request.status, 403, "{request:?}");
        assert_eq!(
            request.header("host"),
            Some(fake.endpoint.trim_start_matches("http://"))
        );
        assert!(
            request
                .header("authorization")
                .unwrap()
                .contains("/us-east-1/s3/aws4_request")
        );
    }
    assert_eq!(log[1].path(), "/uploads/audio/2026/11/a%20b%2Bc.webm");
    // The type is the generic one, and nothing but the signed headers.
    let (stored, content_type) = fake.stored(BUCKET, "audio/2026/11/a b+c.webm").unwrap();
    assert_eq!(stored, b"opus");
    assert_eq!(content_type, "application/octet-stream");
    assert!(
        log[1]
            .headers
            .iter()
            .all(|(n, _)| !n.starts_with("x-amz-meta"))
    );
    assert!(log[1].header("x-amz-content-sha256").unwrap().len() == 64);
    assert_eq!(
        log[1]
            .headers
            .iter()
            .filter(|(n, _)| n == "content-length")
            .count(),
        1
    );
    assert_eq!(log[1].header("content-length"), Some("4"));
}

#[tokio::test]
async fn the_existing_key_is_found_with_a_head_before_a_byte_is_sent() {
    let (fake, store) = prepared().await;
    put(&store, "twice/one.bin", b"first").await.unwrap();
    fake.clear_log();
    let error = put(&store, "twice/one.bin", b"second").await.unwrap_err();
    assert!(matches!(error, Error::AlreadyExists(_)));
    let methods: Vec<String> = fake.requests().into_iter().map(|r| r.method).collect();
    assert_eq!(methods, ["HEAD"]);
    assert_eq!(fake.stored(BUCKET, "twice/one.bin").unwrap().0, b"first");
}

#[tokio::test]
async fn unsafe_keys_never_reach_the_network() {
    let (fake, store) = prepared().await;
    fake.clear_log();
    for key in [
        "../x",
        "a/../b",
        "./x",
        "a//b",
        "/abs",
        "",
        ".incoming/x",
        "a\\b",
    ] {
        assert!(matches!(
            put(&store, key, b"x").await,
            Err(Error::InvalidKey(_))
        ));
        assert!(matches!(
            store.get(key, None).await,
            Err(Error::InvalidKey(_))
        ));
        assert!(matches!(store.delete(key).await, Err(Error::InvalidKey(_))));
    }
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn a_small_object_is_one_request_and_a_big_one_goes_in_equal_parts() {
    let (fake, store) = prepared().await;
    let mib = 1024 * 1024;
    let cases: [(usize, &[usize]); 6] = [
        (5 * mib - 1, &[]),
        (5 * mib, &[]),
        (5 * mib + 1, &[5 * mib, 1]),
        (10 * mib, &[5 * mib, 5 * mib]),
        (12 * mib, &[5 * mib, 5 * mib, 2 * mib]),
        (10 * mib + 1, &[5 * mib, 5 * mib, 1]),
    ];
    for (n, (length, parts)) in cases.iter().enumerate() {
        let key = format!("parts/{n}.bin");
        let bytes = pattern(*length, n as u64);
        fake.clear_log();
        put(&store, &key, &bytes).await.unwrap();
        let puts = fake.requests().iter().filter(|r| r.method == "PUT").count();
        let posts = fake
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count();
        if parts.is_empty() {
            assert_eq!((puts, posts), (1, 0), "{length} bytes is one request");
        } else {
            assert_eq!((puts, posts), (parts.len(), 2), "{length} bytes");
            let sizes = fake
                .state
                .lock()
                .unwrap()
                .completed_part_sizes
                .last()
                .cloned()
                .unwrap();
            assert_eq!(&sizes, parts);
        }
        assert_eq!(fake.stored(BUCKET, &key).unwrap().0, bytes);
    }
    assert_eq!(fake.open_uploads(), 0);
}

#[tokio::test]
async fn a_part_the_store_refuses_aborts_the_upload() {
    let (fake, store) = prepared().await;
    // The first PUT is the part: a refusal that is no use to repeat.
    fake.fail_next("PUT", 400, 1);
    let error = put(&store, "refused/big.bin", &pattern(11 * 1024 * 1024, 1))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Store {
                kind: StoreFault::Rejected,
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(fake.aborted(), 1);
    assert_eq!(fake.open_uploads(), 0);
    assert!(fake.stored(BUCKET, "refused/big.bin").is_none());
}

#[tokio::test]
async fn a_part_that_fails_in_the_middle_of_a_long_upload_aborts_it_too() {
    let (fake, store) = prepared().await;
    let mut data = std::io::Cursor::new(pattern(6 * 1024 * 1024, 1));
    // A reader that gives 6 MiB and then fails: part 1 is up, part 2 is not.
    let mut failing = (&mut data).chain(support::FailingReader { before_failure: 0 });
    let error = store
        .put("failing/big.bin", &mut failing)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Io(_)), "{error}");
    assert_eq!(fake.aborted(), 1);
    assert_eq!(fake.open_uploads(), 0);
}

#[tokio::test]
async fn an_answer_of_200_that_carries_an_error_fails_the_upload() {
    let (fake, store) = prepared().await;
    fake.state.lock().unwrap().settings.complete_with_error = true;
    let error = put(&store, "assembly/big.bin", &pattern(6 * 1024 * 1024, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Store { .. }), "{error}");
    assert!(fake.stored(BUCKET, "assembly/big.bin").is_none());
    assert_eq!(fake.open_uploads(), 0);
}

#[tokio::test]
async fn blanks_before_the_answer_to_completing_are_no_harm() {
    let (fake, store) = prepared().await;
    fake.state
        .lock()
        .unwrap()
        .settings
        .complete_leading_whitespace = true;
    let bytes = pattern(6 * 1024 * 1024, 1);
    put(&store, "blanks/big.bin", &bytes).await.unwrap();
    assert_eq!(fake.stored(BUCKET, "blanks/big.bin").unwrap().0, bytes);
}

#[tokio::test]
async fn a_store_that_fails_twice_is_tried_again_and_one_that_keeps_failing_is_not_endless() {
    let (fake, store) = prepared().await;
    fake.fail_next("HEAD", 503, 2);
    put(&store, "retry/one.bin", b"x").await.unwrap();

    fake.fail_next("HEAD", 503, 10);
    let error = put(&store, "retry/two.bin", b"x").await.unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Store {
                kind: StoreFault::Throttled,
                ..
            }
        ),
        "{error}"
    );
    assert!(error.is_retryable());
    let heads = fake
        .requests()
        .iter()
        .filter(|r| r.method == "HEAD" && r.status == 503)
        .count();
    assert_eq!(
        heads,
        2 + 3,
        "three tries for the second, after two for the first"
    );
}

#[tokio::test]
async fn the_answers_of_a_store_are_told_apart_by_what_a_caller_can_do() {
    let (fake, store) = prepared().await;
    // Wrong secret: the signature is refused.
    let wrong = StorageOptions {
        secret_key: "another-secret".into(),
        ..options(&fake.endpoint)
    };
    let denied = BucketStore::with_tuning(&wrong, Client::new(), tuning()).unwrap();
    let error = denied.probe().await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::Store {
                kind: StoreFault::Denied,
                ..
            }
        ),
        "{error}"
    );
    assert!(!error.is_retryable());

    // Throttled.
    fake.fail_next("GET", 503, 10);
    assert!(matches!(
        store.probe().await.unwrap_err(),
        Error::Store {
            kind: StoreFault::Unreachable,
            ..
        }
    ));
    fake.state.lock().unwrap().settings.faults.clear();
}

#[tokio::test]
async fn a_bucket_that_is_not_there_is_a_store_that_is_not_ready() {
    let fake = FakeS3::start().await;
    let store = store_for(&fake);
    let not_ready = |e: &Error| {
        matches!(
            e,
            Error::Store {
                kind: StoreFault::NotReady,
                ..
            }
        )
    };
    assert!(not_ready(&store.probe().await.unwrap_err()));
    assert!(not_ready(&store.get("a/b", None).await.unwrap_err()));
    assert!(not_ready(&put(&store, "a/b", b"x").await.unwrap_err()));
    assert!(not_ready(&store.list().collect().await.unwrap_err()));
    assert!(not_ready(&store.delete("a/b").await.unwrap_err()));
    assert!(store.probe().await.unwrap_err().is_retryable());

    store.prepare().await.unwrap();
    store.probe().await.unwrap();
    put(&store, "a/b", b"x").await.unwrap();
}

#[tokio::test]
async fn preparing_creates_the_bucket_once() {
    let fake = FakeS3::start().await;
    let store = store_for(&fake);
    store.prepare().await.unwrap();
    store.prepare().await.unwrap();
    let creates = fake.requests().iter().filter(|r| r.method == "PUT").count();
    assert_eq!(creates, 1);
}

#[tokio::test]
async fn a_store_that_ignores_a_range_is_not_taken_for_one_that_honours_it() {
    let (fake, store) = prepared().await;
    put(&store, "ignored/ten.bin", b"0123456789").await.unwrap();
    put(&store, "ignored/empty.bin", b"").await.unwrap();
    fake.state.lock().unwrap().settings.ignore_range = true;

    let error = store
        .get(
            "ignored/ten.bin",
            Some(ByteRange {
                from: 2,
                to: Some(5),
            }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Store {
                kind: StoreFault::Malformed,
                ..
            }
        ),
        "{error}"
    );
    // A range that asks for everything is answered rightly with everything.
    let all = store
        .get("ignored/ten.bin", Some(ByteRange { from: 0, to: None }))
        .await
        .unwrap();
    assert_eq!(
        all.range,
        Some(ByteRange {
            from: 0,
            to: Some(9)
        })
    );
    assert_eq!(all.length, 10);
    assert_eq!(read(all).await, b"0123456789");
    // An empty object has no range to give.
    assert!(matches!(
        store
            .get("ignored/empty.bin", Some(ByteRange { from: 0, to: None }))
            .await,
        Err(Error::RangeNotSatisfiable(_))
    ));
}

#[tokio::test]
async fn a_listing_follows_the_pages_and_reads_keys_as_the_store_wrote_them() {
    let (fake, store) = prepared().await;
    fake.state.lock().unwrap().settings.page_size = 2;
    let keys = [
        "a b/c+d",
        "x%2By",
        "é/ü.bin",
        "plain",
        "q?r#s",
        "u&v<w",
        "z/z",
    ];
    for key in keys {
        put(&store, key, b"1").await.unwrap();
    }
    fake.clear_log();
    let listed: Vec<String> = store
        .list()
        .collect()
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    let mut expected: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(listed, expected);
    let requests = fake.requests();
    assert!(requests.len() >= 4, "{} pages", requests.len());
    assert!(
        requests
            .iter()
            .all(|r| r.query().contains("encoding-type=url"))
    );
    assert!(
        requests
            .iter()
            .skip(1)
            .all(|r| r.query().contains("continuation-token="))
    );
}

#[tokio::test]
async fn keys_that_no_store_takes_are_left_out_of_a_listing() {
    let (fake, store) = prepared().await;
    put(&store, "good/one.bin", b"1").await.unwrap();
    for key in [
        "../escape",
        "a/../b",
        "a//b",
        "/abs",
        ".incoming/x",
        "a\\b",
        "trailing/",
    ] {
        fake.plant(BUCKET, key, b"x");
    }
    let listed = store.list().collect().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "good/one.bin");
}

#[tokio::test]
async fn a_download_cut_short_by_the_store_is_an_error_not_a_short_object() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\nonly twenty bytes...",
                    )
                    .await;
                // Closing here ends the body 80 bytes early.
            });
        }
    });
    let store = BucketStore::with_tuning(&options(&endpoint), Client::new(), tuning()).unwrap();
    let object = store.get("cut/one.bin", None).await.unwrap();
    assert_eq!(object.length, 100);
    let error = object.body.collect(1000).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::Store {
                kind: StoreFault::Unreachable,
                ..
            }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn a_partial_answer_that_contradicts_itself_is_refused() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer).await;
                // Says 4 bytes of 10 in the range and 9 in the length.
                let _ = stream
                    .write_all(b"HTTP/1.1 206 Partial Content\r\ncontent-length: 9\r\ncontent-range: bytes 2-5/10\r\nconnection: close\r\n\r\n123456789")
                    .await;
            });
        }
    });
    let store = BucketStore::with_tuning(&options(&endpoint), Client::new(), tuning()).unwrap();
    let error = store
        .get(
            "odd/one.bin",
            Some(ByteRange {
                from: 2,
                to: Some(5),
            }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Store {
                kind: StoreFault::Malformed,
                ..
            }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn settings_that_cannot_make_a_store_are_refused_at_once() {
    let client = Client::new();
    for (endpoint, bucket) in [
        ("", "b"),
        ("http://x.test", ""),
        ("   ", "b"),
        ("ftp://x.test", "b"),
        ("http://x.test/a?b=1", "b"),
    ] {
        let options = StorageOptions {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            ..options("http://x.test")
        };
        assert!(
            matches!(
                BucketStore::new(&options, client.clone()),
                Err(Error::Store {
                    kind: StoreFault::Rejected,
                    ..
                })
            ),
            "{endpoint:?} {bucket:?}"
        );
    }
    for part_size in [0, 5 * 1024 * 1024 - 1, 5 * 1024 * 1024 * 1024 + 1] {
        let tuning = BucketTuning {
            part_size,
            ..BucketTuning::default()
        };
        assert!(
            BucketStore::with_tuning(&options("http://x.test"), client.clone(), tuning).is_err()
        );
    }
}

#[tokio::test]
async fn the_provider_in_the_settings_picks_the_store() {
    let data = support::TempDir::new("open");
    let folder =
        stellarr_storage::open(&StorageOptions::default(), data.path(), Client::new()).unwrap();
    put(&*folder, "pick/one.bin", b"x").await.unwrap();
    assert!(data.path().join("files/pick/one.bin").is_file());

    let fake = FakeS3::start().await;
    let bucket =
        stellarr_storage::open(&options(&fake.endpoint), data.path(), Client::new()).unwrap();
    bucket.prepare().await.unwrap();
    put(&*bucket, "pick/one.bin", b"y").await.unwrap();
    assert_eq!(fake.stored(BUCKET, "pick/one.bin").unwrap().0, b"y");

    let incomplete = StorageOptions {
        provider: StorageProvider::S3,
        ..StorageOptions::default()
    };
    assert!(stellarr_storage::open(&incomplete, data.path(), Client::new()).is_err());
}

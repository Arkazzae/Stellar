//! The promises every store keeps, as tests. A test file builds an [`Env`] for
//! its store and the macro [`contract_tests!`] runs each case against it.

use std::sync::Arc;

use stellarr_storage::{ByteRange, Error, ObjectStorage, StoredObjectInfo};
use tokio::io::AsyncWriteExt;

use super::{FailingReader, pattern, put, read, read_range, sha256_hex};

pub struct Env {
    pub store: Arc<dyn ObjectStorage>,
    /// Whether writing to a taken key is refused, and two writers racing for
    /// one key leave exactly one winner: the folder store, and a bucket on a
    /// server that honours the conditional create.
    pub exclusive_put: bool,
    /// The size at which uploads change shape (a bucket's part size), so the
    /// suite can try objects on both sides of it.
    pub boundary: usize,
    /// The longest key to try. A bucket takes 1024 bytes; a folder is held to
    /// what its whole path may be, which the platform decides.
    pub longest_key: usize,
    /// What a cancelled or failed write left behind: staged files, open
    /// multipart uploads. Empty when the store cannot tell.
    pub leftovers: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    /// Kept alive for the test: scratch folders, servers.
    pub guard: Box<dyn std::any::Any + Send + Sync>,
}

const TEN: &[u8] = b"0123456789";

pub async fn whole_object_reads_back_as_stored(env: &Env) {
    put(&*env.store, "attachments/2026/11/whole.bin", TEN)
        .await
        .unwrap();
    let object = env
        .store
        .get("attachments/2026/11/whole.bin", None)
        .await
        .unwrap();
    assert_eq!(
        (object.length, object.total_length, object.range),
        (10, 10, None)
    );
    assert_eq!(read(object).await, TEN);
}

pub async fn a_range_reads_only_its_bytes_and_says_which(env: &Env) {
    put(&*env.store, "ranges/object.bin", TEN).await.unwrap();
    for (from, to, first, last) in [
        (2, Some(5), 2, 5),
        (0, Some(0), 0, 0),
        (7, None, 7, 9),
        (7, Some(100), 7, 9),
        (0, None, 0, 9),
        (9, None, 9, 9),
        (0, Some(9), 0, 9),
    ] {
        let (object, bytes) = read_range(&*env.store, "ranges/object.bin", ByteRange { from, to })
            .await
            .unwrap();
        assert_eq!(bytes, &TEN[first..=last], "range {from}-{to:?}");
        assert_eq!(object.length, (last - first + 1) as u64);
        assert_eq!(object.total_length, 10);
        assert_eq!(
            object.range,
            Some(ByteRange {
                from: first as u64,
                to: Some(last as u64)
            })
        );
    }
}

pub async fn a_range_that_starts_at_or_past_the_end_is_refused(env: &Env) {
    put(&*env.store, "ranges/short.bin", TEN).await.unwrap();
    put(&*env.store, "ranges/empty.bin", b"").await.unwrap();
    for (key, from, to) in [
        ("ranges/short.bin", 10, None),
        ("ranges/short.bin", 11, Some(20)),
        ("ranges/short.bin", u64::MAX, None),
        ("ranges/short.bin", 5, Some(2)),
        ("ranges/empty.bin", 0, None),
        ("ranges/empty.bin", 0, Some(0)),
    ] {
        let error = env
            .store
            .get(key, Some(ByteRange { from, to }))
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::RangeNotSatisfiable(_)),
            "{key} {from}-{to:?}: {error}"
        );
    }
}

pub async fn an_empty_object_reads_back_empty(env: &Env) {
    put(&*env.store, "empty/one.bin", b"").await.unwrap();
    let object = env.store.get("empty/one.bin", None).await.unwrap();
    assert_eq!(
        (object.length, object.total_length, object.range),
        (0, 0, None)
    );
    assert!(read(object).await.is_empty());
}

pub async fn a_key_that_names_nothing_is_not_found(env: &Env) {
    put(&*env.store, "present/one.bin", TEN).await.unwrap();
    let missing = env.store.get("present/two.bin", None).await.unwrap_err();
    assert!(
        matches!(&missing, Error::NotFound(key) if key == "present/two.bin"),
        "{missing}"
    );
    let missing = env
        .store
        .get("absent/folder/one.bin", None)
        .await
        .unwrap_err();
    assert!(matches!(missing, Error::NotFound(_)));
}

pub async fn deleting_removes_the_object_and_deleting_again_is_no_error(env: &Env) {
    put(&*env.store, "deleted/one.bin", TEN).await.unwrap();
    env.store.delete("deleted/one.bin").await.unwrap();
    env.store.delete("deleted/one.bin").await.unwrap();
    env.store.delete("never/stored.bin").await.unwrap();
    assert!(matches!(
        env.store.get("deleted/one.bin", None).await.unwrap_err(),
        Error::NotFound(_)
    ));
    // The key can be used again.
    put(&*env.store, "deleted/one.bin", b"again").await.unwrap();
    assert_eq!(
        read(env.store.get("deleted/one.bin", None).await.unwrap()).await,
        b"again"
    );
}

pub async fn a_key_that_already_names_an_object_is_not_overwritten(env: &Env) {
    put(&*env.store, "unique/one.bin", TEN).await.unwrap();
    if env.exclusive_put {
        let error = put(&*env.store, "unique/one.bin", b"other")
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AlreadyExists(key) if key == "unique/one.bin"),
            "{error}"
        );
        assert_eq!(
            read(env.store.get("unique/one.bin", None).await.unwrap()).await,
            TEN
        );
    } else {
        // A server that does not know the conditional create replaces the
        // object: the promise is not kept, and this says so.
        put(&*env.store, "unique/one.bin", b"other").await.unwrap();
        assert_eq!(
            read(env.store.get("unique/one.bin", None).await.unwrap()).await,
            b"other"
        );
    }
}

pub async fn the_listing_names_every_object_by_key_and_size_in_key_order(env: &Env) {
    put(&*env.store, "b/2026/11/second.bin", TEN).await.unwrap();
    put(&*env.store, "a/first.bin", &[1, 2, 3]).await.unwrap();
    put(&*env.store, "top.bin", b"").await.unwrap();
    let listed = env.store.list().collect().await.unwrap();
    let expected = [
        ("a/first.bin", 3),
        ("b/2026/11/second.bin", 10),
        ("top.bin", 0),
    ]
    .map(|(key, size)| StoredObjectInfo {
        key: key.into(),
        size,
    });
    assert_eq!(listed, expected);
}

/// Order is by bytes, as an S3 listing has it, and not by what a person would
/// sort: capitals before lower case, `-` before `.` before `/`, accents last.
pub async fn the_listing_is_in_byte_order(env: &Env) {
    let keys = ["a/b", "a.b", "a-b", "B/x", "a_b", "é/x", "z"];
    for key in keys {
        put(&*env.store, key, b"x").await.unwrap();
    }
    let listed: Vec<String> = env
        .store
        .list()
        .collect()
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    let mut sorted: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(listed, sorted);
}

pub async fn an_empty_store_lists_nothing(env: &Env) {
    assert!(env.store.list().collect().await.unwrap().is_empty());
}

pub async fn a_prepared_store_answers_the_probe(env: &Env) {
    env.store.prepare().await.unwrap();
    env.store.probe().await.unwrap();
    // Preparing again changes nothing.
    env.store.prepare().await.unwrap();
}

const REFUSED_KEYS: &[&str] = &[
    "",
    "/etc/passwd",
    "/",
    "../outside",
    "..",
    ".",
    "a/../../outside",
    "a/../b",
    "./x",
    "a/./b",
    "a//b",
    "a/",
    "//",
    "a\\b",
    "\\",
    "a\0b",
    "\0",
    ".incoming",
    ".incoming/half-written",
    ".incoming/",
];

pub async fn a_key_that_could_leave_the_folder_or_reach_unfinished_writes_is_refused(env: &Env) {
    let too_long = format!("{}/{}", "a".repeat(600), "b".repeat(430));
    assert!(too_long.len() > 1024);
    let mut keys: Vec<&str> = REFUSED_KEYS.to_vec();
    keys.push(&too_long);
    for key in keys {
        let shown = key.escape_debug().to_string();
        assert!(
            matches!(put(&*env.store, key, b"x").await, Err(Error::InvalidKey(_))),
            "put {shown}"
        );
        assert!(
            matches!(env.store.get(key, None).await, Err(Error::InvalidKey(_))),
            "get {shown}"
        );
        assert!(
            matches!(env.store.delete(key).await, Err(Error::InvalidKey(_))),
            "delete {shown}"
        );
    }
    assert!(env.store.list().collect().await.unwrap().is_empty());
}

/// Keys at the edge of what the rules allow work, and read back as themselves.
pub async fn keys_at_the_edge_of_the_rules_are_taken(env: &Env) {
    // The longest key in segments short enough for a file name.
    let mut longest = String::new();
    while longest.len() < env.longest_key {
        if !longest.is_empty() {
            longest.push('/');
        }
        let room = (env.longest_key - longest.len()).min(200);
        longest.push_str(&"abcdefghij".repeat(room.div_ceil(10))[..room]);
    }
    assert_eq!(longest.len(), env.longest_key);
    let keys = [
        "x",
        ".hidden",
        "...",
        "a/.../b",
        "a/..b/c..",
        ".incomingx",
        "a/.incoming/b",
        "dir/.dot",
        "żółć/ü.bin",
        "日本語/ファイル.txt",
        "with space/and+plus%percent?q#h",
        "a:b/c*d",
        "quote\"s/and'apos",
        "<tag>&amp;/x",
        "tab\tinside/x",
        longest.as_str(),
    ];
    for (n, key) in keys.iter().enumerate() {
        let bytes = pattern(100 + n, n as u64);
        put(&*env.store, key, &bytes)
            .await
            .unwrap_or_else(|e| panic!("put {key:?}: {e}"));
        let object = env
            .store
            .get(key, None)
            .await
            .unwrap_or_else(|e| panic!("get {key:?}: {e}"));
        assert_eq!(read(object).await, bytes, "{key:?}");
    }
    let mut listed: Vec<String> = env
        .store
        .list()
        .collect()
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    let mut expected: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    listed.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(listed, expected);
    for key in keys {
        env.store.delete(key).await.unwrap();
        assert!(
            matches!(env.store.get(key, None).await, Err(Error::NotFound(_))),
            "{key:?}"
        );
    }
}

/// A key that is the start of another key's path names nothing, and neither
/// can be confused with the other.
pub async fn a_key_and_the_keys_below_it_are_different_things(env: &Env) {
    put(&*env.store, "tree/leaf.bin", TEN).await.unwrap();
    let error = env.store.get("tree", None).await.unwrap_err();
    assert!(matches!(error, Error::NotFound(_)), "{error}");
    env.store.delete("tree").await.unwrap();
    assert_eq!(
        read(env.store.get("tree/leaf.bin", None).await.unwrap()).await,
        TEN
    );
}

/// Objects on both sides of every size at which an upload changes shape come
/// back byte for byte, whatever the way they went up.
pub async fn objects_around_the_part_boundary_round_trip(env: &Env) {
    let b = env.boundary;
    for (n, length) in [
        0,
        1,
        b - 1,
        b,
        b + 1,
        2 * b - 1,
        2 * b,
        2 * b + 1,
        3 * b + 7,
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = pattern(length, 100 + n as u64);
        let key = format!("sizes/{length}.bin");
        put(&*env.store, &key, &bytes).await.unwrap();
        let object = env.store.get(&key, None).await.unwrap();
        assert_eq!(object.total_length, length as u64);
        let read_back = read(object).await;
        assert_eq!(sha256_hex(&read_back), sha256_hex(&bytes), "{length} bytes");
    }
    // A slice that crosses a part boundary.
    let key = format!("sizes/{}.bin", 3 * b + 7);
    let (_, bytes) = read_range(
        &*env.store,
        &key,
        ByteRange {
            from: (b - 3) as u64,
            to: Some((b + 3) as u64),
        },
    )
    .await
    .unwrap();
    assert_eq!(bytes, pattern(3 * b + 7, 100 + 8)[b - 3..=b + 3]);
}

/// Of writers that race for one key, a store that promises it lets exactly
/// one win; none may leave a mix of two writers' bytes.
pub async fn concurrent_writers_to_one_key_never_mix_their_bytes(env: &Env) {
    let writers = 8;
    let payloads: Vec<Vec<u8>> = (0..writers)
        .map(|n| pattern(300_000 + n * 1000, n as u64))
        .collect();
    let mut tasks = Vec::new();
    for payload in &payloads {
        let store = env.store.clone();
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            put(&*store, "race/one.bin", &payload).await
        }));
    }
    let mut won = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(()) => won += 1,
            Err(Error::AlreadyExists(_)) => {}
            Err(other) => panic!("a writer failed for another reason: {other}"),
        }
    }
    if env.exclusive_put {
        assert_eq!(won, 1, "exactly one writer wins");
    } else {
        assert!(won >= 1);
    }
    let stored = read(env.store.get("race/one.bin", None).await.unwrap()).await;
    assert!(
        payloads.contains(&stored),
        "the object is one writer's bytes, whole"
    );
    assert_eq!(env.store.list().collect().await.unwrap().len(), 1);
    assert!((env.leftovers)().is_empty(), "{:?}", (env.leftovers)());
}

/// A reader that has started keeps reading the bytes it started with, though
/// the key is deleted and used again under it.
pub async fn a_reader_during_a_replace_sees_one_version_whole(env: &Env) {
    let old = pattern(2_500_000, 1);
    let new = pattern(2_500_000, 2);
    put(&*env.store, "replace/one.bin", &old).await.unwrap();
    let mut object = env.store.get("replace/one.bin", None).await.unwrap();
    let mut seen = Vec::new();
    let first = object.body.chunk().await.unwrap().unwrap();
    seen.extend_from_slice(&first);

    // The delete may have to wait for the read on a store that does so, and
    // the read must not wait for it: run them together.
    let store = env.store.clone();
    let replacing = tokio::spawn(async move {
        store.delete("replace/one.bin").await.unwrap();
        put(&*store, "replace/one.bin", &new_bytes()).await.unwrap();
    });
    fn new_bytes() -> Vec<u8> {
        pattern(2_500_000, 2)
    }
    while let Some(chunk) = object.body.chunk().await.unwrap() {
        seen.extend_from_slice(&chunk);
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), replacing)
        .await
        .expect("the replace finished")
        .unwrap();
    assert_eq!(
        sha256_hex(&seen),
        sha256_hex(&old),
        "the reader got the old version whole"
    );
    let now = read(env.store.get("replace/one.bin", None).await.unwrap()).await;
    assert_eq!(sha256_hex(&now), sha256_hex(&new));
}

/// An upload that has not finished is not an object.
pub async fn a_write_in_progress_is_invisible(env: &Env) {
    let (mut sender, mut receiver) = tokio::io::duplex(1024);
    let store = env.store.clone();
    let writing = tokio::spawn(async move { store.put("pending/one.bin", &mut receiver).await });
    sender.write_all(&pattern(10_000, 3)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    assert!(matches!(
        env.store.get("pending/one.bin", None).await,
        Err(Error::NotFound(_))
    ));
    assert!(env.store.list().collect().await.unwrap().is_empty());

    drop(sender);
    writing.await.unwrap().unwrap();
    let object = env.store.get("pending/one.bin", None).await.unwrap();
    assert_eq!(object.total_length, 10_000);
}

pub async fn a_write_that_fails_leaves_nothing_behind(env: &Env) {
    // Short, and long enough to be past the first part of a bucket upload.
    for (n, before) in [4, env.boundary + 100, 2 * env.boundary + 100]
        .into_iter()
        .enumerate()
    {
        let mut failing = FailingReader {
            before_failure: before,
        };
        let key = format!("failed/upload{n}.bin");
        let error = env.store.put(&key, &mut failing).await.unwrap_err();
        assert!(matches!(error, Error::Io(_)), "{error}");
        assert!(matches!(
            env.store.get(&key, None).await,
            Err(Error::NotFound(_))
        ));
    }
    assert!(env.store.list().collect().await.unwrap().is_empty());
    assert!((env.leftovers)().is_empty(), "{:?}", (env.leftovers)());
}

/// Dropping the future of a write, as a client that hangs up does, leaves no
/// object and nothing staged.
pub async fn a_cancelled_write_leaves_nothing_behind(env: &Env) {
    for (n, sent) in [10_000, env.boundary + 5_000].into_iter().enumerate() {
        let (mut sender, mut receiver) = tokio::io::duplex(64 * 1024);
        let store = env.store.clone();
        let key = format!("cancelled/upload{n}.bin");
        let k = key.clone();
        let writing = tokio::spawn(async move { store.put(&k, &mut receiver).await });
        let feeding = tokio::spawn(async move {
            let _ = sender.write_all(&pattern(sent, 5)).await;
            // Never closed: the writer is waiting for more when it is cancelled.
            std::future::pending::<()>().await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        writing.abort();
        let _ = writing.await;
        feeding.abort();
        // Let the clean-up that a drop starts run.
        for _ in 0..40 {
            if (env.leftovers)().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(matches!(
            env.store.get(&key, None).await,
            Err(Error::NotFound(_))
        ));
    }
    assert!(env.store.list().collect().await.unwrap().is_empty());
    assert!((env.leftovers)().is_empty(), "{:?}", (env.leftovers)());
}

/// A download is handed over in pieces, not as one buffer of the object.
pub async fn a_download_arrives_in_pieces(env: &Env) {
    let bytes = pattern(3_000_000, 9);
    put(&*env.store, "pieces/big.bin", &bytes).await.unwrap();
    let mut object = env.store.get("pieces/big.bin", None).await.unwrap();
    let mut total = 0;
    let mut largest = 0;
    while let Some(piece) = object.body.chunk().await.unwrap() {
        total += piece.len();
        largest = largest.max(piece.len());
    }
    assert_eq!(total, bytes.len());
    assert!(
        largest <= 1024 * 1024,
        "the largest piece was {largest} bytes"
    );
}

#[macro_export]
macro_rules! contract_tests {
    (ignored $make:expr) => {
        contract_tests!(@all [ignore = "needs a real store: see tests/servers.sh"] $make);
    };
    ($make:expr) => {
        contract_tests!(@all [cfg(all())] $make);
    };
    (@all [$attr:meta] $make:expr) => {
        contract_tests!(@each [$attr] $make;
            whole_object_reads_back_as_stored,
            a_range_reads_only_its_bytes_and_says_which,
            a_range_that_starts_at_or_past_the_end_is_refused,
            an_empty_object_reads_back_empty,
            a_key_that_names_nothing_is_not_found,
            deleting_removes_the_object_and_deleting_again_is_no_error,
            a_key_that_already_names_an_object_is_not_overwritten,
            the_listing_names_every_object_by_key_and_size_in_key_order,
            the_listing_is_in_byte_order,
            an_empty_store_lists_nothing,
            a_prepared_store_answers_the_probe,
            a_key_that_could_leave_the_folder_or_reach_unfinished_writes_is_refused,
            keys_at_the_edge_of_the_rules_are_taken,
            a_key_and_the_keys_below_it_are_different_things,
            objects_around_the_part_boundary_round_trip,
            concurrent_writers_to_one_key_never_mix_their_bytes,
            a_reader_during_a_replace_sees_one_version_whole,
            a_write_in_progress_is_invisible,
            a_write_that_fails_leaves_nothing_behind,
            a_cancelled_write_leaves_nothing_behind,
            a_download_arrives_in_pieces
        );
    };
    (@each [$attr:meta] $make:expr; $($name:ident),*) => {
        $(
            #[$attr]
            #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
            async fn $name() {
                let env = $make;
                $crate::support::contract::$name(&env).await;
            }
        )*
    };
}

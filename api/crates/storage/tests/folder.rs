//! The folder store: the contract every store keeps, and what is the folder's
//! own: the layout on disk, staging, and what a crash leaves behind.

#[macro_use]
mod support;

use std::path::Path;
use std::sync::Arc;

use stellarr_storage::{ByteRange, Error, FolderStore, ObjectStorage};
use support::contract::Env;
use support::{FailingReader, TempDir, pattern, put, read};
use tokio::io::AsyncWriteExt;

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn env() -> Env {
    let dir = TempDir::new("folder");
    let root = dir.path().join("files");
    let incoming = root.join(".incoming");
    Env {
        store: Arc::new(FolderStore::new(&root)),
        exclusive_put: true,
        boundary: 100_000,
        longest_key: 500,
        leftovers: Box::new(move || names_in(&incoming)),
        guard: Box::new(dir),
    }
}

contract_tests!(env());

struct Fixture {
    dir: TempDir,
    store: FolderStore,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("folder-own");
        let store = FolderStore::new(dir.path().join("files"));
        Self { dir, store }
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("files")
    }

    fn incoming(&self) -> std::path::PathBuf {
        self.root().join(".incoming")
    }
}

#[tokio::test]
async fn an_object_is_a_plain_file_at_the_path_its_key_names() {
    let f = Fixture::new();
    put(&f.store, "audio/2026/11/voice.webm", b"opus")
        .await
        .unwrap();
    let on_disk = std::fs::read(f.root().join("audio/2026/11/voice.webm")).unwrap();
    assert_eq!(on_disk, b"opus");
    assert!(names_in(&f.incoming()).is_empty(), "nothing is left staged");
}

/// The tree a person's files make is the tree an older server wrote: the same
/// folders and file names, the bytes untouched, an empty `.incoming`, and
/// nothing else.
#[tokio::test]
async fn the_tree_on_disk_has_exactly_the_names_and_bytes_of_the_keys() {
    let f = Fixture::new();
    let objects = [
        ("images/2026/11/0f1e2d3c.png", pattern(5_000, 1)),
        ("images/previews/2026/11/0f1e2d3c.webp", pattern(300, 2)),
        ("audio/2026/11/aa.webm", pattern(70_000, 3)),
        ("videos/2026/11/bb.mp4", pattern(1_000_000, 4)),
        ("files/2026/11/cc.pdf", Vec::new()),
        ("encrypted/2026/11/dd.bin", pattern(10, 5)),
        ("embeds/2026/11/ee.jpg", pattern(10, 6)),
    ];
    for (key, bytes) in &objects {
        put(&f.store, key, bytes).await.unwrap();
    }
    fn walk(folder: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let name = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((name, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut found = Vec::new();
    walk(&f.root(), &f.root(), &mut found);
    found.sort();
    let mut expected: Vec<(String, Vec<u8>)> = objects
        .iter()
        .map(|(k, b)| (k.to_string(), b.clone()))
        .collect();
    expected.sort();
    assert_eq!(found, expected);
    assert!(
        f.incoming().is_dir(),
        "the staging folder is where it always was"
    );

    // Permissions are the process's defaults: readable by the group and the
    // world as an ordinary file is.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = std::fs::metadata(f.root().join("audio/2026/11/aa.webm")).unwrap();
        assert_eq!(file.permissions().mode() & 0o600, 0o600);
        assert!(!file.permissions().readonly());
    }
}

#[tokio::test]
async fn a_staged_file_is_named_by_32_hex_digits_in_the_incoming_folder() {
    let f = Fixture::new();
    let (mut sender, mut receiver) = tokio::io::duplex(1024);
    let store = f.store.clone();
    let writing = tokio::spawn(async move { store.put("named/one.bin", &mut receiver).await });
    sender.write_all(b"half").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let staged = names_in(&f.incoming());
    assert_eq!(staged.len(), 1, "{staged:?}");
    assert_eq!(staged[0].len(), 32);
    assert!(
        staged[0]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    drop(sender);
    writing.await.unwrap().unwrap();
    assert!(names_in(&f.incoming()).is_empty());
}

#[tokio::test]
async fn a_key_that_leaves_the_folder_creates_nothing_anywhere() {
    let f = Fixture::new();
    for key in [
        "../outside",
        "a/../../outside",
        "/tmp/stellarr-storage-escape",
        ".incoming/x",
    ] {
        assert!(matches!(
            put(&f.store, key, b"x").await,
            Err(Error::InvalidKey(_))
        ));
    }
    assert!(!f.dir.path().join("outside").exists());
    assert!(!Path::new("/tmp/stellarr-storage-escape").exists());
    assert!(names_in(&f.incoming()).is_empty());
}

#[tokio::test]
async fn the_listing_skips_files_still_being_written() {
    let f = Fixture::new();
    put(&f.store, "done/one.bin", b"0123456789").await.unwrap();
    std::fs::write(f.incoming().join("unfinished"), "half").unwrap();
    let listed = f.store.list().collect().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "done/one.bin");
}

#[tokio::test]
async fn a_nested_folder_named_like_the_staging_folder_is_listed() {
    let f = Fixture::new();
    put(&f.store, "a/.incoming/b", b"x").await.unwrap();
    assert_eq!(
        f.store.list().collect().await.unwrap()[0].key,
        "a/.incoming/b"
    );
}

#[tokio::test]
async fn preparing_clears_what_an_interrupted_write_left_behind() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.incoming()).unwrap();
    std::fs::write(f.incoming().join("unfinished"), "half").unwrap();
    f.store.prepare().await.unwrap();
    assert!(f.incoming().is_dir());
    assert!(names_in(&f.incoming()).is_empty());
}

/// The power goes between the end of the staged file and its move: the staged
/// file is on the disk, no object has its key. The next start clears it, and
/// the key names nothing.
#[tokio::test]
async fn a_crash_between_staging_and_the_move_loses_only_the_unfinished_write() {
    let f = Fixture::new();
    put(&f.store, "kept/one.bin", b"safe").await.unwrap();
    std::fs::write(
        f.incoming().join("0123456789abcdef0123456789abcdef"),
        pattern(1_000, 1),
    )
    .unwrap();

    let restarted = FolderStore::new(f.root());
    assert!(matches!(
        restarted.get("lost/two.bin", None).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(restarted.list().collect().await.unwrap().len(), 1);
    restarted.prepare().await.unwrap();
    assert!(names_in(&f.incoming()).is_empty());
    assert_eq!(
        read(restarted.get("kept/one.bin", None).await.unwrap()).await,
        b"safe"
    );
}

/// The power goes after the link and before the staged name is removed: the
/// object has its key, and a second name of the same file is still staged.
/// Clearing that name must not touch the object.
#[tokio::test]
async fn a_crash_between_the_link_and_the_clean_up_keeps_the_object() {
    let f = Fixture::new();
    let bytes = pattern(50_000, 3);
    put(&f.store, "linked/one.bin", &bytes).await.unwrap();
    std::fs::hard_link(
        f.root().join("linked/one.bin"),
        f.incoming().join("ffffffffffffffffffffffffffffffff"),
    )
    .unwrap();

    let restarted = FolderStore::new(f.root());
    assert_eq!(restarted.list().collect().await.unwrap().len(), 1);
    restarted.prepare().await.unwrap();
    assert!(names_in(&f.incoming()).is_empty());
    assert_eq!(
        read(restarted.get("linked/one.bin", None).await.unwrap()).await,
        bytes
    );
}

#[tokio::test]
async fn a_store_whose_folder_is_gone_fails_the_probe_and_lists_nothing() {
    let f = Fixture::new();
    f.store.prepare().await.unwrap();
    std::fs::remove_dir_all(f.root()).unwrap();
    assert!(
        matches!(f.store.probe().await, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound)
    );
    assert!(f.store.list().collect().await.unwrap().is_empty());
    // And a read finds nothing rather than failing.
    assert!(matches!(
        f.store.get("a/b", None).await,
        Err(Error::NotFound(_))
    ));
    f.store.delete("a/b").await.unwrap();
}

#[tokio::test]
async fn a_store_whose_folder_is_a_file_fails_the_probe() {
    let dir = TempDir::new("folder-file");
    std::fs::write(dir.path().join("files"), "not a folder").unwrap();
    let store = FolderStore::new(dir.path().join("files"));
    assert!(store.probe().await.is_err());
    assert!(store.prepare().await.is_err());
}

#[tokio::test]
async fn a_file_deleted_by_hand_is_not_found() {
    let f = Fixture::new();
    put(&f.store, "by-hand/gone.bin", b"x").await.unwrap();
    std::fs::remove_file(f.root().join("by-hand/gone.bin")).unwrap();
    assert!(matches!(
        f.store.get("by-hand/gone.bin", None).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn writing_works_before_the_store_is_prepared() {
    let f = Fixture::new();
    assert!(!f.root().exists());
    put(&f.store, "early/one.bin", b"x").await.unwrap();
    assert_eq!(
        read(f.store.get("early/one.bin", None).await.unwrap()).await,
        b"x"
    );
}

#[tokio::test]
async fn a_folder_of_other_keys_is_not_an_object_for_any_call() {
    let f = Fixture::new();
    put(&f.store, "tree/leaf.bin", b"leaf").await.unwrap();
    assert!(matches!(
        put(&f.store, "tree", b"x").await,
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        f.store.get("tree", None).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        f.store
            .get("tree", Some(ByteRange { from: 0, to: None }))
            .await,
        Err(Error::NotFound(_))
    ));
    f.store.delete("tree").await.unwrap();
    // A file where a folder of the key's path has to be.
    assert!(matches!(
        put(&f.store, "tree/leaf.bin/deeper", b"x").await,
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        f.store.get("tree/leaf.bin/deeper", None).await,
        Err(Error::NotFound(_))
    ));
    f.store.delete("tree/leaf.bin/deeper").await.unwrap();
    assert_eq!(
        read(f.store.get("tree/leaf.bin", None).await.unwrap()).await,
        b"leaf"
    );
    assert!(names_in(&f.incoming()).is_empty());
}

#[tokio::test]
async fn a_file_name_too_long_for_the_disk_is_an_error_not_a_panic() {
    let f = Fixture::new();
    let key = format!("long/{}", "n".repeat(300));
    assert!(matches!(put(&f.store, &key, b"x").await, Err(Error::Io(_))));
    assert!(matches!(
        f.store.get(&key, None).await,
        Err(Error::NotFound(_))
    ));
    f.store.delete(&key).await.unwrap();
    assert!(names_in(&f.incoming()).is_empty());
}

#[tokio::test]
async fn a_file_cut_short_while_it_is_read_is_an_error_not_a_short_object() {
    let f = Fixture::new();
    put(&f.store, "cut/one.bin", &pattern(1_000_000, 1))
        .await
        .unwrap();
    let mut object = f.store.get("cut/one.bin", None).await.unwrap();
    object.body.chunk().await.unwrap().unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(f.root().join("cut/one.bin"))
        .unwrap()
        .set_len(10)
        .unwrap();
    let mut failed = false;
    while let Some(result) = object.body.chunk().await.transpose() {
        if result.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_writers_to_different_keys_all_land_and_nothing_stays_staged() {
    let f = Fixture::new();
    let mut tasks = Vec::new();
    for n in 0..64 {
        let store = f.store.clone();
        tasks.push(tokio::spawn(async move {
            put(
                &store,
                &format!("many/{n:03}/file.bin"),
                &pattern(20_000 + n, n as u64),
            )
            .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert_eq!(f.store.list().collect().await.unwrap().len(), 64);
    assert!(names_in(&f.incoming()).is_empty());
}

#[tokio::test]
async fn a_failing_source_leaves_the_incoming_folder_empty_and_the_key_free() {
    let f = Fixture::new();
    let mut failing = FailingReader { before_failure: 4 };
    assert!(matches!(
        f.store.put("failed/upload.bin", &mut failing).await,
        Err(Error::Io(_))
    ));
    assert!(!f.root().join("failed/upload.bin").exists());
    assert!(names_in(&f.incoming()).is_empty());
    put(&f.store, "failed/upload.bin", b"second try")
        .await
        .unwrap();
}

#[tokio::test]
async fn the_store_can_be_held_as_a_shared_trait_object_across_tasks() {
    let f = Fixture::new();
    let shared: Arc<dyn ObjectStorage> = Arc::new(f.store.clone());
    let task = tokio::spawn({
        let shared = shared.clone();
        async move { put(&*shared, "shared/one.bin", b"x").await }
    });
    task.await.unwrap().unwrap();
    assert_eq!(shared.list().collect().await.unwrap().len(), 1);
}

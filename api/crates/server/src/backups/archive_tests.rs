//! What an archive holds, in which order, and what verifying it catches.

use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use stellarr_archive::ZipArchive;
use stellarr_storage::{ObjectStorage, StoredObjectInfo};

use super::archive::{self, Sources};
use super::fixtures::Instance;
use super::parts::Parts;
use super::progress::Progress;
use crate::testing::TempDir;

fn names(path: &Path) -> Vec<String> {
    let zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    zip.entries().iter().map(|entry| entry.name_lossy()).collect()
}

#[test]
fn a_full_backup_holds_the_database_every_object_the_keys_and_the_instance_files_in_order() {
    let instance = Instance::new();
    let folder = TempDir::new();
    let path = folder.path().join("full.zip");
    let manifest = instance.archive(Parts::ALL, &path);

    assert_eq!(
        names(&path),
        [
            "database/stellarr.db",
            "objects/audio/b.ogg",
            "objects/c.bin",
            "objects/images/2026/a.png",
            "keys/master.key",
            // Folder by folder, and inside a folder by the whole path as
            // text: `a-b/x` comes before `a/x` because `-` sorts before `/`.
            "instance/secrets/turn-shared-secret",
            "instance/tls/fullchain.pem",
            "instance/tls/privkey.pem",
            "instance/releases/a-b/x",
            "instance/releases/a/x",
            "instance/releases/desktop/1.0/app.AppImage",
            "manifest.json",
        ]
    );
    assert_eq!(manifest.parts, ["database", "objects", "keys", "instance"]);
    assert_eq!((manifest.object_count, manifest.key_files, manifest.instance_files), (3, 1, 6));
    assert_eq!(
        manifest.object_bytes,
        super::fixtures::OBJECTS.iter().map(|(_, bytes)| bytes.len() as i64).sum::<i64>()
    );
    assert!(manifest.missing_objects.is_empty());
    assert_eq!(manifest.environment_secrets, ["STELLARR_MAIL_RESEND_API_KEY"]);
    assert_eq!(archive::verify(&path, folder.path()), "ok");
    // The verification left nothing of its own beside the archive.
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);

    // Objects are stored, as they mostly are compressed already; the rest is
    // deflated.
    let zip = ZipArchive::new(File::open(&path).unwrap()).unwrap();
    for entry in zip.entries() {
        let name = entry.name_lossy();
        let stored = name.starts_with("objects/");
        assert_eq!(
            entry.method() == stellarr_archive::Method::Stored,
            stored,
            "{name}"
        );
    }
}

#[test]
fn only_the_parts_asked_for_are_in_the_archive() {
    let instance = Instance::new();
    let folder = TempDir::new();
    let path = folder.path().join("some.zip");
    let manifest = instance.archive(Parts::DATABASE.with(Parts::KEYS), &path);
    assert_eq!(names(&path), ["database/stellarr.db", "keys/master.key", "manifest.json"]);
    assert_eq!(manifest.parts, ["database", "keys"]);
    assert_eq!(archive::verify(&path, folder.path()), "ok");
}

#[test]
fn a_damaged_archive_does_not_verify() {
    let instance = Instance::new();
    let folder = TempDir::new();
    let path = folder.path().join("full.zip");
    instance.archive(Parts::ALL, &path);
    let whole = fs::read(&path).unwrap();

    let damaged = folder.path().join("damaged.zip");
    let mut bytes = whole.clone();
    let at = bytes
        .windows(10)
        .position(|window| window == b"OggS bytes")
        .unwrap();
    bytes[at + 2] ^= 0xff;
    fs::write(&damaged, &bytes).unwrap();
    assert!(archive::verify(&damaged, folder.path()).contains("checksum"));

    fs::write(&damaged, &whole[..whole.len() / 2]).unwrap();
    assert_ne!(archive::verify(&damaged, folder.path()), "ok");
    fs::write(&damaged, b"not an archive").unwrap();
    assert_ne!(archive::verify(&damaged, folder.path()), "ok");
    assert!(archive::read_manifest(&damaged).is_none());
}

fn sources(instance: &Instance, objects: Vec<StoredObjectInfo>) -> Sources {
    Sources {
        database: instance.database_path(),
        store: instance.store.clone(),
        objects,
        keys_folder: instance.root().join("keys"),
        instance_name: "x".into(),
        environment_secrets: vec![],
        work_folder: instance.root().to_path_buf(),
        data_root: instance.root().to_path_buf(),
        database_snapshot: None,
    }
}

fn write(instance: &Instance, sources: &Sources, at: &Path) -> Result<super::manifest::Manifest, super::BackupFailed> {
    archive::write(
        at,
        Parts::ALL,
        sources,
        &Progress::new(0),
        &AtomicBool::new(false),
        instance.runtime.handle(),
    )
}

#[test]
fn an_object_the_store_cannot_give_is_listed_and_does_not_fail_the_backup() {
    let instance = Instance::new();
    let folder = TempDir::new();
    let mut objects = instance.objects();
    objects.push(StoredObjectInfo { key: "gone/object".into(), size: 9 });
    // A key that can never be an entry of an archive is no object either.
    objects.push(StoredObjectInfo { key: "../unsafe".into(), size: 9 });
    let path = folder.path().join("a.zip");
    let manifest = write(&instance, &sources(&instance, objects), &path).unwrap();
    assert_eq!(manifest.missing_objects, ["gone/object", "../unsafe"]);
    assert_eq!(manifest.object_count, 3);
    assert_eq!(archive::verify(&path, folder.path()), "ok");
}

#[test]
fn when_no_object_can_be_read_the_store_is_the_problem() {
    let instance = Instance::new();
    let folder = TempDir::new();
    let objects = vec![StoredObjectInfo { key: "gone/object".into(), size: 9 }];
    let path = folder.path().join("a.zip");
    let failure = write(&instance, &sources(&instance, objects), &path).unwrap_err();
    assert_eq!(failure.message(), "No object could be read from the object store.");
}

#[test]
fn a_manifest_that_counts_wrong_or_a_database_that_is_missing_is_found_by_verifying() {
    let folder = TempDir::new();
    let db = super::fixtures::database_bytes();
    // Counts: the manifest says one object, the archive holds none.
    let path = folder.path().join("counts.zip");
    super::fixtures::archive_of(
        &[
            ("database/stellarr.db", &db, None),
            (
                "manifest.json",
                br#"{"format":"stellarr-backup","version":1,"createdAt":"2026-11-24T10:00:00Z","parts":["database","objects"],"objectCount":1}"#,
                None,
            ),
        ],
        &path,
    );
    assert_eq!(
        archive::verify(&path, folder.path()),
        "The archive holds 0 objects; its manifest names 1."
    );
    // The database a manifest promises is not there.
    let path = folder.path().join("nodb.zip");
    super::fixtures::archive_of(&[("objects/a", b"x", None)], &path);
    assert_eq!(
        archive::verify(&path, folder.path()),
        "The database is missing from the archive."
    );
    // No manifest.
    let path = folder.path().join("nomanifest.zip");
    let mut zip = stellarr_archive::ZipWriter::new(File::create(&path).unwrap());
    zip.add_bytes("objects/a", stellarr_archive::EntryOptions::stored(), b"x")
        .unwrap();
    zip.finish().unwrap();
    assert_eq!(
        archive::verify(&path, folder.path()),
        "The archive has no readable manifest."
    );
}

#[test]
fn a_copy_of_a_database_that_is_written_to_is_a_whole_state() {
    let instance = Instance::new();
    let folder = TempDir::new();
    let copy = folder.path().join("copy.db");
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let writer = {
        let (database, stop) = (instance.database.clone(), stop.clone());
        let runtime = instance.runtime.handle().clone();
        std::thread::spawn(move || {
            let mut count = 0i64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                count += 1;
                runtime
                    .block_on(database.write(move |tx| -> Result<(), crate::database::DbError> {
                        // Two rows that belong together, in one transaction.
                        tx.execute("INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, 'a', '2026-11-24 00:00:00')", [format!("pair-{count}-a")])?;
                        tx.execute("INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, 'b', '2026-11-24 00:00:00')", [format!("pair-{count}-b")])?;
                        Ok(())
                    }))
                    .unwrap();
            }
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    super::snapshot::copy_database(&instance.database_path(), &copy).unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.join().unwrap();

    assert_eq!(super::snapshot::check_integrity(&copy), "ok");
    let connection = rusqlite::Connection::open(&copy).unwrap();
    let (halves, pairs): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT substr(\"Name\", 1, length(\"Name\") - 2)) FROM \"InstanceSecrets\" WHERE \"Name\" LIKE 'pair-%'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(pairs > 0, "the writer wrote before the copy");
    assert_eq!(halves, pairs * 2, "no pair is cut in half");
    // A copy is never made over a file that is there.
    assert!(super::snapshot::copy_database(&instance.database_path(), &copy).is_err());
    assert!(copy.exists());
    // And one that fails leaves nothing.
    let missing = folder.path().join("missing.db");
    assert!(super::snapshot::copy_database(&folder.path().join("none.db"), &missing).is_err());
    assert!(!missing.exists());
    let _: &dyn ObjectStorage = &*instance.store;
}

/// A backup of an instance whose objects are in a bucket, against a real
/// server (`tests/backups-bucket.sh` starts one and sets the variables): the
/// objects are listed, read through the bucket, written into the archive, and
/// come back byte for byte from a restore, a large one among them.
#[test]
#[ignore = "needs an S3 server: tests/backups-bucket.sh"]
fn a_backup_of_a_bucket_holds_its_objects_and_restores_them() {
    use stellarr_storage::{BucketStore, StorageOptions, StorageProvider};

    let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let bucket = format!("backup-source-{}", super::random_token());
    let store: std::sync::Arc<dyn ObjectStorage> = std::sync::Arc::new(
        BucketStore::new(
            &StorageOptions {
                provider: StorageProvider::S3,
                endpoint: env("STELLARR_TEST_S3_ENDPOINT"),
                access_key: env("STELLARR_TEST_S3_ACCESS_KEY"),
                secret_key: env("STELLARR_TEST_S3_SECRET_KEY"),
                bucket,
            },
            stellarr_outbound::Client::new(),
        )
        .unwrap(),
    );
    let instance = Instance::new();
    let rt = instance.runtime.handle().clone();
    rt.block_on(store.prepare()).unwrap();
    // Small ones, an empty one and one big enough to have been sent in parts.
    let large: Vec<u8> = (0..9 * 1024 * 1024u32).map(|at| (at % 251) as u8).collect();
    let mut wanted: Vec<(String, Vec<u8>)> = vec![
        ("images/2026/a.png".into(), b"png bytes".to_vec()),
        ("audio/b.ogg".into(), b"OggS".to_vec()),
        ("empty".into(), Vec::new()),
        ("video/large.bin".into(), large),
    ];
    for (key, bytes) in &wanted {
        rt.block_on(store.put(key, &mut std::io::Cursor::new(bytes.clone())))
            .unwrap();
    }
    wanted.sort_by(|a, b| archive::utf16_order(&a.0, &b.0));

    let listed = rt.block_on(store.list().collect()).unwrap();
    assert_eq!(
        listed.iter().map(|object| object.key.as_str()).collect::<Vec<_>>(),
        wanted.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>()
    );
    let folder = TempDir::new();
    let path = folder.path().join("bucket.zip");
    let mut from = sources(&instance, listed);
    from.store = store.clone();
    let manifest = write(&instance, &from, &path).unwrap();
    assert_eq!(manifest.object_count, 4);
    assert!(manifest.missing_objects.is_empty());
    assert_eq!(archive::verify(&path, folder.path()), "ok");

    let target = Instance::new();
    super::restore::apply(
        &path,
        target.root(),
        crate::values::Timestamp::parse("2026-11-24 10:00:00").unwrap(),
    )
    .unwrap();
    for (key, bytes) in &wanted {
        assert_eq!(&fs::read(target.root().join("files").join(key)).unwrap(), bytes, "{key}");
    }
    for (key, _) in &wanted {
        rt.block_on(store.delete(key)).unwrap();
    }
}

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::backups::fixtures::{Instance, archive_of, database_bytes};
use crate::backups::parts::Parts;
use crate::testing::TempDir;

fn now() -> Timestamp {
    Timestamp::parse("2026-11-24T10:15:30Z").unwrap()
}

fn listing(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn without_database(files: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    files
        .into_iter()
        .filter(|(path, _)| path != "db/stellarr.db")
        .collect()
}

#[test]
fn a_backup_is_restored_over_another_instance_and_the_old_data_is_kept_aside() {
    let source = Instance::new();
    let archive_folder = TempDir::new();
    let archive = archive_folder.path().join("backup.zip");
    let manifest = source.archive(Parts::ALL, &archive);
    assert_eq!(manifest.object_count, 3);

    let target = Instance::new();
    target.write_rows("second");
    fs::write(target.root().join("keys/other"), b"x").unwrap();
    fs::create_dir_all(target.root().join("logs")).unwrap();
    fs::write(target.root().join("logs/today.log"), b"log").unwrap();
    let before = target.files();

    let result = apply(&archive, target.root(), now()).unwrap();
    assert_eq!(result.manifest.instance_name, "Test instance");
    assert_eq!(
        result.previous_data_directory,
        target.root().join("restore/previous-20261124-101530")
    );

    // What was restored is what the source had; the logs stayed.
    let restored = target.files();
    assert_eq!(
        without_database(restored.clone())
            .into_iter()
            .filter(|(path, _)| path != "logs/today.log")
            .collect::<Vec<_>>(),
        without_database(source.files())
    );
    assert!(restored.iter().any(|(path, _)| path == "logs/today.log"));
    assert_eq!(target.mark(), "first");
    // And what was replaced is whole in the folder set aside.
    let aside = result.previous_data_directory;
    assert_eq!(fs::read(aside.join("keys/other")).unwrap(), b"x");
    let old = rusqlite::Connection::open(aside.join("db/stellarr.db")).unwrap();
    let mark: String = old
        .query_row(
            "SELECT \"ProtectedValue\" FROM \"InstanceSecrets\" WHERE \"Name\" = 'mark'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mark, "second");
    assert!(before.iter().any(|(path, _)| path == "keys/other"));
    // The database is the first writer's and nobody else's.
    let check = rusqlite::Connection::open(target.database_path()).unwrap();
    let verdict: String = check
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(verdict, "ok");
}

#[cfg(unix)]
#[test]
fn everything_that_holds_a_secret_is_its_owners_alone_from_the_moment_it_is_made() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let source = Instance::new();
    // Something a person put in a folder of the instance, with a folder in it.
    fs::create_dir_all(source.root().join("secrets/nested")).unwrap();
    fs::write(source.root().join("secrets/nested/token"), b"t").unwrap();
    let archive_folder = TempDir::new();
    let archive = archive_folder.path().join("backup.zip");
    source.archive(Parts::ALL, &archive);
    // The archive, and the copy of the database it was made from.
    assert_eq!(mode(&archive), 0o600);
    let copy = archive_folder.path().join("copy.db");
    crate::backups::snapshot::copy_database(&source.database_path(), &copy).unwrap();
    assert_eq!(mode(&copy), 0o600);
    // The staged archive.
    let target = Instance::new();
    let staged = stage(&archive, target.root()).unwrap();
    assert_eq!(mode(&staged), 0o600);

    apply(&archive, target.root(), now()).unwrap();
    for private in [
        "keys/master.key",
        "secrets/turn-shared-secret",
        "secrets/nested/token",
        "tls/privkey.pem",
        "db/stellarr.db",
    ] {
        assert_eq!(mode(&target.root().join(private)), 0o600, "{private}");
    }
    for folder in ["keys", "secrets", "secrets/nested", "tls", "db"] {
        assert_eq!(mode(&target.root().join(folder)), 0o700, "{folder}");
    }
}

/// A data root inside a folder of its own, so that a file written outside
/// the data root is seen.
struct Place {
    outer: TempDir,
}

impl Place {
    fn new() -> Place {
        let outer = TempDir::new();
        fs::write(outer.path().join("sentinel"), b"s").unwrap();
        Place { outer }
    }

    fn data(&self) -> PathBuf {
        self.outer.path().join("data")
    }
}

/// Gives `archive` to a restore on an instance and says nothing changed:
/// not the instance's files, and nothing next to its data folder.
fn refused_and_unchanged(entries: &[(&str, &[u8], Option<u32>)], needle: &str) {
    let archives = TempDir::new();
    let archive = archives.path().join("hostile.zip");
    archive_of(entries, &archive);
    refused_file_and_unchanged(&archive, needle);
}

fn refused_file_and_unchanged(archive: &Path, needle: &str) {
    let place = Place::new();
    let instance = Instance::new();
    let data = place.data();
    fs::rename(instance.root(), &data).unwrap_or_else(|_| {
        copy_tree(instance.root(), &data);
    });
    let outside = listing(place.outer.path());
    let inside = listing(&data);
    let files = snapshot(&data);

    let outcome = apply(archive, &data, now());
    match outcome {
        Err(RestoreError::Refused(message)) => {
            assert!(message.contains(needle), "{message} does not say {needle}");
        }
        other => panic!("not refused: {other:?}"),
    }
    assert_eq!(listing(place.outer.path()), outside);
    assert_eq!(snapshot(&data), files);
    let mut now_inside = listing(&data);
    now_inside.retain(|name| name != "restore");
    let mut then_inside = inside;
    then_inside.retain(|name| name != "restore");
    assert_eq!(now_inside, then_inside);
    if let Ok(left) = fs::read_dir(data.join("restore")) {
        for entry in left.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.starts_with(".unpacking") && !name.starts_with("previous"),
                "{name} was left behind"
            );
        }
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn snapshot(folder: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    fn walk(root: &Path, folder: &Path, found: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(folder).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, found);
            } else if !path.to_string_lossy().ends_with("-wal") && !path.to_string_lossy().ends_with("-shm") {
                found.push((
                    path.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    walk(folder, folder, &mut found);
    found.sort();
    found
}

#[test]
fn a_name_that_could_climb_out_of_its_folder_is_refused_and_nothing_is_written() {
    let db = database_bytes();
    for name in [
        "objects/../../evil",
        "objects/../evil",
        "objects/a/../../../evil",
        "/etc/evil",
        "/objects/a",
        "objects//a",
        "objects/./a",
        "C:\\evil",
        "objects/C:\\evil",
        "objects/a\\b",
        "keys/../../evil",
        "keys/sub/file",
        "instance/secrets/../../evil",
        "instance/secrets",
        "instance/other/file",
        "instance/../evil",
        "instance//secrets/x",
        "database/other.db",
        "database/../../evil",
        "objects/.incoming/x",
        "objects/.hidden",
        "x",
        "../evil",
    ] {
        refused_and_unchanged(
            &[(name, b"data", None), ("database/stellarr.db", &db, None)],
            "no part of a backup puts there",
        );
    }
}

#[test]
fn a_folder_entry_is_no_file_of_a_backup() {
    let db = database_bytes();
    refused_and_unchanged(
        &[("objects/", b"", None), ("database/stellarr.db", &db, None)],
        "kind of file",
    );
}

#[test]
fn a_link_is_refused_whatever_it_points_at() {
    let db = database_bytes();
    for target in ["/etc/passwd", "../../outside", "x"] {
        refused_and_unchanged(
            &[
                ("objects/link", target.as_bytes(), Some(0o120_777)),
                ("database/stellarr.db", &db, None),
            ],
            "a link",
        );
    }
    refused_and_unchanged(
        &[("objects/device", b"", Some(0o020_666)), ("database/stellarr.db", &db, None)],
        "a link or another kind of file",
    );
}

#[test]
fn a_name_given_twice_is_refused() {
    let db = database_bytes();
    refused_and_unchanged(
        &[
            ("database/stellarr.db", &db, None),
            ("objects/a", b"one", None),
            ("objects/a", b"two", None),
        ],
        "twice",
    );
}

#[test]
fn a_name_with_a_nul_in_it_is_refused() {
    let db = database_bytes();
    let archives = TempDir::new();
    let archive = archives.path().join("nul.zip");
    archive_of(
        &[("database/stellarr.db", &db, None), ("objects/aXb", b"data", None)],
        &archive,
    );
    let mut bytes = fs::read(&archive).unwrap();
    let mut found = 0;
    let needle = b"objects/aXb";
    let mut at = 0;
    while at + needle.len() <= bytes.len() {
        if &bytes[at..at + needle.len()] == needle {
            bytes[at + 9] = 0;
            found += 1;
            at += needle.len();
        } else {
            at += 1;
        }
    }
    assert_eq!(found, 2, "the name is in the local and the central header");
    fs::write(&archive, bytes).unwrap();
    refused_file_and_unchanged(&archive, "no part of a backup puts there");
}

fn central_field(bytes: &mut [u8], name: &[u8], offset: usize, value: u32) {
    let mut at = 0;
    while at + 46 < bytes.len() {
        if bytes[at..at + 4] == [0x50, 0x4b, 0x01, 0x02] {
            let length = u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]) as usize;
            if &bytes[at + 46..at + 46 + length] == name {
                bytes[at + offset..at + offset + 4].copy_from_slice(&value.to_le_bytes());
                return;
            }
        }
        at += 1;
    }
    panic!("no such entry");
}

#[test]
fn a_size_that_lies_is_refused_before_anything_is_unpacked() {
    let db = database_bytes();
    let archives = TempDir::new();
    for (how, offset, value) in [
        ("larger", 24, 5_000_000u32),
        ("gigabytes in a tiny file", 24, 0xffff_fffe),
        ("smaller", 24, 2),
    ] {
        let archive = archives.path().join(format!("lie-{offset}-{value}.zip"));
        archive_of(
            &[("database/stellarr.db", &db, None), ("objects/a", b"hello", None)],
            &archive,
        );
        let mut bytes = fs::read(&archive).unwrap();
        central_field(&mut bytes, b"objects/a", offset, value);
        fs::write(&archive, bytes).unwrap();
        let _ = how;
        refused_file_and_unchanged(&archive, "");
    }
}

#[test]
fn a_wrong_checksum_is_refused() {
    let db = database_bytes();
    let archives = TempDir::new();
    let archive = archives.path().join("crc.zip");
    archive_of(
        &[("database/stellarr.db", &db, None), ("objects/a", b"hello", None)],
        &archive,
    );
    let mut bytes = fs::read(&archive).unwrap();
    central_field(&mut bytes, b"objects/a", 16, 0x1234_5678);
    fs::write(&archive, bytes).unwrap();
    refused_file_and_unchanged(&archive, "failed its checksum or length check");

    // And a byte of an object that changed after it was written.
    let archive = archives.path().join("flipped.zip");
    archive_of(
        &[("database/stellarr.db", &db, None), ("objects/a", b"hello", None)],
        &archive,
    );
    let mut bytes = fs::read(&archive).unwrap();
    let at = bytes.windows(5).position(|window| window == b"hello").unwrap();
    bytes[at] ^= 1;
    fs::write(&archive, bytes).unwrap();
    refused_file_and_unchanged(&archive, "checksum");
}

#[test]
fn a_truncated_archive_is_refused_wherever_it_is_cut() {
    let source = Instance::new();
    let archives = TempDir::new();
    let archive = archives.path().join("full.zip");
    source.archive(Parts::ALL, &archive);
    let whole = fs::read(&archive).unwrap();
    for keep in [0, 10, 100, whole.len() / 2, whole.len() - 30, whole.len() - 1] {
        let cut = archives.path().join(format!("cut-{keep}.zip"));
        fs::write(&cut, &whole[..keep]).unwrap();
        refused_file_and_unchanged(&cut, "");
    }
}

#[test]
fn a_database_that_is_not_one_is_refused() {
    refused_and_unchanged(
        &[("database/stellarr.db", b"this is not a database at all, not even close", None)],
        "failed its check",
    );
    let mut damaged = database_bytes();
    let middle = damaged.len() / 2;
    for byte in &mut damaged[middle..] {
        *byte = 0xA5;
    }
    refused_and_unchanged(&[("database/stellarr.db", &damaged, None)], "failed its check");
}

#[test]
fn only_a_full_backup_of_this_format_is_restored() {
    let db = database_bytes();
    let entry = |manifest: &'static str| -> Vec<(&'static str, Vec<u8>, Option<u32>)> {
        vec![
            ("database/stellarr.db", db.clone(), None),
            ("manifest.json", manifest.as_bytes().to_vec(), None),
        ]
    };
    let cases: [(&str, &str); 6] = [
        (
            r#"{"format":"stellarr-backup","version":1,"createdAt":"2026-11-24T10:00:00Z","parts":["database"]}"#,
            "lacks objects, keys, instance",
        ),
        (
            r#"{"format":"stellarr-backup","version":2,"createdAt":"2026-11-24T10:00:00Z","parts":["database","objects","keys","instance"]}"#,
            "format 2",
        ),
        (
            r#"{"format":"other","version":1,"createdAt":"2026-11-24T10:00:00Z"}"#,
            "not a Stellarr backup's",
        ),
        ("not json", "not a readable backup archive"),
        ("[]", "not a readable backup archive"),
        (
            r#"{"format":"stellarr-backup","version":1,"createdAt":"2026-11-24T10:00:00Z","parts":["database","objects","keys","instance"],"objectCount":4}"#,
            "holds 0 objects",
        ),
    ];
    for (manifest, needle) in cases {
        let owned = entry(manifest);
        let borrowed: Vec<(&str, &[u8], Option<u32>)> = owned
            .iter()
            .map(|(name, bytes, mode)| (*name, bytes.as_slice(), *mode))
            .collect();
        refused_and_unchanged(&borrowed, needle);
    }
    // No manifest at all: not an archive of this program.
    let archives = TempDir::new();
    let archive = archives.path().join("bare.zip");
    let mut zip = stellarr_archive::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.add_bytes("database/stellarr.db", stellarr_archive::EntryOptions::stored(), &db)
        .unwrap();
    zip.finish().unwrap();
    refused_file_and_unchanged(&archive, "has no manifest");
    // A database alone is no archive.
    let plain = archives.path().join("plain.db");
    fs::write(&plain, &db).unwrap();
    refused_file_and_unchanged(&plain, "database-only backup");
}

#[test]
fn a_swap_that_fails_puts_the_previous_data_back_where_it_was() {
    let source = Instance::new();
    let archives = TempDir::new();
    let archive = archives.path().join("backup.zip");
    source.archive(Parts::ALL, &archive);
    let target = Instance::new();
    // A file where the secrets folder is: the folder that is unpacked cannot
    // take its place.
    fs::remove_dir_all(target.root().join("secrets")).unwrap();
    fs::write(target.root().join("secrets"), b"not a folder").unwrap();
    let before = snapshot(target.root());

    let outcome = apply(&archive, target.root(), now());
    assert!(matches!(outcome, Err(RestoreError::Refused(_))), "{outcome:?}");
    assert_eq!(snapshot(target.root()), before);
    assert!(!target.root().join("restore").join("previous-20261124-101530").exists());
}

#[test]
fn a_swap_that_cannot_be_undone_says_so_and_where_everything_is() {
    let source = Instance::new();
    let archives = TempDir::new();
    let archive = archives.path().join("backup.zip");
    source.archive(Parts::ALL, &archive);
    let target = Instance::new();
    let calls = std::cell::Cell::new(0);
    // Every move after the fifth fails, the undoing ones too.
    let outcome = apply_with(&archive, target.root(), now(), &|from, to| {
        calls.set(calls.get() + 1);
        if calls.get() > 5 {
            return Err(std::io::Error::other("the volume went away"));
        }
        fs::rename(from, to)
    });
    let Err(RestoreError::Broken(message)) = outcome else {
        panic!("not broken: {outcome:?}");
    };
    assert!(message.contains("the volume went away"), "{message}");
    assert!(message.contains("previous-20261124-101530"), "{message}");
    // What marks a volume as broken, and keeps a refused archive, is the
    // tool's own code (`stellarr backup apply`), not this library's.
    assert_eq!(BROKEN_MARKER_NAME, "broken");
}

#[test]
fn staging_copies_the_archive_whole_and_cancelling_takes_it_away() {
    let source = Instance::new();
    let archives = TempDir::new();
    let archive = archives.path().join("backup.zip");
    source.archive(Parts::ALL, &archive);
    let target = Instance::new();
    // What a stop left in the restore folder is cleared first.
    fs::create_dir_all(target.root().join("restore/.unpacking-x/files")).unwrap();
    fs::write(target.root().join("restore/.verify-x.partial"), b"x").unwrap();
    fs::write(target.root().join("restore/staged.zip.abc.partial"), b"x").unwrap();
    fs::write(target.root().join("restore/previous-1"), b"keep").unwrap();

    let staged = stage(&archive, target.root()).unwrap();
    assert_eq!(staged, staged_archive_path(target.root()));
    assert_eq!(fs::read(&staged).unwrap(), fs::read(&archive).unwrap());
    assert_eq!(
        listing(&target.root().join("restore")),
        ["previous-1", "staged.zip"]
    );
    assert!(cancel(target.root()).unwrap());
    assert!(!cancel(target.root()).unwrap());
    assert!(archive.exists(), "the archive itself is left alone");

    write_result(target.root(), &serde_json::json!({"outcome": "refused"})).unwrap();
    assert!(
        fs::read_to_string(target.root().join("restore/last-restore.json"))
            .unwrap()
            .contains("refused")
    );
}

#[test]
fn a_second_restore_in_the_same_second_gets_a_folder_of_its_own() {
    let folder = TempDir::new();
    let first = new_previous_directory(folder.path(), now());
    fs::create_dir_all(&first).unwrap();
    let second = new_previous_directory(folder.path(), now());
    assert_ne!(first, second);
    assert!(second.to_string_lossy().ends_with("previous-20261124-101530-2"));
}

#[test]
fn a_restore_without_room_or_without_a_figure_for_it_is_refused() {
    use crate::backups::space::fake;
    let source = Instance::new();
    let archives = TempDir::new();
    let archive = archives.path().join("backup.zip");
    source.archive(Parts::ALL, &archive);
    let target = Instance::new();
    let before = snapshot(target.root());

    for (told, words) in [
        (Some(10), "bytes on the"),
        (None, "could not be read"),
    ] {
        fake::tell(target.root(), told);
        for outcome in [
            apply(&archive, target.root(), now()).map(|_| ()),
            check(&archive, &target.root().join("restore")).map(|_| ()),
            stage(&archive, target.root()).map(|_| ()),
        ] {
            match outcome {
                Err(RestoreError::Refused(message)) => {
                    assert!(message.contains(words), "{told:?}: {message}");
                }
                other => panic!("{told:?}: not refused: {other:?}"),
            }
        }
        assert_eq!(snapshot(target.root()), before);
    }
    // With room, and a figure, the same archive goes through.
    fake::tell(target.root(), Some(u64::MAX / 2));
    apply(&archive, target.root(), now()).unwrap();
}

#[test]
fn a_database_that_is_not_an_instances_and_a_part_with_nothing_in_it_are_refused_on_purpose() {
    use crate::backups::fixtures::instance_database_bytes;
    // The first server restored any SQLite file; this one wants accounts and
    // servers in it, and something behind every part the manifest names.
    refused_and_unchanged(
        &[
            ("database/stellarr.db", &database_bytes(), None),
            ("keys/master.key", b"k", None),
            ("instance/secrets/turn", b"s", None),
        ],
        "has no Users or Servers table",
    );
    let real = instance_database_bytes();
    refused_and_unchanged(
        &[("database/stellarr.db", &real, None), ("instance/secrets/turn", b"s", None)],
        "names the keys part",
    );
    // With a key, it is accepted.
    let archives = TempDir::new();
    let archive = archives.path().join("whole.zip");
    archive_of(
        &[
            ("database/stellarr.db", &real, None),
            ("keys/master.key", b"k", None),
            ("instance/secrets/turn", b"s", None),
        ],
        &archive,
    );
    assert!(check(&archive, &archives.path().join("work")).is_ok());
}

#[test]
fn nobody_is_signed_in_after_a_restore() {
    let source = Instance::new();
    let user = crate::values::Id::random();
    let connection = rusqlite::Connection::open(source.database_path()).unwrap();
    connection.pragma_update(None, "foreign_keys", false).unwrap();
    connection
        .execute(
            "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
                 \"AbsoluteExpiresAt\", \"PrimarySource\", \"AuthenticatedAt\", \"SecurityVersion\", \"ShortLived\")
             VALUES (?1, ?2, 'D', '127.0.0.1', ?3, ?3, '2999-01-01 00:00:00', '2999-01-01 00:00:00', 'password', ?3, 1, 0)",
            rusqlite::params![crate::values::Id::random(), user, now()],
        )
        .unwrap();
    // A remembered browser, a sign-in half-way and a reset link, all in the archive.
    for sql in [
        "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\")
         VALUES (?1, ?2, 'h', 'n', 'd', 'ip', ?3, '2999-01-01 00:00:00')",
        "INSERT INTO \"AuthChallenges\" (\"Id\", \"UserId\", \"Purpose\", \"SecretHash\", \"PrimarySource\", \"SecurityVersion\", \"ShortLived\",
             \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\")
         VALUES (?1, ?2, 'mfa', 'h', 'password', 1, 0, 0, 5, ?3, '2999-01-01 00:00:00')",
        "INSERT INTO \"PasswordResetTokens\" (\"Id\", \"UserId\", \"TokenHash\", \"CreatedAt\", \"ExpiresAt\")
         VALUES (?1, ?2, 'h', ?3, '2999-01-01 00:00:00')",
    ] {
        connection
            .execute(sql, rusqlite::params![crate::values::Id::random(), user, now()])
            .unwrap();
    }
    drop(connection);
    let archives = TempDir::new();
    let archive = archives.path().join("backup.zip");
    source.archive(Parts::ALL, &archive);
    let target = Instance::new();

    apply(&archive, target.root(), now()).unwrap();
    // The database is one file again.
    assert!(!target.root().join("db/stellarr.db-wal").exists());
    let restored = rusqlite::Connection::open(target.database_path()).unwrap();
    let running: i64 = restored
        .query_row(
            "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let ended: i64 = restored
        .query_row("SELECT COUNT(*) FROM \"UserSessions\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!((running, ended), (0, 1));
    for (what, sql) in [
        ("a remembered browser", "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"),
        ("a sign-in half-way", "SELECT COUNT(*) FROM \"AuthChallenges\""),
        ("a reset link", "SELECT COUNT(*) FROM \"PasswordResetTokens\" WHERE \"UsedAt\" IS NULL"),
    ] {
        let left: i64 = restored.query_row(sql, [], |row| row.get(0)).unwrap();
        assert_eq!(left, 0, "{what}");
    }
}

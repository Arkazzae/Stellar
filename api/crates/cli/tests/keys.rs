//! The master key: when the tool makes one, when it leaves one alone, and what
//! it needs of one.

mod support;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rusqlite::params;
use stellarr_api::seal::Sealer;
use stellarr_api::values::Timestamp;
use stellarr_cli::terminal::PromptFailure;
use support::Instance;

const SET: [&str; 4] = ["--json", "secrets", "set", "mail.smtp-password"];

fn key(instance: &Instance) -> std::path::PathBuf {
    instance.keys().join("master.key")
}

fn without_a_key() -> Instance {
    let instance = Instance::started();
    std::fs::remove_file(key(&instance)).unwrap();
    instance
}

#[test]
fn a_cancelled_or_refused_set_makes_no_key() {
    let instance = without_a_key();

    // Not confirmed.
    let declined = instance.run_interactive("n\n", &["secrets", "set", "mail.smtp-password"]);
    assert_eq!(declined.exit, 2);
    assert!(
        declined.output.contains("this makes") && declined.output.contains("master.key"),
        "the question does not say a key will be made: {}",
        declined.output
    );
    assert!(!key(&instance).exists());

    // Confirmed, then Ctrl+C at the prompt.
    let mut args = SET.to_vec();
    args.push("--yes");
    let interrupted = instance.run_prompted(
        &args,
        Box::new(|| Err(PromptFailure::Interrupted)),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(interrupted.exit, 2);
    assert!(!key(&instance).exists());

    // Confirmed, and the value is refused.
    let mut stdin_args = SET.to_vec();
    stdin_args.extend(["--stdin", "--yes"]);
    let blank = instance.run_with_stdin("   \n", &stdin_args);
    assert_eq!(blank.code(), "invalid_secret");
    assert!(!key(&instance).exists());
    assert_eq!(
        std::fs::read_dir(instance.keys()).unwrap().count(),
        0,
        "something was left in the keys folder"
    );
}

#[test]
fn a_set_that_makes_the_key_says_so_with_its_path_and_owner() {
    let instance = without_a_key();
    let mut args = SET.to_vec();
    args.extend(["--stdin", "--yes"]);

    let run = instance.run_with_stdin("s3cret\n", &args);

    assert_eq!(run.exit, 0, "{}", run.error);
    let meta = std::fs::metadata(key(&instance)).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let warning = run.json()["warnings"][0].as_str().unwrap().to_string();
    assert!(
        warning.contains(&key(&instance).display().to_string()),
        "{warning}"
    );
    assert!(
        warning.contains(&format!("owner uid {}", meta.uid())),
        "{warning}"
    );
    assert!(!run.output.contains("s3cret"));

    // The next set finds a key and says nothing about one.
    let again = instance.run_with_stdin("s3cret2\n", &args);
    assert!(again.json()["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn an_existing_key_is_read_as_it_is_and_its_loose_mode_is_said() {
    let instance = Instance::started();
    std::fs::set_permissions(key(&instance), std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut args = SET.to_vec();
    args.extend(["--stdin", "--yes"]);

    let run = instance.run_with_stdin("s3cret\n", &args);

    assert_eq!(run.exit, 0, "{}", run.error);
    let mode = std::fs::metadata(key(&instance))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644, "the tool changed a mode it did not say");
    let warning = run.json()["warnings"][0].as_str().unwrap().to_string();
    assert!(warning.contains("0644"), "{warning}");
}

#[test]
fn clearing_a_secret_needs_no_readable_key() {
    let instance = Instance::started();
    let sealer = Sealer::open_existing(&instance.keys()).unwrap().unwrap();
    instance
        .db()
        .execute(
            "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('mail.smtp-password', ?1, ?2)",
            params![
                sealer.seal("instance-secret", b"mail.smtp-password", None, b"old"),
                Timestamp::now()
            ],
        )
        .unwrap();
    std::fs::write(key(&instance), b"damaged").unwrap();

    let run = instance.run(&["--json", "secrets", "clear", "mail.smtp-password", "--yes"]);

    assert_eq!(run.exit, 0, "{}", run.output);
    let connection = instance.db();
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"InstanceSecrets\"", [], |r| r.get(0))
        .unwrap();
    let audits: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"OperatorAuditEntries\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!((rows, audits), (0, 1));
    // The damaged key is left as it was.
    assert_eq!(std::fs::read(key(&instance)).unwrap(), b"damaged");
}

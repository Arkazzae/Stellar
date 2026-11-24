use rusqlite::params;

use super::*;
use crate::testing::new_database;

fn sealer() -> Sealer {
    Sealer::from_key([9u8; 32])
}

fn environment(values: &[(&'static str, &str)]) -> EnvironmentSecrets {
    EnvironmentSecrets::with(values)
}

async fn resolved(
    database: &crate::database::Database,
    environment: EnvironmentSecrets,
    sealer: Sealer,
) -> ResolvedSecrets {
    database
        .read(move |db| read(db, &environment, &sealer))
        .await
        .unwrap()
}

async fn store(database: &crate::database::Database, definition: SecretDefinition, value: &str) {
    let value = value.to_string();
    database
        .write(move |tx| set(tx, &sealer(), &definition, &value))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_secret_set_in_the_panel_is_stored_sealed_and_read_back_whole() {
    let (_folder, database) = new_database();
    store(&database, SMTP_PASSWORD, " pass with spaces ").await;

    let secrets = resolved(&database, environment(&[]), sealer()).await;

    let password = secrets.smtp_password();
    assert_eq!(password.expose(), " pass with spaces ");
    assert_eq!(password.source, Source::Panel);
    let (name, stored): (String, String) = database
        .read(|db| {
            db.query_row(
                "SELECT \"Name\", \"ProtectedValue\" FROM \"InstanceSecrets\"",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(name, "mail.smtp-password");
    assert!(!stored.contains("pass"), "{stored}");
    assert!(!stored.contains("spaces"), "{stored}");
}

#[tokio::test]
async fn keys_are_trimmed_when_stored_and_passwords_are_not() {
    let (_folder, database) = new_database();
    store(&database, RESEND_API_KEY, " re_panel ").await;
    store(&database, SMTP_PASSWORD, " p ").await;

    let secrets = resolved(&database, environment(&[]), sealer()).await;

    assert_eq!(secrets.resend_api_key().expose(), "re_panel");
    assert_eq!(secrets.smtp_password().expose(), " p ");
}

#[tokio::test]
async fn the_panels_value_wins_and_clearing_it_falls_back_to_the_environment() {
    let (_folder, database) = new_database();
    let env = || {
        environment(&[
            ("Giphy:ApiKey", " env-giphy "),
            ("Mail:SmtpPassword", " env pass "),
        ])
    };

    let before = resolved(&database, env(), sealer()).await;
    assert_eq!(before.get(&GIPHY_API_KEY).source, Source::Environment);
    assert_eq!(before.get(&GIPHY_API_KEY).expose(), "env-giphy");
    // The environment's password keeps its spaces.
    assert_eq!(before.smtp_password().expose(), " env pass ");

    store(&database, GIPHY_API_KEY, "panel-giphy").await;
    let during = resolved(&database, env(), sealer()).await;
    assert_eq!(during.get(&GIPHY_API_KEY).source, Source::Panel);
    assert_eq!(during.get(&GIPHY_API_KEY).expose(), "panel-giphy");

    database
        .write(|tx| clear(tx, &GIPHY_API_KEY))
        .await
        .unwrap();
    let after = resolved(&database, env(), sealer()).await;
    assert_eq!(after.get(&GIPHY_API_KEY).source, Source::Environment);
    assert_eq!(after.get(&GIPHY_API_KEY).expose(), "env-giphy");
    assert!(
        !resolved(&database, environment(&[]), sealer())
            .await
            .get(&GIPHY_API_KEY)
            .is_set()
    );
}

#[tokio::test]
async fn setting_again_replaces_the_value_and_moves_the_time() {
    let (_folder, database) = new_database();
    store(&database, GIPHY_API_KEY, "first").await;
    let first = resolved(&database, environment(&[]), sealer()).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    store(&database, GIPHY_API_KEY, "second").await;
    let second = resolved(&database, environment(&[]), sealer()).await;

    assert_eq!(second.get(&GIPHY_API_KEY).expose(), "second");
    let rows: i64 = database
        .read(|db| db.query_row("SELECT COUNT(*) FROM \"InstanceSecrets\"", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(rows, 1);
    let (a, b) = (
        first.status(&GIPHY_API_KEY).updated_at.unwrap(),
        second.status(&GIPHY_API_KEY).updated_at.unwrap(),
    );
    assert!(b > a);
}

/// Reading the secrets with these keys fails.
async fn fails(
    database: &crate::database::Database,
    environment: EnvironmentSecrets,
    sealer: Sealer,
) -> bool {
    database
        .read(move |db| read(db, &environment, &sealer))
        .await
        .is_err()
}

#[tokio::test]
async fn a_row_the_keys_cannot_open_is_a_damaged_value() {
    let (_folder, database) = new_database();
    store(&database, RESEND_API_KEY, "re_sealed_elsewhere").await;

    // The same row under keys it was not sealed with, and the environment's
    // value does not stand in for it.
    assert!(fails(&database, environment(&[]), Sealer::from_key([1u8; 32])).await);
    assert!(
        fails(
            &database,
            environment(&[("Mail:ResendApiKey", "re_env")]),
            Sealer::from_key([1u8; 32])
        )
        .await
    );
    assert!(!fails(&database, environment(&[]), sealer()).await);
}

#[tokio::test]
async fn a_row_moved_under_another_name_does_not_open() {
    let (_folder, database) = new_database();
    store(&database, GIPHY_API_KEY, "giphy-secret").await;
    database
        .write(|tx| {
            tx.execute(
                "UPDATE \"InstanceSecrets\" SET \"Name\" = ?1",
                [RESEND_API_KEY.name],
            )
        })
        .await
        .unwrap();

    assert!(fails(&database, environment(&[]), sealer()).await);
}

#[tokio::test]
async fn a_row_with_nothing_in_it_counts_as_unset_and_one_that_is_not_text_is_damaged() {
    let (_folder, database) = new_database();
    let empty = sealer().seal(PURPOSE, GIPHY_API_KEY.name.as_bytes(), None, b"");
    let not_text = sealer().seal(PURPOSE, RESEND_API_KEY.name.as_bytes(), None, &[0xff, 0xfe]);
    let insert = |name: &'static str, sealed: String| {
        database.write(move |tx| {
            tx.execute(
                "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, ?2, ?3)",
                params![name, sealed, Timestamp::now()],
            )
        })
    };
    insert(GIPHY_API_KEY.name, empty).await.unwrap();

    let secrets = resolved(&database, environment(&[("Giphy:ApiKey", "env")]), sealer()).await;

    assert_eq!(secrets.get(&GIPHY_API_KEY).source, Source::Environment);
    insert(RESEND_API_KEY.name, not_text).await.unwrap();
    assert!(fails(&database, environment(&[]), sealer()).await);
}

#[test]
fn a_value_is_valid_when_it_is_not_blank_and_within_the_limit() {
    assert!(!is_valid_value(""));
    assert!(!is_valid_value("   "));
    assert!(!is_valid_value("\t\n"));
    assert!(is_valid_value("x"));
    assert!(is_valid_value(&"k".repeat(MAX_LENGTH)));
    assert!(!is_valid_value(&"k".repeat(MAX_LENGTH + 1)));
    // The limit is counted in UTF-16 units: a letter outside the plane counts twice.
    assert!(is_valid_value(&"😀".repeat(MAX_LENGTH / 2)));
    assert!(!is_valid_value(&"😀".repeat(MAX_LENGTH / 2 + 1)));
}

#[test]
fn the_names_the_panel_knows_are_these_six_with_their_variables() {
    let listed: Vec<(&str, &str)> = ALL
        .iter()
        .map(|definition| (definition.name, definition.variable()))
        .collect();

    assert_eq!(
        listed,
        [
            ("mail.resend-api-key", "STELLARR_RESEND_API_KEY"),
            ("mail.smtp-password", "STELLARR_SMTP_PASSWORD"),
            ("giphy.api-key", "STELLARR_GIPHY_API_KEY"),
            (
                "cloudflare.r2.access-key",
                "STELLARR_CLOUDFLARE_R2_ACCESS_KEY"
            ),
            (
                "cloudflare.r2.secret-key",
                "STELLARR_CLOUDFLARE_R2_SECRET_KEY"
            ),
            (
                "cloudflare.r2.worker-token",
                "STELLARR_CLOUDFLARE_R2_WORKER_TOKEN"
            ),
        ]
    );
    assert_eq!(find("giphy.api-key"), Some(&GIPHY_API_KEY));
    assert_eq!(find("storage.secret-key"), None);
    for definition in &ALL {
        assert!(EnvironmentSecrets::KEYS.contains(&definition.setting));
    }
}

#[tokio::test]
async fn nothing_that_describes_a_secret_holds_its_value() {
    let (_folder, database) = new_database();
    store(&database, SMTP_PASSWORD, "hunter2-the-secret").await;
    let secrets = resolved(
        &database,
        environment(&[("Giphy:ApiKey", "env-secret-key")]),
        sealer(),
    )
    .await;

    let shown = format!(
        "{secrets:?} {:?} {:?}",
        secrets.statuses(),
        secrets.smtp_password()
    );

    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("env-secret"), "{shown}");
    let status = secrets.status(&SMTP_PASSWORD);
    assert_eq!(
        (status.configured, status.source, status.variable),
        (true, "panel", "STELLARR_SMTP_PASSWORD")
    );
    assert!(status.updated_at.is_some());
    let from_environment = secrets.status(&GIPHY_API_KEY);
    assert_eq!(
        (from_environment.configured, from_environment.source),
        (true, "environment")
    );
    assert_eq!(from_environment.updated_at, None);
}

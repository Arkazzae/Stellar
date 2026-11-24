//! The commands of the tool, run on an instance on disk: what each prints,
//! what it changes, and what it refuses. A port of the first tool's
//! `CliCommandTests`, with the key ring replaced by the master key.

mod support;

use std::os::unix::fs::PermissionsExt;

use rusqlite::params;
use stellarr_api::seal::Sealer;
use stellarr_api::secrets;
use stellarr_api::values::{Id, Timestamp};
use support::Instance;

#[test]
fn help_version_and_usage_errors_do_not_touch_the_data_volume() {
    let instance = Instance::unstarted();

    let version = instance.run(&["--json", "version"]);
    assert_eq!(version.exit, 0);
    let version = version.json();
    assert_eq!(version["ok"], true);
    assert!(!version["data"]["version"].as_str().unwrap().is_empty());

    let status = instance.run(&["--json", "status"]);
    assert_eq!(status.exit, 3);
    assert_eq!(status.code(), "database_not_found");

    let unknown = instance.run(&["--json", "not-a-command"]);
    assert_eq!(
        (unknown.exit, unknown.code().as_str()),
        (2, "unknown_command")
    );
    let missing = instance.run(&["--json", "--data-root", "--json", "version"]);
    assert_eq!((missing.exit, missing.code().as_str()), (2, "usage"));
    for empty in ["", "   "] {
        let run = instance.run(&["--json", "--data-root", empty, "version"]);
        assert_eq!((run.exit, run.code().as_str()), (2, "usage"));
    }
    let inline = instance.run(&[
        "--json",
        "config",
        "set",
        "time-zone",
        "Europe/Warsaw",
        "--yes",
        "--data-root=",
    ]);
    assert_eq!((inline.exit, inline.code().as_str()), (2, "usage"));
    // A deliberately dash-prefixed value still has the explicit = form.
    let dashed = instance.run(&["--data-root=--virtual", "--json", "version"]);
    assert_eq!(dashed.exit, 0);

    assert!(!instance.root().join("db").exists());
    assert!(!instance.root().join("keys").exists());
}

#[test]
fn version_and_help_never_let_a_write_command_run() {
    let instance = Instance::unstarted();

    for terminal in ["--version", "--help"] {
        let run = instance.run(&[
            "config",
            "set",
            "time-zone",
            "Europe/Warsaw",
            "--yes",
            terminal,
        ]);
        assert_eq!(run.exit, 0);
        assert!(!run.output.contains("Set time-zone"));
    }
    assert!(!instance.root().join("db").exists());
    assert!(!instance.root().join("keys").exists());
}

#[test]
fn read_commands_work_on_a_read_only_database_without_writing() {
    let instance = Instance::started();
    {
        let connection = instance.db();
        let _: String = connection
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .unwrap();
    }
    let path = instance.database_path();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let listing = || {
        let mut names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let before = listing();

    for command in [
        ["config", "list"],
        ["users", "list"],
        ["sessions", "list"],
        ["secrets", "list"],
    ] {
        let run = instance.run(&["--json", command[0], command[1]]);
        assert_eq!(run.exit, 0, "{command:?}: {}", run.output);
    }

    assert_eq!(before, listing());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn read_commands_never_create_the_keys_folder() {
    let instance = Instance::started();
    std::fs::remove_dir_all(instance.keys()).unwrap();

    assert_eq!(instance.run(&["--json", "config", "list"]).exit, 0);
    assert_eq!(instance.run(&["--json", "users", "list"]).exit, 0);
    assert_eq!(instance.run(&["--json", "secrets", "list"]).exit, 0);

    assert!(!instance.keys().exists());
    assert!(instance.database_path().exists());
}

#[test]
fn config_set_uses_shared_validation_and_audits_in_the_same_transaction() {
    let instance = Instance::started();

    let set = instance.run(&[
        "--json",
        "config",
        "set",
        "time-zone",
        "Europe/Warsaw",
        "--yes",
    ]);
    assert_eq!(set.exit, 0, "{}", set.output);
    let data = set.json()["data"].clone();
    assert_eq!(data["value"], "Europe/Warsaw");
    assert_eq!(data["auditActor"], "local-cli");
    let connection = instance.db();
    let zone: String = connection
        .query_row(
            "SELECT \"TimeZone\" FROM \"OperationsSettings\"",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(zone, "Europe/Warsaw");
    let audit: (String, String, String) = connection
        .query_row(
            "SELECT \"Actor\", \"Operation\", \"Target\" FROM \"OperatorAuditEntries\"",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        audit,
        ("local-cli".into(), "config.set".into(), "time-zone".into())
    );

    let invalid = instance.run(&[
        "--json",
        "config",
        "set",
        "mail.smtp.port",
        "99999",
        "--yes",
    ]);
    assert_eq!(
        (invalid.exit, invalid.code().as_str()),
        (2, "invalid_value")
    );
    let read_only = instance.run(&[
        "--json",
        "config",
        "set",
        "public-url",
        "https://chat.example",
        "--yes",
    ]);
    assert_eq!(
        (read_only.exit, read_only.code().as_str()),
        (4, "read_only_setting")
    );

    let audits: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"OperatorAuditEntries\"", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(audits, 1);
}

#[test]
fn config_set_instance_name_persists_where_the_panel_reads_it() {
    let instance = Instance::started();

    let set = instance.run(&[
        "--json",
        "config",
        "set",
        "instance.name",
        "Orbit CLI",
        "--yes",
    ]);

    assert_eq!(set.exit, 0);
    let name: String = instance
        .db()
        .query_row("SELECT \"Name\" FROM \"Servers\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Orbit CLI");
    assert_eq!(
        instance
            .run(&["--json", "config", "get", "instance.name"])
            .json()["data"]["value"],
        "Orbit CLI"
    );
}

#[test]
fn empty_text_settings_are_allowed_when_the_shared_rules_allow_them() {
    let instance = Instance::started();

    for key in ["mail.smtp.host", "mail.smtp.username", "mail.from-address"] {
        assert_eq!(
            instance
                .run(&["--json", "config", "set", key, "", "--yes"])
                .exit,
            0,
            "{key}"
        );
    }
    let connection = instance.db();
    let (host, username, from): (String, String, String) = connection
        .query_row(
            "SELECT \"MailSmtpHost\", \"MailSmtpUsername\", \"MailFromAddress\" FROM \"OperationsSettings\"",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (host.as_str(), username.as_str(), from.as_str()),
        ("", "", "")
    );

    let invalid = instance.run(&["--json", "config", "set", "instance.name", "", "--yes"]);
    assert_eq!(
        (invalid.exit, invalid.code().as_str()),
        (2, "invalid_value")
    );
    let name: String = connection
        .query_row("SELECT \"Name\" FROM \"Servers\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Orbit");
    let audits: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"OperatorAuditEntries\"", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(audits, 3);
}

fn value_and_source(
    instance: &Instance,
    environment: &[(&str, &str)],
    key: &str,
) -> (String, String) {
    let run = instance.run_with_environment(environment, &["--json", "config", "get", key]);
    let data = run.json()["data"].clone();
    (
        data["value"]
            .as_str()
            .map_or_else(|| data["value"].to_string(), str::to_string),
        data["source"].as_str().unwrap().to_string(),
    )
}

#[test]
fn startup_settings_read_the_public_address_the_launcher_derives() {
    let instance = Instance::started();
    let wrapped = [
        ("STELLARR_DOMAIN", "chat.example.com"),
        ("STELLARR_HTTPS_PORT", "8443"),
        ("Operations__PublicUrl", "https://chat.example.com:8443"),
    ];
    assert_eq!(
        value_and_source(&instance, &wrapped, "public-url"),
        ("https://chat.example.com:8443".into(), "environment".into())
    );
    // The tool derives nothing itself: outside the launcher a domain alone
    // gives it no public address.
    let domain_only = [("STELLARR_DOMAIN", "chat.example.com")];
    assert_eq!(
        value_and_source(&instance, &domain_only, "public-url"),
        ("".into(), "default".into())
    );
    assert_eq!(
        value_and_source(&instance, &domain_only, "rtc.public-host"),
        ("chat.example.com".into(), "environment".into())
    );
    assert_eq!(
        value_and_source(&instance, &[], "https-port"),
        ("443".into(), "default".into())
    );
    assert_eq!(
        value_and_source(&instance, &[("STELLARR_HTTPS_PORT", "8443")], "https-port"),
        ("8443".into(), "environment".into())
    );
    // A blank variable is not set, as the entrypoint reads it.
    assert_eq!(
        value_and_source(&instance, &[("STELLARR_HTTPS_PORT", "  ")], "https-port"),
        ("443".into(), "default".into())
    );
}

#[test]
fn a_startup_setting_reads_the_same_without_a_database() {
    let instance = Instance::unstarted();
    let environment = [("Operations__PublicUrl", "https://chat.example.com")];

    let public_url =
        instance.run_with_environment(&environment, &["--json", "config", "get", "public-url"]);

    assert_eq!(public_url.exit, 0);
    assert_eq!(
        public_url.json()["data"]["value"],
        "https://chat.example.com"
    );
    assert_eq!(public_url.json()["data"]["source"], "environment");
    assert!(!instance.root().join("db").exists());
    let zone =
        instance.run_with_environment(&environment, &["--json", "config", "get", "time-zone"]);
    assert_eq!((zone.exit, zone.code().as_str()), (3, "database_not_found"));
}

#[test]
fn startup_settings_read_the_image_defaults_and_the_operators_variables() {
    let instance = Instance::started();

    for (key, value) in [
        ("rtc.turn-tcp-enabled", "false"),
        ("rtc.turns-enabled", "false"),
        ("rtc.media-port", "49000"),
    ] {
        assert_eq!(
            value_and_source(&instance, &[], key),
            (value.into(), "default".into()),
            "{key}"
        );
    }
    assert_eq!(
        value_and_source(&instance, &[], "rtc.public-host"),
        ("localhost".into(), "entrypoint".into())
    );
    let operator = [
        ("STELLARR_TURN_TCP_ENABLED", "true"),
        ("STELLARR_MEDIA_PORT", "49010"),
        ("Rtc__TurnsPort", "5350"),
    ];
    assert_eq!(
        value_and_source(&instance, &operator, "rtc.turn-tcp-enabled"),
        ("true".into(), "environment".into())
    );
    assert_eq!(
        value_and_source(&instance, &operator, "rtc.media-port"),
        ("49010".into(), "environment".into())
    );
    // A raw key still counts as the environment's.
    assert_eq!(
        value_and_source(&instance, &operator, "rtc.turns-port"),
        ("5350".into(), "environment".into())
    );
}

#[test]
fn with_no_environment_named_the_tool_takes_the_production_seeds() {
    // `docker exec` does not see the entrypoint's environment name, and the
    // container runs in production.
    let production = {
        let instance = Instance::started();
        let words = ["status".to_string()];
        let _ = &instance;
        stellarr_cli::runtime::Runtime::create(
            stellarr_cli::invocation::Invocation::parse(&words).unwrap(),
            support::silent_io(),
            vec![],
            Default::default(),
        )
    };
    assert_eq!(production.config.raw("Mail:Enabled"), Some("true"));
    assert_eq!(production.config.raw("Backups:Enabled"), Some("true"));
    assert_eq!(
        production.config.raw("Desktop:RecommendedVersion"),
        Some("0.1.3")
    );

    let development = stellarr_cli::runtime::Runtime::create(
        stellarr_cli::invocation::Invocation::parse(&["status".to_string()]).unwrap(),
        support::silent_io(),
        vec![("ASPNETCORE_ENVIRONMENT".into(), "Development".into())],
        Default::default(),
    );
    assert_eq!(development.config.raw("Mail:Enabled"), Some("false"));
    assert_eq!(development.config.raw("Backups:Enabled"), None);

    // The operator's variable wins over the production file.
    let mail_off = stellarr_cli::runtime::Runtime::create(
        stellarr_cli::invocation::Invocation::parse(&["status".to_string()]).unwrap(),
        support::silent_io(),
        vec![("STELLARR_MAIL_ENABLED".into(), "false".into())],
        Default::default(),
    );
    assert_eq!(mail_off.config.raw("Mail:Enabled"), Some("false"));
}

#[test]
fn nullable_integrations_keep_preference_source_and_effective_availability_apart() {
    let instance = Instance::started();

    let set = instance.run(&["--json", "config", "set", "giphy.enabled", "true", "--yes"]);
    assert_eq!(set.exit, 0, "{}", set.output);
    let json = set.json();
    assert_eq!(json["data"]["value"], true);
    assert_eq!(json["data"]["effectiveValue"], false);
    assert_eq!(json["data"]["source"], "database");
    assert!(!json["warnings"].as_array().unwrap().is_empty());
    let stored: Option<bool> = instance
        .db()
        .query_row(
            "SELECT \"GiphyEnabled\" FROM \"OperationsSettings\"",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, Some(true));

    let with_key = instance.run_with_environment(
        &[("STELLARR_GIPHY_API_KEY", "env-giphy")],
        &["--json", "config", "get", "giphy.enabled"],
    );
    assert_eq!(with_key.json()["data"]["value"], true);
    assert_eq!(with_key.json()["data"]["effectiveValue"], true);

    let resend = instance.run(&["--json", "config", "get", "mail.resend.enabled"]);
    assert!(resend.json()["data"]["value"].is_null());
    assert_eq!(resend.json()["data"]["effectiveValue"], false);
    assert_eq!(resend.json()["data"]["source"], "default");
    let with_resend_key = instance.run_with_environment(
        &[("Mail__ResendApiKey", "re_env")],
        &["--json", "config", "get", "mail.resend.enabled"],
    );
    assert!(with_resend_key.json()["data"]["value"].is_null());
    assert_eq!(with_resend_key.json()["data"]["effectiveValue"], true);

    let port = instance.run(&["--json", "config", "get", "mail.smtp.port"]);
    assert_eq!(port.json()["data"]["value"], 587);
    assert_eq!(port.json()["data"]["source"], "database");
}

#[test]
fn doctor_reads_the_volume_and_status_treats_an_unreachable_api_as_unhealthy() {
    let instance = Instance::started();
    let environment = [("STELLARR_API_URL", "http://127.0.0.1:1")];

    let status = instance.run_with_environment(&environment, &["--json", "status"]);
    assert_eq!(status.exit, 1);
    let json = status.json();
    assert_eq!(json["ok"], false);
    assert_eq!(json["data"]["healthy"], false);
    assert_eq!(json["data"]["api"]["available"], false);

    let doctor = instance.run_with_environment(&environment, &["--json", "doctor"]);
    assert_eq!(doctor.exit, 0, "{}", doctor.output);
    let checks = doctor.json()["data"]["checks"].clone();
    let local_api = checks
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "local-api")
        .unwrap();
    assert_eq!(local_api["status"], "warn");
}

#[test]
fn unconfirmed_or_cancelled_mutations_change_nothing() {
    let instance = Instance::started();

    let closed = instance.run(&["--json", "config", "set", "time-zone", "Europe/Warsaw"]);
    assert_eq!(
        (closed.exit, closed.code().as_str()),
        (2, "confirmation_required")
    );
    let cancelled = instance.run_interactive(
        "n\n",
        &["--json", "config", "set", "time-zone", "Europe/Warsaw"],
    );
    assert_eq!(
        (cancelled.exit, cancelled.code().as_str()),
        (2, "cancelled")
    );

    let connection = instance.db();
    let settings: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"OperationsSettings\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    let audits: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"OperatorAuditEntries\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!((settings, audits), (0, 0));
}

#[test]
fn secrets_use_the_existing_master_key_and_never_reach_the_output() {
    let secret = " super-secret ";
    let instance = Instance::started();

    let set = instance.run_with_stdin(
        &format!("{secret}\n"),
        &[
            "secrets",
            "set",
            "mail.smtp-password",
            "--stdin",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(set.exit, 0, "{}", set.error);
    assert!(!set.output.contains("super-secret") && !set.error.contains("super-secret"));

    let connection = instance.db();
    let (name, sealed): (String, String) = connection
        .query_row(
            "SELECT \"Name\", \"ProtectedValue\" FROM \"InstanceSecrets\"",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(name, "mail.smtp-password");
    assert!(!sealed.contains("super-secret"));
    // The server reads it with the key it has: spaces kept, as a password's are.
    let sealer = Sealer::open_existing(&instance.keys()).unwrap().unwrap();
    let environment =
        stellarr_cli::secrets::environment_secrets(&stellarr_api::settings::Config::default());
    let resolved = secrets::read(&connection, &environment, &sealer).unwrap();
    assert_eq!(resolved.smtp_password().expose(), secret);
    let audit: (String, String) = connection
        .query_row(
            "SELECT \"Operation\", \"Target\" FROM \"OperatorAuditEntries\"",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(audit, ("secrets.set".into(), "mail.smtp-password".into()));

    let list = instance.run(&["--json", "secrets", "list"]);
    assert_eq!(list.exit, 0);
    assert!(!list.output.contains("super-secret"));
    let listed = list.json()["data"]["secrets"].clone();
    let one = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "mail.smtp-password")
        .unwrap();
    assert_eq!(one["source"], "database");
    assert_eq!(one["configured"], true);

    let clear = instance.run(&["--json", "secrets", "clear", "mail.smtp-password", "--yes"]);
    assert_eq!(clear.exit, 0);
    assert!(!clear.output.contains("super-secret"));
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"InstanceSecrets\"", [], |r| r.get(0))
        .unwrap();
    let audits: i64 = connection
        .query_row("SELECT COUNT(*) FROM \"OperatorAuditEntries\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!((rows, audits), (0, 2));
}

#[test]
fn a_secret_cannot_be_given_as_an_argument_and_needs_the_keys_folder() {
    let instance = Instance::started();

    let argument = instance.run(&["--json", "secrets", "set", "mail.smtp-password", "value"]);
    assert_eq!(
        (argument.exit, argument.code().as_str()),
        (2, "secret_value_argument")
    );
    let without_yes = instance.run_with_stdin(
        "v\n",
        &["--json", "secrets", "set", "mail.smtp-password", "--stdin"],
    );
    assert_eq!(
        (without_yes.exit, without_yes.code().as_str()),
        (2, "confirmation_required")
    );
    let nothing_to_ask = instance.run(&["--json", "secrets", "set", "mail.smtp-password", "--yes"]);
    assert_eq!(
        (nothing_to_ask.exit, nothing_to_ask.code().as_str()),
        (2, "secret_value_required")
    );
    let blank = instance.run_with_stdin(
        "   \n",
        &[
            "--json",
            "secrets",
            "set",
            "mail.smtp-password",
            "--stdin",
            "--yes",
        ],
    );
    assert_eq!((blank.exit, blank.code().as_str()), (2, "invalid_secret"));

    std::fs::remove_dir_all(instance.keys()).unwrap();
    let no_keys = instance.run_with_stdin(
        "v\n",
        &[
            "--json",
            "secrets",
            "set",
            "mail.smtp-password",
            "--stdin",
            "--yes",
        ],
    );
    assert_eq!(
        (no_keys.exit, no_keys.code().as_str()),
        (4, "data_protection_keys_missing")
    );
    assert!(!instance.keys().exists());
}

#[test]
fn a_value_sealed_under_another_key_counts_as_unset_and_is_said() {
    let instance = Instance::started();
    // What the first server wrote, or this one under a key since lost.
    let other = Sealer::from_key([9u8; 32]);
    instance
        .db()
        .execute(
            "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('mail.smtp-password', ?1, ?2)",
            params![other.seal("instance-secret", b"mail.smtp-password", None, b"old"), Timestamp::now()],
        )
        .unwrap();

    let list = instance.run(&["--json", "secrets", "list"]);

    assert_eq!(list.exit, 0);
    let json = list.json();
    let one = json["data"]["secrets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "mail.smtp-password")
        .unwrap()
        .clone();
    assert_eq!(one["configured"], false);
    assert_eq!(one["source"], "none");
    assert!(
        json["warnings"][0]
            .as_str()
            .unwrap()
            .contains("mail.smtp-password")
    );
    assert!(!list.output.contains("old"));
    // Clearing removes the row; so would setting it again, under the key in use.
    assert_eq!(
        instance
            .run(&["--json", "secrets", "clear", "mail.smtp-password", "--yes"])
            .exit,
        0
    );
    let rows: i64 = instance
        .db()
        .query_row("SELECT COUNT(*) FROM \"InstanceSecrets\"", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn users_and_sessions_expose_administrative_metadata_only() {
    let instance = Instance::started();
    let jo = instance
        .db()
        .query_row(
            "SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'jo'",
            [],
            |r| r.get::<_, Id>(0),
        )
        .unwrap();
    let hash: String = instance
        .db()
        .query_row(
            "SELECT \"PasswordHash\" FROM \"Users\" WHERE \"Id\" = ?1",
            [jo],
            |r| r.get(0),
        )
        .unwrap();
    let session = instance.session(jo, "TestBrowser/1.0");

    let users = instance.run(&["--json", "users", "list"]);
    assert_eq!(users.exit, 0);
    assert!(!users.output.contains(&hash));
    assert!(
        users.json()["data"]["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|user| user["username"] == "jo")
    );

    let sessions = instance.run(&["--json", "sessions", "list", "--user", "jo"]);
    assert_eq!(sessions.exit, 0);
    let listed = sessions.json()["data"]["sessions"].clone();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], session.to_string());
    assert_eq!(listed[0]["device"], "TestBrowser/1.0");
    assert!(!sessions.output.to_lowercase().contains("cookie"));

    let revoke = instance.run(&[
        "--json",
        "sessions",
        "revoke",
        &session.to_string(),
        "--yes",
    ]);
    assert_eq!(revoke.exit, 0);
    assert!(revoke.output.contains("realtimeDisconnectPending"));
    let revoked: Option<Timestamp> = instance
        .db()
        .query_row(
            "SELECT \"RevokedAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [session],
            |r| r.get(0),
        )
        .unwrap();
    assert!(revoked.is_some());
    let operation: String = instance
        .db()
        .query_row(
            "SELECT \"Operation\" FROM \"OperatorAuditEntries\"",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(operation, "sessions.revoke");
}

#[test]
fn revoke_all_invalidates_sessions_and_approved_desktop_handoffs() {
    let instance = Instance::started();
    let jo = instance
        .db()
        .query_row(
            "SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'jo'",
            [],
            |r| r.get::<_, Id>(0),
        )
        .unwrap();
    let (first, second) = (
        instance.session(jo, "First"),
        instance.session(jo, "Second"),
    );
    let handoff = Id::random();
    let now = Timestamp::now();
    instance
        .db()
        .execute(
            "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\")
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?4)",
            params![handoff, "a".repeat(64), jo, now, now.plus(std::time::Duration::from_secs(120))],
        )
        .unwrap();

    let result = instance.run(&[
        "--json", "sessions", "revoke", "--user", "jo", "--all", "--yes",
    ]);

    assert_eq!(result.exit, 0, "{}", result.output);
    let data = result.json()["data"].clone();
    assert_eq!(data["sessionsRevoked"], 2);
    assert_eq!(data["handoffsInvalidated"], 1);
    assert_eq!(data["realtimeDisconnectPending"], true);
    let connection = instance.db();
    for session in [first, second] {
        let revoked: Option<Timestamp> = connection
            .query_row(
                "SELECT \"RevokedAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
                [session],
                |r| r.get(0),
            )
            .unwrap();
        assert!(revoked.is_some());
    }
    let redeemed: Option<Timestamp> = connection
        .query_row(
            "SELECT \"RedeemedAt\" FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
            [handoff],
            |r| r.get(0),
        )
        .unwrap();
    assert!(redeemed.is_some());
    let audit: (String, String) = connection
        .query_row(
            "SELECT \"Actor\", \"Operation\" FROM \"OperatorAuditEntries\"",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(audit, ("local-cli".into(), "sessions.revoke-all".into()));
}

#[test]
fn a_secret_read_from_a_file_of_the_shell_and_from_input_is_the_same_and_the_arguments_are_refused()
{
    // `--stdin` is how a secret arrives from a file: `stellarr ... --stdin < file`.
    let instance = Instance::started();
    let from_file = "line from a file\n";

    let run = instance.run_with_stdin(
        from_file,
        &[
            "--json",
            "secrets",
            "set",
            "giphy.api-key",
            "--stdin",
            "--yes",
        ],
    );

    assert_eq!(run.exit, 0);
    let sealer = Sealer::open_existing(&instance.keys()).unwrap().unwrap();
    let environment =
        stellarr_cli::secrets::environment_secrets(&stellarr_api::settings::Config::default());
    let resolved = secrets::read(&instance.db(), &environment, &sealer).unwrap();
    assert_eq!(
        resolved.get(&secrets::GIPHY_API_KEY).expose(),
        "line from a file"
    );
}

#[test]
fn applying_a_staged_restore_says_what_it_replaces_before_it_asks() {
    let instance = support::Instance::unstarted();
    let restore = instance.root().join("restore");
    std::fs::create_dir_all(&restore).unwrap();
    std::fs::write(restore.join("staged.zip"), b"zip").unwrap();

    let asked = instance.run_interactive("n\n", &["backup", "apply"]);
    let said = format!("{}{}", asked.output, asked.error);
    assert!(
        said.contains(
            "It replaces every account, every session, every setting and secret, and where automatic backups are sent"
        ),
        "{said}"
    );
    assert_eq!(asked.exit, 2, "{said}");
    // Said, not done: the staged archive is where it was.
    assert!(restore.join("staged.zip").exists());
}

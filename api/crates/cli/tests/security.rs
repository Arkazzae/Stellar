//! The commands about an account's security, run on an instance on disk: what
//! `security show` leaves out, what `security recover` revokes and issues, and
//! how an identity link is removed. A port of the first tool's
//! `AccountSecurityCliTests`.

mod support;

use rusqlite::{Connection, params};
use serde_json::Value;
use stellarr_api::auth::tokens;
use stellarr_api::values::{Id, Timestamp};
use support::Instance;

fn user_id(connection: &Connection, name: &str) -> Id {
    connection
        .query_row(
            "SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn add_method(
    connection: &Connection,
    user: Id,
    kind: &str,
    name: &str,
    secret: Option<&str>,
    email: Option<&str>,
) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"ProtectedSecret\", \"EmailAddress\")
             VALUES (?1, ?2, ?3, ?4, '2026-11-10 09:00:00', '2026-11-10 09:01:00', ?5, ?6)",
            params![id, user, kind, name, secret, email],
        )
        .unwrap();
    id
}

fn count(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn security_show_exposes_metadata_but_never_seed_material() {
    let instance = Instance::started();
    let connection = instance.db();
    let jo = user_id(&connection, "jo");
    add_method(
        &connection,
        jo,
        "totp",
        "Phone",
        Some("seed-must-not-show"),
        None,
    );

    let show = instance.run(&["--json", "security", "show", "jo"]);

    assert_eq!(show.exit, 0, "{}", show.output);
    assert!(!show.output.contains("seed-must-not-show"));
    let data = show.json()["data"].clone();
    let method = &data["methods"][0];
    assert_eq!(
        (
            method["name"].as_str(),
            method["type"].as_str(),
            method["confirmed"].as_bool()
        ),
        (Some("Phone"), Some("totp"), Some(true))
    );
    assert_eq!(data["policy"]["mfaRequirement"], "optional");
    let methods = instance.run(&["--json", "security", "methods", "jo"]);
    assert_eq!(methods.exit, 0);
    assert_eq!(methods.json()["command"], "security.methods");
}

#[test]
fn recover_revokes_everything_and_issues_only_an_enrollment_credential() {
    let instance = Instance::started();
    let connection = instance.db();
    let jo = user_id(&connection, "jo");
    add_method(&connection, jo, "totp", "Phone", Some("seed"), None);
    add_method(
        &connection,
        jo,
        "email",
        "Mail",
        None,
        Some("JO@EXAMPLE.TEST"),
    );
    connection
        .execute(
            "UPDATE \"Users\" SET \"NormalizedEmail\" = 'JO@EXAMPLE.TEST' WHERE \"Id\" = ?1",
            [jo],
        )
        .unwrap();
    instance.session(jo, "Browser");
    connection
        .execute(
            "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, 'hash', 'Laptop', 'Firefox', '10.0.0.1', '2026-11-10 09:00:00', '2030-01-01 00:00:00')",
            params![Id::random(), jo],
        )
        .unwrap();
    let handoff = Id::random();
    connection
        .execute(
            "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\")
             VALUES (?1, 's', ?2, 1, '2026-11-24 10:00:00', '2030-01-01 00:00:00', '2026-11-24 10:00:01')",
            params![handoff, jo],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"RecoveryCodes\" (\"Id\", \"UserId\", \"BatchId\", \"CodeHash\", \"CreatedAt\") VALUES (?1, ?2, ?3, 'code-hash', '2026-11-10 09:02:00')",
            params![Id::random(), jo, Id::random()],
        )
        .unwrap();

    let recover = instance.run(&[
        "--json",
        "security",
        "recover",
        "jo",
        "--reason",
        "lost phone",
        "--yes",
    ]);

    assert_eq!(recover.exit, 0, "{}", recover.output);
    let data = recover.json()["data"].clone();
    assert_eq!(data["methodsRevoked"], 2);
    assert_eq!(data["trustedDevicesRevoked"], 1);
    assert_eq!(data["sessionsRevoked"], 1);
    assert_eq!(data["handoffsInvalidated"], 1);
    assert_eq!(data["securityRecoveryRequired"], true);
    let credential = data["recoveryCredential"].as_str().unwrap().to_string();
    assert!(recover.output.contains(&credential));
    let expires = Timestamp::parse(data["recoveryExpiresAt"].as_str().unwrap()).unwrap();
    assert!(expires > Timestamp::now().plus(std::time::Duration::from_secs(8 * 60)));

    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"RevokedAt\" IS NULL"
        ),
        0
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"
        ),
        0
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL"
        ),
        0
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UsedAt\" IS NULL"
        ),
        0
    );
    let redeemed: Option<Timestamp> = connection
        .query_row(
            "SELECT \"RedeemedAt\" FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
            [handoff],
            |r| r.get(0),
        )
        .unwrap();
    assert!(redeemed.is_some());
    // The credential is kept only as a digest, and can only enrol a new step.
    let (hash, reason): (String, String) = connection
        .query_row(
            "SELECT \"TokenHash\", \"Reason\" FROM \"CliRecoveryCredentials\"",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (hash, reason.as_str()),
        (tokens::hash(&credential), "lost phone")
    );
    let operation: String = connection
        .query_row(
            "SELECT \"Operation\" FROM \"OperatorAuditEntries\"",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(operation, "security.recover");
    let (version, required): (i64, bool) = connection
        .query_row("SELECT \"SessionVersion\", \"SecurityRecoveryRequired\" FROM \"Users\" WHERE \"Id\" = ?1", [jo], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((version, required), (2, true));
    let (kind, detail): (String, String) = connection
        .query_row(
            "SELECT \"Kind\", \"Detail\" FROM \"SecurityEvents\"",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "cli-recovery");
    assert!(detail.contains("lost phone"));
    let recipient: String = connection
        .query_row(
            "SELECT \"EmailAddress\" FROM \"SecurityNotifications\"",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recipient, "jo@example.test");
}

#[test]
fn security_settings_use_shared_validation_and_never_break_optional_defaults() {
    let instance = Instance::started();

    let before = instance.run(&["--json", "config", "get", "mfa.requirement"]);
    assert_eq!(before.json()["data"]["value"], "optional");
    let set = instance.run(&[
        "--json",
        "config",
        "set",
        "mfa.requirement",
        "Everyone",
        "--yes",
    ]);
    assert_eq!(set.exit, 0);
    assert_eq!(set.json()["data"]["value"], "everyone");
    let invalid = instance.run(&[
        "--json",
        "config",
        "set",
        "mfa.requirement",
        "sometimes",
        "--yes",
    ]);
    assert_eq!(
        (invalid.exit, invalid.code().as_str()),
        (2, "invalid_value")
    );
    let range = instance.run(&[
        "--json",
        "config",
        "set",
        "mfa.email-code-minutes",
        "61",
        "--yes",
    ]);
    assert_eq!((range.exit, range.code().as_str()), (2, "invalid_value"));

    let (requirement, minutes): (String, i64) = instance
        .db()
        .query_row(
            "SELECT \"MfaRequirement\", \"EmailCodeMinutes\" FROM \"OperationsSettings\"",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((requirement.as_str(), minutes), ("everyone", 8));
}

fn link(connection: &Connection, user: Id, issuer: &str, provider: Option<&str>) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, ?5, '2026-11-12 09:00:00')",
            params![id, user, issuer, id.to_string(), provider],
        )
        .unwrap();
    id
}

#[test]
fn an_operator_sees_and_removes_an_identity_link_that_a_recovery_keeps() {
    let instance = Instance::started();
    let connection = instance.db();
    let jo = user_id(&connection, "jo");
    let identity = link(&connection, jo, "https://id.example.test", Some("work"));

    let show = instance.run(&["--json", "security", "show", "jo"]);
    let identities = show.json()["data"]["identities"].clone();
    assert_eq!(identities[0]["id"], identity.to_string());
    assert_eq!(identities[0]["issuer"], "https://id.example.test");
    assert_eq!(identities[0]["providerId"], "work");
    instance.run(&[
        "--json", "security", "recover", "jo", "--reason", "abc", "--yes",
    ]);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM \"ExternalIdentities\""),
        1,
        "a recovery keeps links"
    );

    let unlink = instance.run(&[
        "--json",
        "security",
        "unlink",
        "jo",
        &identity.to_string(),
        "--yes",
    ]);

    assert_eq!(unlink.exit, 0, "{}", unlink.output);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM \"ExternalIdentities\""),
        0
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'identity-unlinked'"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM \"OperatorAuditEntries\" WHERE \"Operation\" = 'security.unlink'"
        ),
        1
    );
    let again = instance.run(&[
        "--json",
        "security",
        "unlink",
        "jo",
        &identity.to_string(),
        "--yes",
    ]);
    assert_eq!(
        (again.exit, again.code().as_str()),
        (3, "identity_not_found")
    );
}

#[test]
fn unlink_warns_of_the_last_way_in_as_the_server_counts_it() {
    let instance = Instance::started();
    let connection = instance.db();
    let jo = user_id(&connection, "jo");
    connection
        .execute(
            "UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1",
            [jo],
        )
        .unwrap();
    let lab = link(&connection, jo, "https://lab.example.test", Some("lab"));
    link(&connection, jo, "https://work.example.test", Some("work"));
    let configured = |ids: &[&str]| -> Vec<(String, String)> {
        ids.iter()
            .flat_map(|id| {
                [
                    (
                        format!("Oidc__Providers__{id}__Authority"),
                        "https://idp.example.test".to_string(),
                    ),
                    (
                        format!("Oidc__Providers__{id}__ClientId"),
                        "client".to_string(),
                    ),
                    (
                        format!("Oidc__Providers__{id}__ClientSecret"),
                        "secret".to_string(),
                    ),
                ]
            })
            .collect()
    };
    let ask = |providers: &[&str]| {
        let owned = configured(providers);
        let environment: Vec<(&str, &str)> = owned
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        instance.run_with(
            &["security", "unlink", "jo", &lab.to_string()],
            "n\n",
            true,
            &environment,
        )
    };

    // `work` is still configured, so `lab` is not the last way in.
    let here = ask(&["lab", "work"]);
    assert!(here.output.contains("Will unlink"), "{}", here.output);
    assert!(!here.output.contains("last way in"));
    // Only `lab` is configured: `work` has no button to press.
    let gone = ask(&["lab"]);
    assert!(gone.output.contains("last way in"), "{}", gone.output);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM \"ExternalIdentities\""),
        2
    );
}

#[test]
fn config_lists_the_identity_providers_without_their_secrets() {
    let instance = Instance::started();
    let environment = [
        (
            "Oidc__Providers__work__Authority",
            "https://work.example.test",
        ),
        ("Oidc__Providers__work__ClientSecret", "never-printed"),
        ("Oidc__Providers__work__TrustProviderMfa", "true"),
        (
            "Oidc__Providers__lab__Authority",
            "https://lab.example.test",
        ),
    ];

    let providers =
        instance.run_with_environment(&environment, &["--json", "config", "get", "oidc.providers"]);

    let data = providers.json()["data"].clone();
    assert_eq!(data["value"], "lab, work (trusts provider MFA)");
    assert_eq!(data["source"], "environment");
    assert!(!providers.output.contains("never-printed"));
    assert_eq!(
        instance
            .run(&["--json", "config", "get", "oidc.trust-provider-mfa"])
            .json()["data"]["value"],
        "false"
    );
    assert_eq!(
        instance
            .run(&["--json", "config", "get", "oidc.provider-mfa-amr"])
            .json()["data"]["value"],
        "mfa"
    );
}

#[test]
fn disabled_accounts_cannot_be_recovered() {
    let instance = Instance::started();
    instance
        .db()
        .execute(
            "UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'jo'",
            [],
        )
        .unwrap();

    let recover = instance.run(&[
        "--json",
        "security",
        "recover",
        "jo",
        "--reason",
        "lost phone",
        "--yes",
    ]);

    assert_eq!(
        (recover.exit, recover.code().as_str()),
        (4, "account_disabled")
    );
    assert_eq!(
        count(
            &instance.db(),
            "SELECT COUNT(*) FROM \"CliRecoveryCredentials\""
        ),
        0
    );
}

fn version(instance: &Instance) -> i64 {
    instance
        .db()
        .query_row(
            "SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Username\" = 'jo'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_recovery_asked_before_another_cannot_reuse_the_older_security_version() {
    let instance = std::rc::Rc::new(Instance::started());
    let first = version(&instance);
    let other = std::rc::Rc::clone(&instance);

    // While the first waits for its answer, a second recovery goes through.
    let run = instance.run_asked(
        &["--json", "security", "recover", "jo", "--reason", "first"],
        "y\n",
        move || {
            let second = other.run(&[
                "--json", "security", "recover", "jo", "--reason", "second", "--yes",
            ]);
            assert_eq!(second.exit, 0);
        },
    );

    assert_eq!(run.exit, 0, "{}", run.output);
    assert_eq!(version(&instance), first + 2);
    let credentials = count(
        &instance.db(),
        "SELECT COUNT(*) FROM \"CliRecoveryCredentials\" WHERE \"UsedAt\" IS NULL",
    );
    assert_eq!(credentials, 1, "only the last credential is good");
    let sessions: Value = run.json();
    assert_eq!(sessions["data"]["sessionVersion"], first + 2);
}

#[test]
fn disabling_while_the_question_waits_makes_the_recovery_fail() {
    let instance = std::rc::Rc::new(Instance::started());
    let first = version(&instance);
    let other = std::rc::Rc::clone(&instance);

    let run = instance.run_asked(
        &[
            "--json",
            "security",
            "recover",
            "jo",
            "--reason",
            "lost phone",
        ],
        "y\n",
        move || {
            other
                .db()
                .execute(
                    "UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'jo'",
                    [],
                )
                .unwrap();
        },
    );

    assert_eq!((run.exit, run.code().as_str()), (4, "account_disabled"));
    assert_eq!(version(&instance), first);
    let required: bool = instance
        .db()
        .query_row(
            "SELECT \"SecurityRecoveryRequired\" FROM \"Users\" WHERE \"Username\" = 'jo'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!required);
}

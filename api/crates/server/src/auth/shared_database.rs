//! What these modules make of the rows the first server wrote, and what it
//! makes of theirs: the two halves of `tests/oracle-security-shared-database.sh`,
//! which runs them on copies of one database file.
//!
//! Not part of the ordinary run. Each half reads its settings from the
//! environment, works on the copy of the database it is given, and prints one
//! line a check, which the script compares with what it expects.

use rusqlite::Connection;
use stellarr_messages::{Language, Messages};

use super::email_codes::{self, Scope};
use super::flow::Context;
use super::policy::SecurityPolicy;
use super::{mfa_methods, recovery_codes, tokens, trusted_devices};
use crate::seal::Sealer;
use crate::values::{Id, Timestamp};

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn open() -> Connection {
    Connection::open(setting("STELLARR_SHARED_DB")).expect("the copy of the database opens")
}

/// The keys of this server are not the first server's, whatever they are.
fn sealer() -> Sealer {
    Sealer::from_key([0x5a; 32])
}

fn report(name: &str, value: impl std::fmt::Display) {
    println!("RESULT {name}: {value}");
}

/// Reads what the first server wrote: the digests this side can use, and the
/// sealed values it cannot.
#[test]
#[ignore = "runs on a copy of a database the first server wrote"]
fn read_what_the_first_server_wrote() {
    let mut connection = open();
    let user = Id::parse(&setting("STELLARR_SHARED_USER")).expect("a user id");
    let now = Timestamp::now();

    // Recovery codes: stored as digests, so every unused one is good here,
    // typed as a person would, and the one the first server spent is not.
    report(
        "recovery codes remaining",
        recovery_codes::remaining(&connection, user).unwrap(),
    );
    let tx = connection.transaction().unwrap();
    report(
        "recovery code the first server spent",
        recovery_codes::consume(&tx, user, &setting("STELLARR_SHARED_SPENT_CODE"), now).unwrap(),
    );
    let unused = setting("STELLARR_SHARED_UNUSED_CODE");
    report(
        "recovery code the first server left, typed in lower case",
        recovery_codes::consume(&tx, user, &unused.to_lowercase(), now).unwrap(),
    );
    report(
        "the same code again",
        recovery_codes::consume(&tx, user, &unused, now).unwrap(),
    );
    tx.commit().unwrap();
    report(
        "recovery codes remaining after",
        recovery_codes::remaining(&connection, user).unwrap(),
    );

    // The remembered device: stored as a digest of the cookie.
    let token = setting("STELLARR_SHARED_TRUST_TOKEN");
    report(
        "trusted device found by the first server's cookie",
        trusted_devices::find(&connection, user, Some(&token), now)
            .unwrap()
            .is_some(),
    );
    report(
        "trusted device found by another cookie",
        trusted_devices::find(&connection, user, Some("another"), now)
            .unwrap()
            .is_some(),
    );

    // The authenticator app: sealed by the first server's own keys.
    let method = Id::parse(&setting("STELLARR_SHARED_TOTP_METHOD")).unwrap();
    let found = mfa_methods::find_active(&connection, user, method)
        .unwrap()
        .unwrap();
    report(
        "authenticator app method read",
        format!("{} {}", found.kind, found.name),
    );
    let code = setting("STELLARR_SHARED_TOTP_CODE");
    report(
        "authenticator app code accepted",
        mfa_methods::totp_step_of(&sealer(), &found, &code, now).is_some(),
    );

    // The mailed code: sealed too.
    let challenge = Id::parse(&setting("STELLARR_SHARED_EMAIL_CHALLENGE")).unwrap();
    let mail_method = Id::parse(&setting("STELLARR_SHARED_EMAIL_METHOD"));
    let purpose = setting("STELLARR_SHARED_EMAIL_PURPOSE");
    let scope = Scope {
        user,
        challenge,
        purpose: &purpose,
        method: mail_method,
        address: "maya@example.test",
    };
    let tx = connection.transaction().unwrap();
    let accepted = email_codes::check(
        &tx,
        &sealer(),
        &scope,
        &setting("STELLARR_SHARED_EMAIL_CODE"),
        now,
    )
    .unwrap();
    tx.commit().unwrap();
    report("mailed code accepted", accepted.is_some());
    let attempts: i64 = connection
        .query_row(
            "SELECT \"Attempts\" FROM \"EmailOtpCodes\" WHERE \"ChallengeId\" = ?1",
            [challenge],
            |row| row.get(0),
        )
        .unwrap();
    report("attempts the mailed code was charged", attempts);

    // The shapes of the rows, side by side.
    let hash: String = connection
        .query_row(
            "SELECT \"CodeHash\" FROM \"RecoveryCodes\" WHERE \"UsedAt\" IS NULL LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    report(
        "recovery digest looks like ours",
        hash.len() == 64 && hash == hash.to_uppercase(),
    );
    report("recovery digest of the typed code is the stored one", {
        let stored: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"CodeHash\" = ?1",
                [tokens::hash(&super::recovery_codes::normalize(&unused))],
                |row| row.get(0),
            )
            .unwrap();
        stored == 1
    });
}

/// Writes what the first server is to read: a set of recovery codes and a
/// remembered device, for an account that has an authenticator app of the
/// first server's. Writes the codes and the token to the file named.
#[test]
#[ignore = "runs on a copy of a database the first server wrote"]
fn write_what_the_first_server_reads() {
    let mut connection = open();
    let user = Id::parse(&setting("STELLARR_SHARED_USER")).expect("a user id");
    let now = Timestamp::now();
    let tx = connection.transaction().unwrap();
    let codes = recovery_codes::regenerate(&tx, user, now).unwrap();
    let context = Context {
        ip_address: "203.0.113.9",
        user_agent: "Rust server test browser",
        https: false,
        trusted_device_cookie: None,
        passkeys_available: false,
    };
    let device = trusted_devices::issue(
        &tx,
        &context,
        &Messages::new(Language::English),
        user,
        &SecurityPolicy::default(),
        now,
    )
    .unwrap();
    tx.commit().unwrap();
    let token = device
        .cookie
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_string();
    let mut lines = vec![format!("TRUST {token}")];
    lines.extend(codes.iter().map(|code| format!("CODE {code}")));
    std::fs::write(setting("STELLARR_SHARED_OUT"), lines.join("\n") + "\n").unwrap();
}

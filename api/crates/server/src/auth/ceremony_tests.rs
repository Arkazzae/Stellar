//! The ceremonies as a route calls them: nothing here reaches below the calls
//! of `auth::ceremony`, and each test looks at the rows afterwards.

use std::time::Duration;

use rusqlite::{Connection, params};
use stellarr_webauthn::Algorithm;

use super::ceremony::*;
use super::ceremony::{PasswordCheck, check_password, may_change_security};
use super::desktop_exchange::{Exchanged, exchange_browser_login};
use super::email_codes::{ENROLLMENT, LOGIN};
use super::email_send::Sent;
use super::fixtures::{
    add_challenge, add_method, add_pending_method, add_session, add_user, events_of, preauth_of,
};
use super::flow::{
    Context, PURPOSE_ENROLLMENT, PURPOSE_MFA, PURPOSE_RECOVERY, PURPOSE_REVERIFY, StoredChallenge,
};
use super::mfa_methods::{self, EMAIL};
use super::passkeys::authenticator::{self as soft, AssertionSpec, RegistrationSpec};
use super::session::Caller;
use super::{passkeys, recovery, recovery_codes, security_cookies, totp, trusted_devices};
use crate::database::DbError;
use crate::mail::tests::support::{Behaviour, FakeSmtp, Saved, found, instance};
use crate::state::AppState;
use crate::testing::TempDir;
use crate::values::{Id, Timestamp};

const CONTEXT: Context<'static> = Context {
    ip_address: "203.0.113.9",
    user_agent: "Firefox/130",
    https: false,
    trusted_device_cookie: None,
    passkeys_available: true,
};

struct World {
    _folder: TempDir,
    state: AppState,
}

impl World {
    fn new() -> World {
        let (folder, state) = instance(&[]);
        World {
            _folder: folder,
            state,
        }
    }

    fn db(&self) -> Connection {
        let connection = Connection::open(self.state.database().path()).unwrap();
        connection.busy_timeout(Duration::from_secs(10)).unwrap();
        connection
    }

    /// A desktop handoff approved and not used, and one nobody approved.
    fn handoffs(&self, user: Id) {
        let now = Timestamp::now();
        self.db()
            .execute(
                "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\", \"RedeemedAt\")
                 VALUES (?1, 'h', ?3, 1, ?4, ?4, ?4, NULL), (?2, 'h', ?3, 1, ?4, ?4, NULL, NULL)",
                params![Id::random(), Id::random(), user, now],
            )
            .unwrap();
    }

    fn count(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn user(&self, email: Option<&str>) -> Id {
        add_user(&self.db(), email)
    }

    fn challenge(&self, user: Id, purpose: &str) -> (StoredChallenge, (Id, String)) {
        let challenge = add_challenge(&self.db(), user, purpose);
        let preauth = preauth_of(&challenge);
        (challenge, preauth)
    }

    /// An account with a confirmed authenticator app; its secret.
    async fn with_app(&self, user: Id) -> (Id, String) {
        let state = self.state.clone();
        self.state
            .database()
            .write(move |tx| -> Result<(Id, String), DbError> {
                let now = Timestamp::now();
                let (method, secret) =
                    mfa_methods::start_totp(tx, state.sealer(), user, "Phone", now)?;
                assert!(mfa_methods::confirm_totp(
                    tx, user, method, 1, "Phone", now
                )?);
                Ok((method, secret))
            })
            .await
            .unwrap()
    }
}

fn ask<'a>(challenge: &'a StoredChallenge, preauth: &'a (Id, String), id: &'a str) -> Ask<'a> {
    let _ = challenge;
    Ask {
        context: &CONTEXT,
        preauth: Some(preauth),
        challenge_id: id,
    }
}

fn nothing(_: &rusqlite::Transaction, _: &Completed) -> Result<(), DbError> {
    Ok(())
}

fn now_code(secret: &str) -> String {
    totp::code_at(secret, Timestamp::now()).unwrap()
}

fn wrong_code(secret: &str) -> String {
    let right: Vec<String> = (-1..=1)
        .map(|o| {
            totp::code_at(
                secret,
                Timestamp::now()
                    .plus(Duration::from_secs((30 + o * 30).max(0) as u64))
                    .minus(Duration::from_secs(30)),
            )
            .unwrap()
        })
        .collect();
    (0..1_000_000)
        .map(|n| format!("{n:06}"))
        .find(|c| !right.contains(c))
        .unwrap()
}

async fn code_answer(
    world: &World,
    challenge: &StoredChallenge,
    preauth: &(Id, String),
    method: Option<Id>,
    code: &str,
    remember: bool,
) -> Answered<()> {
    let id = challenge.id.to_string();
    answer(
        &world.state,
        &ask(challenge, preauth, &id),
        Answer::Code {
            method: method.map(|id| id.to_string()).as_deref(),
            code,
        },
        remember,
        nothing,
    )
    .await
    .unwrap()
}

// ---- the three findings of the review, as the calls a route makes ----

#[tokio::test]
async fn an_ordinary_sign_in_challenge_cannot_be_used_to_add_a_second_step() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();

    let started = start_totp(&world.state, &ask(&challenge, &preauth, &id), None)
        .await
        .unwrap();

    assert!(matches!(started, TotpStarted::Invalid(_)), "{started:?}");
    assert_eq!(world.count("SELECT COUNT(*) FROM \"MfaMethods\""), 1);
    // A method that is waiting for its code cannot be confirmed through it either.
    let pending = {
        let state = world.state.clone();
        world
            .state
            .database()
            .write(move |tx| {
                mfa_methods::start_totp(tx, state.sealer(), user, "Other", Timestamp::now())
            })
            .await
            .unwrap()
    };
    let confirmed = confirm_totp(
        &world.state,
        &ask(&challenge, &preauth, &id),
        TotpConfirmation {
            method: Some(&pending.0.to_string()),
            name: None,
            code: &now_code(&pending.1),
        },
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(confirmed, Enrolled::Invalid(_)));
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"ConfirmedAt\" IS NOT NULL"),
        1
    );
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 0);
}

#[tokio::test]
async fn a_challenge_the_wrong_codes_closed_answers_nothing_even_to_the_right_code() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);

    for _ in 0..5 {
        assert!(matches!(
            code_answer(&world, &challenge, &preauth, Some(method), "000000", false).await,
            Answered::Refused
        ));
    }
    let late = code_answer(
        &world,
        &challenge,
        &preauth,
        Some(method),
        &now_code(&secret),
        false,
    )
    .await;

    assert!(matches!(late, Answered::Invalid(_)));
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 0);
}

#[tokio::test]
async fn refused_attempts_are_kept_however_often_they_come_and_end_in_a_closed_challenge() {
    let world = World::new();
    let user = world.user(None);
    let (method, _) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);

    let mut refused = 0;
    for _ in 0..50 {
        if matches!(
            code_answer(&world, &challenge, &preauth, Some(method), "000000", false).await,
            Answered::Refused
        ) {
            refused += 1;
        }
    }

    assert_eq!(refused, 5);
    assert_eq!(
        world.count("SELECT \"Attempts\" FROM \"AuthChallenges\""),
        5
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'mfa-failed'"),
        5
    );
    // The account's budget runs across challenges.
    let (second, second_preauth) = world.challenge(user, PURPOSE_MFA);
    for _ in 0..5 {
        let _ = code_answer(
            &world,
            &second,
            &second_preauth,
            Some(method),
            "000000",
            false,
        )
        .await;
    }
    let (third, third_preauth) = world.challenge(user, PURPOSE_MFA);
    let blocked = code_answer(
        &world,
        &third,
        &third_preauth,
        Some(method),
        "000000",
        false,
    )
    .await;
    assert!(matches!(blocked, Answered::TooManyFailures));
}

// ---- signing in ----

#[tokio::test]
async fn an_app_code_signs_in_a_session_that_passed_a_second_step_and_remembers_the_browser_on_request()
 {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    let (method, secret) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    // One code for both answers: a second one read later could belong to the
    // next period and would be a new proof, not a replay.
    let code = now_code(&secret);

    let answered = code_answer(&world, &challenge, &preauth, Some(method), &code, true).await;

    let Answered::SignedIn { completed, .. } = answered else {
        panic!("a sign-in was expected")
    };
    assert_eq!(completed.evidence, "totp");
    assert_eq!(completed.cookies.len(), 3);
    assert!(completed.cookies[0].starts_with("stellarr-dev-session="));
    assert!(completed.cookies[1].starts_with("stellarr-dev-trust="));
    assert!(completed.cookies[2].starts_with("stellarr-dev-preauth=;"));
    assert!(completed.trusted_device_expires_at.is_some());
    let db = world.db();
    let (evidence, fresh, version): (String, bool, i64) = db
        .query_row(
            "SELECT \"MfaEvidence\", \"MfaCompletedAt\" IS NOT NULL, \"SecurityVersion\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [completed.session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((evidence.as_str(), fresh, version), ("totp", true, 1));
    assert_eq!(world.count("SELECT COUNT(*) FROM \"TrustedDevices\""), 1);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"AuthChallenges\" WHERE \"ConsumedAt\" IS NOT NULL"),
        1
    );
    for kind in ["mfa-succeeded", "trusted-device-added"] {
        assert_eq!(
            world.count(&format!(
                "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = '{kind}'"
            )),
            1,
            "{kind}"
        );
    }
    // The same code is not good for a second sign-in.
    let (again, again_preauth) = world.challenge(user, PURPOSE_MFA);
    let replay = code_answer(&world, &again, &again_preauth, Some(method), &code, false).await;
    assert!(matches!(replay, Answered::Refused));
}

#[tokio::test]
async fn a_refused_link_leaves_the_proof_spent_and_the_person_signed_in() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();
    // One code for both answers, so a period turning between them cannot make
    // the second a new proof.
    let code = now_code(&secret);

    let answered = answer(
        &world.state,
        &ask(&challenge, &preauth, &id),
        Answer::Code {
            method: Some(&method.to_string()),
            code: &code,
        },
        true,
        |tx, completed| -> Result<(), DbError> {
            // Writes of its own, then a failure: the writes must be gone.
            tx.execute(
                "UPDATE \"Users\" SET \"Bio\" = 'linked' WHERE \"Id\" = ?1",
                [completed.user],
            )?;
            Err(DbError::Task("the link could not be written".into()))
        },
    )
    .await
    .unwrap();

    let Answered::SignedIn { completed, hook } = answered else {
        panic!("a sign-in was expected");
    };
    assert!(hook.is_err());
    assert_eq!(completed.cookies.len(), 3);
    // The proof stands: the session, the device, the spent challenge and step.
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 1);
    assert_eq!(world.count("SELECT COUNT(*) FROM \"TrustedDevices\""), 1);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"AuthChallenges\" WHERE \"ConsumedAt\" IS NOT NULL"),
        1
    );
    assert_eq!(
        world.count("SELECT \"LastTotpStep\" > 1 FROM \"MfaMethods\""),
        1
    );
    // The caller's own writes are not.
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"Users\" WHERE \"Bio\" = 'linked'"),
        0
    );
    // And the same code opens nothing again.
    let (again, again_preauth) = world.challenge(user, PURPOSE_MFA);
    assert!(matches!(
        code_answer(&world, &again, &again_preauth, Some(method), &code, false).await,
        Answered::Refused
    ));
}

#[tokio::test]
async fn a_hook_that_succeeds_keeps_its_writes_with_the_sign_in() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();

    let answered = answer(
        &world.state,
        &ask(&challenge, &preauth, &id),
        Answer::Code {
            method: Some(&method.to_string()),
            code: &now_code(&secret),
        },
        false,
        |tx, completed| -> Result<usize, DbError> {
            Ok(tx.execute(
                "UPDATE \"Users\" SET \"Bio\" = 'linked' WHERE \"Id\" = ?1",
                [completed.user],
            )?)
        },
    )
    .await
    .unwrap();

    let Answered::SignedIn { hook, .. } = answered else {
        panic!("a sign-in was expected");
    };
    assert_eq!(hook.unwrap(), 1);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"Users\" WHERE \"Bio\" = 'linked'"),
        1
    );
}

#[tokio::test]
async fn a_remembered_device_signs_in_without_making_the_session_fresh_or_being_remembered_again() {
    let world = World::new();
    let user = world.user(None);
    world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let device = {
        let state = world.state.clone();
        world
            .state
            .database()
            .write(move |tx| {
                trusted_devices::issue(
                    tx,
                    &CONTEXT,
                    state.messages(),
                    user,
                    &Default::default(),
                    Timestamp::now(),
                )
            })
            .await
            .unwrap()
    };
    let token = device
        .cookie
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_string();
    let context = Context {
        trusted_device_cookie: Some(&token),
        ..CONTEXT
    };
    let id = challenge.id.to_string();

    let answered = answer(
        &world.state,
        &Ask {
            context: &context,
            preauth: Some(&preauth),
            challenge_id: &id,
        },
        Answer::TrustedDevice,
        true,
        nothing,
    )
    .await
    .unwrap();

    let Answered::SignedIn { completed, .. } = answered else {
        panic!("a sign-in was expected")
    };
    let fresh: bool = world
        .db()
        .query_row(
            "SELECT \"MfaCompletedAt\" IS NOT NULL FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [completed.session],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!fresh, "a remembered device never makes a session fresh");
    assert_eq!(world.count("SELECT COUNT(*) FROM \"TrustedDevices\""), 1);
    assert!(completed.trusted_device_expires_at.is_none());
    assert_eq!(
        world.count(
            "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"MfaEvidence\" = 'trusted_device'"
        ),
        1
    );
}

#[tokio::test]
async fn a_recovery_code_signs_in_once_and_is_written_as_what_it_was() {
    let world = World::new();
    let user = world.user(None);
    world.with_app(user).await;
    let codes = world
        .state
        .database()
        .write(move |tx| recovery_codes::regenerate(tx, user, Timestamp::now()))
        .await
        .unwrap();
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);

    let first = code_answer(&world, &challenge, &preauth, None, &codes[0], false).await;

    assert!(matches!(first, Answered::SignedIn { .. }));
    assert_eq!(
        world
            .count("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'recovery-code-used'"),
        1
    );
    let (second, second_preauth) = world.challenge(user, PURPOSE_MFA);
    assert!(matches!(
        code_answer(&world, &second, &second_preauth, None, &codes[0], false).await,
        Answered::Refused
    ));
    assert!(matches!(
        code_answer(&world, &second, &second_preauth, None, "", false).await,
        Answered::EmptyCode
    ));
}

#[tokio::test]
async fn a_mailed_code_is_sent_and_answers_the_sign_in() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let world = World::new();
    found(
        &world.state,
        Saved {
            port: smtp.port,
            ..Saved::default()
        },
    )
    .await;
    let user = world.user(Some("maya@example.test"));
    let method = add_method(&world.db(), user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();

    let sent = send_login_code(
        &world.state,
        &ask(&challenge, &preauth, &id),
        Some(&method.to_string()),
    )
    .await
    .unwrap();

    assert!(
        matches!(&sent, Sent::Delivered { address, retry_after_seconds: 45 } if address == "maya@example.test"),
        "{sent:?}"
    );
    let data = smtp.mails()[0].data.clone();
    let digits = data
        .lines()
        .map(str::trim)
        .find(|l| l.len() == 6 && l.bytes().all(|b| b.is_ascii_digit()))
        .unwrap()
        .to_string();
    assert!(matches!(
        code_answer(&world, &challenge, &preauth, Some(method), &digits, false).await,
        Answered::SignedIn { .. }
    ));
    // A challenge for something else is sent nothing.
    let (enrolling, enrolling_preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let other = enrolling.id.to_string();
    let none = send_login_code(
        &world.state,
        &ask(&enrolling, &enrolling_preauth, &other),
        Some(&method.to_string()),
    )
    .await
    .unwrap();
    assert!(matches!(none, Sent::Invalid(_)));
    assert_eq!(
        world.count(&format!(
            "SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"Purpose\" = '{LOGIN}'"
        )),
        1
    );
}

// ---- setting a second step up ----

#[tokio::test]
async fn an_app_is_set_up_and_signs_the_person_in_with_recovery_codes_once() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    let (challenge, preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let id = challenge.id.to_string();
    let TotpStarted::Started {
        method,
        secret,
        uri,
    } = start_totp(&world.state, &ask(&challenge, &preauth, &id), Some("Phone"))
        .await
        .unwrap()
    else {
        panic!("a method was expected");
    };
    assert!(uri.starts_with("otpauth://totp/Stellarr%3A"));
    let too_long = "n".repeat(65);

    let invalid = confirm_totp(
        &world.state,
        &ask(&challenge, &preauth, &id),
        TotpConfirmation {
            method: Some(&method.to_string()),
            name: Some(&too_long),
            code: &now_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(invalid, Enrolled::InvalidName));
    let wrong = confirm_totp(
        &world.state,
        &ask(&challenge, &preauth, &id),
        TotpConfirmation {
            method: Some(&method.to_string()),
            name: None,
            code: &wrong_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(wrong, Enrolled::Refused));
    assert_eq!(
        world.count("SELECT \"Attempts\" FROM \"AuthChallenges\""),
        1
    );

    let done = confirm_totp(
        &world.state,
        &ask(&challenge, &preauth, &id),
        TotpConfirmation {
            method: Some(&method.to_string()),
            name: Some("Mine"),
            code: &now_code(&secret),
        },
        true,
        nothing,
    )
    .await
    .unwrap();

    let Enrolled::Done { completed, .. } = done else {
        panic!("the app was expected to be set up")
    };
    assert_eq!(completed.recovery_codes.as_ref().map(Vec::len), Some(10));
    assert_eq!(completed.security_version, 2);
    assert_eq!(world.count("SELECT \"SessionVersion\" FROM \"Users\""), 2);
    assert_eq!(world.count("SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"ConfirmedAt\" IS NOT NULL AND \"Name\" = 'Mine'"), 1);
    assert_eq!(world.count("SELECT COUNT(*) FROM \"TrustedDevices\""), 1);
    for kind in ["method-added", "mfa-succeeded", "trusted-device-added"] {
        assert_eq!(
            world.count(&format!(
                "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = '{kind}'"
            )),
            1,
            "{kind}"
        );
    }
    // Setting up a second one does not make a new set of codes.
    let (second, second_preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let second_id = second.id.to_string();
    let TotpStarted::Started { method, secret, .. } = start_totp(
        &world.state,
        &ask(&second, &second_preauth, &second_id),
        None,
    )
    .await
    .unwrap() else {
        panic!("a method was expected");
    };
    world
        .db()
        .execute("UPDATE \"MfaMethods\" SET \"LastTotpStep\" = 0", [])
        .unwrap();
    let again = confirm_totp(
        &world.state,
        &ask(&second, &second_preauth, &second_id),
        TotpConfirmation {
            method: Some(&method.to_string()),
            name: None,
            code: &now_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();
    let Enrolled::Done { completed, .. } = again else {
        panic!("expected done")
    };
    assert!(completed.recovery_codes.is_none());
}

#[tokio::test]
async fn an_address_is_added_with_its_mailed_code_and_a_bad_name_keeps_the_code() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let world = World::new();
    found(
        &world.state,
        Saved {
            port: smtp.port,
            ..Saved::default()
        },
    )
    .await;
    let user = world.user(Some("maya@example.test"));
    let (challenge, preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let id = challenge.id.to_string();
    let sent = send_enrollment_code(&world.state, &ask(&challenge, &preauth, &id))
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Delivered { .. }), "{sent:?}");
    let data = smtp.mails()[0].data.clone();
    let digits = data
        .lines()
        .map(str::trim)
        .find(|l| l.len() == 6 && l.bytes().all(|b| b.is_ascii_digit()))
        .unwrap()
        .to_string();

    let bad = confirm_email(
        &world.state,
        &ask(&challenge, &preauth, &id),
        Some(&"n".repeat(65)),
        &digits,
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(bad, Enrolled::InvalidName));
    let done = confirm_email(
        &world.state,
        &ask(&challenge, &preauth, &id),
        None,
        &digits,
        false,
        nothing,
    )
    .await
    .unwrap();

    let Enrolled::Done { completed, .. } = done else {
        panic!("expected done")
    };
    assert_eq!(completed.evidence, "email");
    assert_eq!(world.count(&format!("SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"Purpose\" = '{ENROLLMENT}' AND \"ConsumedAt\" IS NOT NULL")), 1);
    assert_eq!(world.count("SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"Type\" = 'email' AND \"EmailAddress\" = 'MAYA@EXAMPLE.TEST'"), 1);
    let alert: String = world
        .db()
        .query_row(
            "SELECT \"EmailAddress\" FROM \"SecurityNotifications\" LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(alert, "maya@example.test");
}

#[tokio::test]
async fn a_passkey_is_added_answers_a_second_step_and_signs_in_alone() {
    let world = World::new();
    let user = world.user(None);
    let rp = soft::relying_party();
    let (challenge, preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let id = challenge.id.to_string();
    let PasskeyStarted::Options(options) =
        start_passkey(&world.state, &ask(&challenge, &preauth, &id), &rp)
            .await
            .unwrap()
    else {
        panic!("options were expected");
    };
    let kept = passkeys::read_kept(&options).unwrap();
    let mut spec = RegistrationSpec::new(&kept.challenge);
    spec.credential_id = vec![9; 32];
    let response = spec.build(soft::key(Algorithm::Es256));

    let done = confirm_passkey(
        &world.state,
        &ask(&challenge, &preauth, &id),
        &rp,
        Some("Key"),
        Some(&response),
        false,
        nothing,
    )
    .await
    .unwrap();

    let Enrolled::Done { completed, .. } = done else {
        panic!("expected done")
    };
    assert_eq!(completed.evidence, "passkey");
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"WebAuthnCredentials\""),
        1
    );
    // A second step with it.
    let (signing, signing_preauth) = world.challenge(user, PURPOSE_MFA);
    let signing_id = signing.id.to_string();
    let PasskeyOptions::Options(options) = passkey_options(
        &world.state,
        &ask(&signing, &signing_preauth, &signing_id),
        &rp,
    )
    .await
    .unwrap() else {
        panic!("options were expected");
    };
    let kept = passkeys::read_kept(&options).unwrap();
    let mut assertion = AssertionSpec::new(&[9; 32], &kept.challenge);
    assertion.sign_count = 1;
    let response = assertion.build(soft::key(Algorithm::Es256));
    let signed = answer(
        &world.state,
        &ask(&signing, &signing_preauth, &signing_id),
        Answer::Passkey {
            relying_party: &rp,
            response: Some(&response),
        },
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(signed, Answered::SignedIn { .. }));
    // Alone: a wrong answer spends the options and is kept; a right one after it finds none.
    let begun = passkey_sign_in_options(&world.state, &rp, false)
        .await
        .unwrap();
    let value = begun
        .cookie
        .strip_prefix("stellarr-dev-passkey=")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let cookie = security_cookies::parse_bound(value).unwrap();
    let kept = passkeys::read_kept(&begun.json).unwrap();
    let handle = stellarr_webauthn::b64::encode(&passkeys::user_handle(user));
    let mut forged = AssertionSpec::new(&[9; 32], &kept.challenge);
    forged.sign_count = 2;
    forged.user_handle = Some(handle.clone());
    let forged = forged.build(soft::other_key(Algorithm::Es256));
    let mut right = AssertionSpec::new(&[9; 32], &kept.challenge);
    right.sign_count = 2;
    right.user_handle = Some(handle);
    let right = right.build(soft::key(Algorithm::Es256));
    let first = passkey_sign_in(
        &world.state,
        &CONTEXT,
        &rp,
        Some(&cookie),
        Some(&forged),
        true,
    )
    .await
    .unwrap();
    assert!(matches!(first, PasskeySignedIn::Rejected { .. }));
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'login-failed'"),
        1
    );
    let second = passkey_sign_in(
        &world.state,
        &CONTEXT,
        &rp,
        Some(&cookie),
        Some(&right),
        true,
    )
    .await
    .unwrap();
    assert!(matches!(second, PasskeySignedIn::Rejected { .. }));
    // And a fresh set of options works, with a session that passed a second step.
    let begun = passkey_sign_in_options(&world.state, &rp, false)
        .await
        .unwrap();
    let value = begun
        .cookie
        .strip_prefix("stellarr-dev-passkey=")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let cookie = security_cookies::parse_bound(value).unwrap();
    let kept = passkeys::read_kept(&begun.json).unwrap();
    let mut spec = AssertionSpec::new(&[9; 32], &kept.challenge);
    spec.sign_count = 3;
    spec.user_handle = Some(stellarr_webauthn::b64::encode(&passkeys::user_handle(user)));
    let response = spec.build(soft::key(Algorithm::Es256));
    let PasskeySignedIn::SignedIn(completed) = passkey_sign_in(
        &world.state,
        &CONTEXT,
        &rp,
        Some(&cookie),
        Some(&response),
        false,
    )
    .await
    .unwrap() else {
        panic!("a sign-in was expected");
    };
    let (evidence, fresh, short): (String, bool, bool) = world
        .db()
        .query_row(
            "SELECT \"MfaEvidence\", \"MfaCompletedAt\" IS NOT NULL, \"ShortLived\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [completed.session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((evidence.as_str(), fresh, short), ("passkey", true, true));
    assert_eq!(completed.cookies.len(), 2);
}

// ---- from inside a session ----

fn caller(world: &World, user: Id, passed: Option<u64>) -> Caller {
    let now = Timestamp::now();
    let session = add_session(
        &world.db(),
        user,
        now,
        passed.map(|s| now.minus(Duration::from_secs(s))),
    );
    Caller { user, session }
}

#[tokio::test]
async fn proving_a_second_step_again_gives_a_new_session_and_leaves_the_old_one() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let me = caller(&world, user, Some(3600));
    let Started::Challenge { body, cookie } = start_from_session(
        &world.state,
        me,
        &CONTEXT,
        FromSession::Reverification,
        None,
    )
    .await
    .unwrap() else {
        panic!("a challenge was expected");
    };
    let id = serde_json::to_value(&*body).unwrap()["challengeId"]
        .as_str()
        .unwrap()
        .to_string();
    let preauth = security_cookies::parse_bound(
        cookie
            .strip_prefix("stellarr-dev-preauth=")
            .unwrap()
            .split(';')
            .next()
            .unwrap(),
    )
    .unwrap();
    let purpose: String = world
        .db()
        .query_row("SELECT \"Purpose\" FROM \"AuthChallenges\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(purpose, PURPOSE_REVERIFY);
    // The challenge is tied to the session it was asked from.
    let parent: Option<Id> = world
        .db()
        .query_row(
            "SELECT \"ParentSessionId\" FROM \"AuthChallenges\"",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent, Some(me.session));

    let answered = answer(
        &world.state,
        &Ask {
            context: &CONTEXT,
            preauth: Some(&preauth),
            challenge_id: &id,
        },
        Answer::Code {
            method: Some(&method.to_string()),
            code: &now_code(&secret),
        },
        true,
        nothing,
    )
    .await
    .unwrap();

    let Answered::SignedIn { completed, .. } = answered else {
        panic!("a sign-in was expected")
    };
    assert_ne!(completed.session, me.session);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL"),
        2
    );
    // And a stale session is not allowed to start one for enrolment; an account without a method has nothing to prove again.
    let stale = caller(&world, user, Some(3600));
    assert!(matches!(
        start_from_session(&world.state, stale, &CONTEXT, FromSession::Enrollment, None)
            .await
            .unwrap(),
        Started::NotAllowed
    ));
    let other = world.user(None);
    let session = caller(&world, other, None);
    assert!(matches!(
        start_from_session(
            &world.state,
            session,
            &CONTEXT,
            FromSession::Reverification,
            None
        )
        .await
        .unwrap(),
        Started::NoMethods
    ));
}

#[tokio::test]
async fn taking_a_method_away_ends_the_other_sessions_and_devices_and_keeps_this_one() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    let (app, _) = world.with_app(user).await;
    let mail = add_method(&world.db(), user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
    let me = caller(&world, user, Some(60));
    let other_session = add_session(&world.db(), user, Timestamp::now(), None);
    world
        .db()
        .execute("UPDATE \"UserSessions\" SET \"SecurityVersion\" = 1", [])
        .unwrap();
    {
        let state = world.state.clone();
        world
            .state
            .database()
            .write(move |tx| {
                trusted_devices::issue(
                    tx,
                    &CONTEXT,
                    state.messages(),
                    user,
                    &Default::default(),
                    Timestamp::now(),
                )
                .map(|_| ())
            })
            .await
            .unwrap();
    }
    let providers = super::ways_in::OwnedProviders {
        configured: vec![],
        default_id: "default".into(),
    };

    let stale = caller(&world, user, Some(3600));
    assert_eq!(
        remove_method(&world.state, stale, &CONTEXT, app, &providers, None)
            .await
            .unwrap(),
        Removed::NotAllowed
    );
    assert_eq!(
        remove_method(&world.state, me, &CONTEXT, Id::random(), &providers, None)
            .await
            .unwrap(),
        Removed::NotFound
    );
    let removed = remove_method(&world.state, me, &CONTEXT, app, &providers, None)
        .await
        .unwrap();

    let Removed::Done { cookie } = removed else {
        panic!("expected done, got {removed:?}")
    };
    assert!(cookie.starts_with("stellarr-dev-session="));
    let db = world.db();
    let revoked: bool = db
        .query_row(
            "SELECT \"RevokedAt\" IS NOT NULL FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [other_session],
            |r| r.get(0),
        )
        .unwrap();
    let (mine_revoked, version): (bool, i64) = db
        .query_row("SELECT \"RevokedAt\" IS NOT NULL, \"SecurityVersion\" FROM \"UserSessions\" WHERE \"Id\" = ?1", [me.session], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert!(revoked && !mine_revoked);
    assert_eq!(version, 2);
    assert_eq!(
        world.count("SELECT \"SessionVersion\" FROM \"Users\" WHERE 1 LIMIT 1"),
        2
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"),
        0
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'method-removed'"),
        1
    );
    // Taking the last one away spends the recovery codes.
    let codes = world
        .state
        .database()
        .write(move |tx| recovery_codes::regenerate(tx, user, Timestamp::now()))
        .await
        .unwrap();
    assert_eq!(codes.len(), 10);
    let me = caller(&world, user, Some(60));
    world
        .db()
        .execute("UPDATE \"UserSessions\" SET \"SecurityVersion\" = 2", [])
        .unwrap();
    assert!(matches!(
        remove_method(&world.state, me, &CONTEXT, mail, &providers, None)
            .await
            .unwrap(),
        Removed::Done { .. }
    ));
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UsedAt\" IS NULL"),
        0
    );
}

#[tokio::test]
async fn the_last_way_in_and_the_policy_stop_a_removal() {
    let world = World::new();
    let user = world.user(None);
    world
        .db()
        .execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL", [])
        .unwrap();
    let passkey = add_method(&world.db(), user, "passkey", None);
    let me = caller(&world, user, Some(60));
    let providers = super::ways_in::OwnedProviders {
        configured: vec![],
        default_id: "default".into(),
    };

    assert_eq!(
        remove_method(&world.state, me, &CONTEXT, passkey, &providers, None)
            .await
            .unwrap(),
        Removed::LastWayIn
    );

    world
        .db()
        .execute("UPDATE \"Users\" SET \"PasswordHash\" = 'x'", [])
        .unwrap();
    let server = crate::auth::fixtures::add_instance_server(&world.db());
    world.db().execute(
        "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\")
         VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, '2026-11-24 00:00:00', 'everyone')",
        params![server],
    ).unwrap();
    assert_eq!(
        remove_method(&world.state, me, &CONTEXT, passkey, &providers, None)
            .await
            .unwrap(),
        Removed::RequiredByPolicy
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"RevokedAt\" IS NULL"),
        1
    );
}

#[tokio::test]
async fn recovery_codes_and_remembered_devices_are_changed_only_by_a_session_that_may() {
    let world = World::new();
    let user = world.user(None);
    world.with_app(user).await;
    let stale = caller(&world, user, Some(3600));
    let fresh = caller(&world, user, Some(30));

    assert_eq!(
        regenerate_recovery_codes(&world.state, stale, &CONTEXT, None)
            .await
            .unwrap(),
        Changed::NotAllowed
    );
    let Changed::Done(codes) = regenerate_recovery_codes(&world.state, fresh, &CONTEXT, None)
        .await
        .unwrap()
    else {
        panic!("expected codes")
    };
    assert_eq!(codes.len(), 10);
    let none = world.user(None);
    let session = caller(&world, none, None);
    assert_eq!(
        regenerate_recovery_codes(&world.state, session, &CONTEXT, Some("Passw0rd!2026"))
            .await
            .unwrap(),
        Changed::NotFound
    );

    let device = {
        let state = world.state.clone();
        world
            .state
            .database()
            .write(move |tx| {
                trusted_devices::issue(
                    tx,
                    &CONTEXT,
                    state.messages(),
                    user,
                    &Default::default(),
                    Timestamp::now(),
                )
            })
            .await
            .unwrap()
            .id
    };
    assert!(
        !forget_trusted_device(&world.state, session, &CONTEXT, device)
            .await
            .unwrap()
    );
    assert!(
        forget_trusted_device(&world.state, fresh, &CONTEXT, device)
            .await
            .unwrap()
    );
    assert!(
        !forget_trusted_device(&world.state, fresh, &CONTEXT, device)
            .await
            .unwrap()
    );
}

// ---- an operator's recovery ----

#[tokio::test]
async fn a_recovered_account_redeems_its_credential_once_and_sets_a_new_step_up() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    world.with_app(user).await;
    let credential = world
        .state
        .database()
        .write(move |tx| {
            recovery::recover(
                tx,
                user,
                "lost phone",
                &Default::default(),
                Timestamp::now(),
                |_| Ok(()),
            )
        })
        .await
        .unwrap()
        .unwrap()
        .credential;

    assert!(matches!(
        redeem_recovery_credential(&world.state, &CONTEXT, "nonsense-credential-x", true)
            .await
            .unwrap(),
        Redeemed::Invalid
    ));
    let Redeemed::Challenge { body, cookie } =
        redeem_recovery_credential(&world.state, &CONTEXT, &credential, true)
            .await
            .unwrap()
    else {
        panic!("a challenge was expected");
    };
    assert!(matches!(
        redeem_recovery_credential(&world.state, &CONTEXT, &credential, true)
            .await
            .unwrap(),
        Redeemed::Invalid
    ));
    let id = serde_json::to_value(&*body).unwrap()["challengeId"]
        .as_str()
        .unwrap()
        .to_string();
    let preauth = security_cookies::parse_bound(
        cookie
            .strip_prefix("stellarr-dev-preauth=")
            .unwrap()
            .split(';')
            .next()
            .unwrap(),
    )
    .unwrap();
    let purpose: String = world
        .db()
        .query_row("SELECT \"Purpose\" FROM \"AuthChallenges\"", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(purpose, PURPOSE_RECOVERY);
    assert_eq!(world.count("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Detail\" = 'recovery credential redeemed'"), 1);
    let ask = Ask {
        context: &CONTEXT,
        preauth: Some(&preauth),
        challenge_id: &id,
    };
    let TotpStarted::Started { method, secret, .. } =
        start_totp(&world.state, &ask, None).await.unwrap()
    else {
        panic!("expected a method")
    };

    let done = confirm_totp(
        &world.state,
        &ask,
        TotpConfirmation {
            method: Some(&method.to_string()),
            name: None,
            code: &now_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();

    assert!(matches!(done, Enrolled::Done { .. }));
    assert_eq!(
        world.count("SELECT \"SecurityRecoveryRequired\" FROM \"Users\""),
        0
    );
}

#[tokio::test]
async fn a_change_of_password_ends_other_sessions_and_remembered_devices() {
    let world = World::new();
    let user = world.user(None);
    let mine = caller(&world, user, None);
    let other = add_session(&world.db(), user, Timestamp::now(), None);
    {
        let state = world.state.clone();
        world
            .state
            .database()
            .write(move |tx| {
                trusted_devices::issue(
                    tx,
                    &CONTEXT,
                    state.messages(),
                    user,
                    &Default::default(),
                    Timestamp::now(),
                )
                .map(|_| ())
            })
            .await
            .unwrap();
    }

    world.handoffs(user);

    let version = world
        .state
        .database()
        .write(move |tx| {
            recovery::credentials_changed(tx, user, Some(mine.session), Timestamp::now())
        })
        .await
        .map_err(|e: DbError| e)
        .unwrap();

    assert_eq!(version, 2);
    // The handoff that was approved is spent; the one nobody approved is left.
    assert_eq!(
        world.count(
            "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NULL"),
        1
    );
    let revoked: bool = world
        .db()
        .query_row(
            "SELECT \"RevokedAt\" IS NOT NULL FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [other],
            |r| r.get(0),
        )
        .unwrap();
    assert!(revoked);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"),
        0
    );
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL AND \"SecurityVersion\" = 2"), 1);
}

#[tokio::test]
async fn a_reset_by_mail_has_no_session_to_keep_and_ends_them_all() {
    let world = World::new();
    let user = world.user(None);
    caller(&world, user, None);
    caller(&world, user, None);
    {
        let state = world.state.clone();
        world
            .state
            .database()
            .write(move |tx| {
                trusted_devices::issue(
                    tx,
                    &CONTEXT,
                    state.messages(),
                    user,
                    &Default::default(),
                    Timestamp::now(),
                )
                .map(|_| ())
            })
            .await
            .unwrap();
    }
    world.handoffs(user);

    let version = world
        .state
        .database()
        .write(move |tx| recovery::credentials_changed(tx, user, None, Timestamp::now()))
        .await
        .map_err(|e: DbError| e)
        .unwrap();

    assert_eq!(version, 2);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL"),
        0
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"),
        0
    );
    assert_eq!(
        world.count(
            "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NOT NULL"
        ),
        1
    );
}

fn with_password(world: &World, user: Id, password: &str) {
    world
        .db()
        .execute(
            "UPDATE \"Users\" SET \"PasswordHash\" = ?2 WHERE \"Id\" = ?1",
            params![user, super::password::hash(password)],
        )
        .unwrap();
}

#[tokio::test]
async fn a_password_is_hashed_beforehand_and_only_when_it_could_decide() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    let fresh_session = caller(&world, user, None);
    // Signed in a moment ago: the password is not needed, so it is not hashed.
    let unneeded = check_password(&world.state, fresh_session, Some("correct horse 42"))
        .await
        .unwrap();
    assert!(unneeded.hash.is_none() && !unneeded.proves);

    let old = add_session(
        &world.db(),
        user,
        Timestamp::now().minus(Duration::from_secs(3600)),
        None,
    );
    let old = Caller { user, session: old };
    let right = check_password(&world.state, old, Some("correct horse 42"))
        .await
        .unwrap();
    assert!(right.hash.is_some() && right.proves);
    let wrong = check_password(&world.state, old, Some("another one"))
        .await
        .unwrap();
    assert!(wrong.hash.is_some() && !wrong.proves);
    // An account with a second step is never decided by its password.
    world.with_app(user).await;
    let decided_by_step = check_password(&world.state, old, Some("correct horse 42"))
        .await
        .unwrap();
    assert!(!decided_by_step.proves);
}

#[tokio::test]
async fn a_password_checked_against_a_hash_that_has_since_changed_counts_for_nothing() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    let session = add_session(
        &world.db(),
        user,
        Timestamp::now().minus(Duration::from_secs(3600)),
        None,
    );
    let old = Caller { user, session };
    let checked = check_password(&world.state, old, Some("correct horse 42"))
        .await
        .unwrap();
    assert!(checked.proves);
    let stands = |world: &World, checked: PasswordCheck| {
        let mut connection = world.db();
        let tx = connection.transaction().unwrap();
        may_change_security(&tx, old, &checked, Timestamp::now()).unwrap()
    };

    assert!(stands(
        &world,
        PasswordCheck {
            hash: checked.hash.clone(),
            proves: true
        }
    ));
    // The password was changed after it was checked.
    with_password(&world, user, "a newer password 77");
    assert!(!stands(
        &world,
        PasswordCheck {
            hash: checked.hash,
            proves: true
        }
    ));
}

/// What the given password comes to, in a transaction of its own.
async fn given(world: &World, caller: Caller, candidate: Option<&str>) -> PasswordProof {
    let checked = check_given_password(&world.state, caller.user, candidate)
        .await
        .unwrap();
    judge(world, caller, &checked)
}

fn judge(world: &World, caller: Caller, checked: &GivenPassword) -> PasswordProof {
    let mut connection = world.db();
    let tx = connection.transaction().unwrap();
    judge_given_password(&tx, caller, checked, Timestamp::now()).unwrap()
}

#[tokio::test]
async fn a_given_password_is_the_proof_whatever_second_step_the_account_has() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    world.with_app(user).await;
    // A session that never passed the second step: the route's own guard is
    // what asks for that, the password is judged on its own.
    let old = add_session(
        &world.db(),
        user,
        Timestamp::now().minus(Duration::from_secs(3600)),
        None,
    );
    let old = Caller { user, session: old };

    assert_eq!(
        given(&world, old, Some("correct horse 42")).await,
        PasswordProof::Given
    );
    assert_eq!(
        given(&world, old, Some("another one")).await,
        PasswordProof::WrongPassword
    );
    assert_eq!(
        given(&world, old, Some("")).await,
        PasswordProof::WrongPassword
    );
    assert_eq!(given(&world, old, None).await, PasswordProof::WrongPassword);
}

#[tokio::test]
async fn a_password_given_before_it_was_changed_counts_for_nothing() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    let session = caller(&world, user, None);
    let checked = check_given_password(&world.state, user, Some("correct horse 42"))
        .await
        .unwrap();
    assert_eq!(judge(&world, session, &checked), PasswordProof::Given);

    with_password(&world, user, "a newer password 77");

    assert_eq!(
        judge(&world, session, &checked),
        PasswordProof::WrongPassword
    );
}

#[tokio::test]
async fn an_account_without_a_password_is_asked_for_a_sign_in_from_the_last_minutes() {
    let world = World::new();
    let user = world.user(None);
    world
        .db()
        .execute(
            "UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1",
            [user],
        )
        .unwrap();
    let recent = caller(&world, user, None);
    let old = Caller {
        user,
        session: add_session(
            &world.db(),
            user,
            Timestamp::now().minus(Duration::from_secs(3600)),
            None,
        ),
    };

    // Giving a password changes nothing for an account that has none.
    assert_eq!(
        given(&world, recent, Some("whatever")).await,
        PasswordProof::Given
    );
    assert_eq!(
        given(&world, old, Some("whatever")).await,
        PasswordProof::FreshProofNeeded
    );
    assert_eq!(
        given(&world, old, None).await,
        PasswordProof::FreshProofNeeded
    );

    // With a second step it is the step, as for any other change of security.
    world.with_app(user).await;
    assert_eq!(
        given(&world, recent, None).await,
        PasswordProof::FreshProofNeeded
    );
}

#[tokio::test]
async fn an_enrolment_from_a_session_accepts_the_password_of_the_account_without_a_second_step() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    let session = add_session(
        &world.db(),
        user,
        Timestamp::now().minus(Duration::from_secs(3600)),
        None,
    );
    let old = Caller { user, session };

    let refused = start_from_session(
        &world.state,
        old,
        &CONTEXT,
        FromSession::Enrollment,
        Some("not it"),
    )
    .await
    .unwrap();
    assert!(matches!(refused, Started::NotAllowed));
    let allowed = start_from_session(
        &world.state,
        old,
        &CONTEXT,
        FromSession::Enrollment,
        Some("correct horse 42"),
    )
    .await
    .unwrap();
    assert!(matches!(allowed, Started::Challenge { .. }));
}

#[tokio::test]
async fn a_challenge_asked_from_a_session_dies_with_that_session() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let me = caller(&world, user, Some(3600));
    let Started::Challenge { body, cookie } = start_from_session(
        &world.state,
        me,
        &CONTEXT,
        FromSession::Reverification,
        None,
    )
    .await
    .unwrap() else {
        panic!("a challenge was expected");
    };
    let id = serde_json::to_value(&*body).unwrap()["challengeId"]
        .as_str()
        .unwrap()
        .to_string();
    let preauth = security_cookies::parse_bound(
        cookie
            .strip_prefix("stellarr-dev-preauth=")
            .unwrap()
            .split(';')
            .next()
            .unwrap(),
    )
    .unwrap();
    // The session it was asked from is signed out before the answer comes.
    world
        .db()
        .execute(
            "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
            params![me.session, Timestamp::now()],
        )
        .unwrap();

    let answered = answer(
        &world.state,
        &Ask {
            context: &CONTEXT,
            preauth: Some(&preauth),
            challenge_id: &id,
        },
        Answer::Code {
            method: Some(&method.to_string()),
            code: &now_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();

    assert!(matches!(answered, Answered::Invalid(_)));
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL"),
        0
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"AuthChallenges\" WHERE \"ConsumedAt\" IS NULL"),
        1
    );
}

#[tokio::test]
async fn a_pending_challenge_is_described_to_its_browser_and_given_up_by_it_alone() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();

    let Pending::Challenge(body) = pending(&world.state, &ask(&challenge, &preauth, &id))
        .await
        .unwrap()
    else {
        panic!("a description was expected");
    };
    let json = serde_json::to_value(&*body).unwrap();
    assert_eq!(json["status"], "mfa_required");
    assert_eq!(json["challengeId"], id);
    assert_eq!(json["methods"].as_array().unwrap().len(), 1);

    // Another browser's secret neither sees it nor gives it up.
    let stranger = (challenge.id, "not the secret".to_string());
    assert!(matches!(
        pending(&world.state, &ask(&challenge, &stranger, &id))
            .await
            .unwrap(),
        Pending::Invalid(_)
    ));
    cancel(&world.state, &ask(&challenge, &stranger, &id))
        .await
        .unwrap();
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"AuthChallenges\" WHERE \"ConsumedAt\" IS NULL"),
        1
    );

    cancel(&world.state, &ask(&challenge, &preauth, &id))
        .await
        .unwrap();
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"AuthChallenges\" WHERE \"ConsumedAt\" IS NULL"),
        0
    );
    assert!(matches!(
        pending(&world.state, &ask(&challenge, &preauth, &id))
            .await
            .unwrap(),
        Pending::Invalid(_)
    ));
}

#[tokio::test]
async fn an_id_is_read_in_its_hyphenated_spelling_only() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    // Both ids in capitals: the letter case is free.
    let spelled = challenge.id.to_string().to_uppercase();
    let method_spelled = method.to_string().to_uppercase();

    let answered = answer(
        &world.state,
        &Ask {
            context: &CONTEXT,
            preauth: Some(&preauth),
            challenge_id: &spelled,
        },
        Answer::Code {
            method: Some(&method_spelled),
            code: &now_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();

    let Answered::SignedIn { completed, .. } = answered else {
        panic!("a sign-in was expected")
    };
    assert_eq!(completed.evidence, "totp");
    // A text that is no id names no method: the code is taken for a recovery
    // code. So is the id of a method in any other spelling.
    let plain = method.to_string().replace('-', "");
    let braced = format!("{{{method}}}");
    let spaced = format!(" {method}");
    for (round, text) in ["not an id", &plain, &braced, &spaced].into_iter().enumerate() {
        let (second, second_preauth) = world.challenge(user, PURPOSE_MFA);
        let second_id = second.id.to_string();
        let refused = answer(
            &world.state,
            &Ask {
                context: &CONTEXT,
                preauth: Some(&second_preauth),
                challenge_id: &second_id,
            },
            Answer::Code {
                method: Some(text),
                code: &now_code(&secret),
            },
            false,
            nothing,
        )
        .await
        .unwrap();
        assert!(matches!(refused, Answered::Refused), "{text:?}");
        assert_eq!(
            world.count("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Detail\" = 'recovery'"),
            round as i64 + 1,
            "{text:?}"
        );
    }
    // A challenge named without its hyphens is no challenge.
    let (third, third_preauth) = world.challenge(user, PURPOSE_MFA);
    let third_id = third.id.to_string().replace('-', "");
    let missing = answer(
        &world.state,
        &Ask {
            context: &CONTEXT,
            preauth: Some(&third_preauth),
            challenge_id: &third_id,
        },
        Answer::Code {
            method: Some(&method.to_string()),
            code: &now_code(&secret),
        },
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(!matches!(missing, Answered::SignedIn { .. }));
}

#[tokio::test]
async fn a_code_is_refused_for_the_challenge_then_the_method_then_the_mail() {
    let world = World::new();
    // Mail is off.
    found(
        &world.state,
        Saved {
            mailing: false,
            ..Saved::default()
        },
    )
    .await;
    let user = world.user(Some("  Maya@Example.test "));
    let method = add_method(&world.db(), user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();
    let ask_for = |text: Option<String>| {
        let (world, challenge, preauth, id) = (&world, &challenge, &preauth, &id);
        async move {
            send_login_code(&world.state, &ask(challenge, preauth, id), text.as_deref())
                .await
                .unwrap()
        }
    };

    // The challenge first: another purpose is invalid whatever else is so.
    let (enrolling, enrolling_preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let other = enrolling.id.to_string();
    let wrong = send_login_code(
        &world.state,
        &ask(&enrolling, &enrolling_preauth, &other),
        Some("nothing"),
    )
    .await
    .unwrap();
    assert!(matches!(wrong, Sent::Invalid(_)));
    // Then the method: one that is not there is that, mail off or not.
    assert_eq!(
        ask_for(Some(Id::random().to_string())).await,
        Sent::MethodUnavailable
    );
    assert_eq!(ask_for(None).await, Sent::MethodUnavailable);
    assert_eq!(
        ask_for(Some("not an id".into())).await,
        Sent::MethodUnavailable
    );
    // Only then mail.
    assert_eq!(
        ask_for(Some(method.to_string())).await,
        Sent::MailUnavailable
    );
    assert_eq!(world.count("SELECT COUNT(*) FROM \"EmailOtpCodes\""), 0);
}

#[tokio::test]
async fn an_enrolment_code_is_refused_for_the_method_before_the_mail() {
    let world = World::new();
    found(
        &world.state,
        Saved {
            mailing: false,
            ..Saved::default()
        },
    )
    .await;
    let without = world.user(None);
    let with = world.user(Some("maya@example.test"));
    let (first, first_preauth) = world.challenge(without, PURPOSE_ENROLLMENT);
    let (second, second_preauth) = world.challenge(with, PURPOSE_ENROLLMENT);
    let (first_id, second_id) = (first.id.to_string(), second.id.to_string());

    let no_address = send_enrollment_code(&world.state, &ask(&first, &first_preauth, &first_id))
        .await
        .unwrap();
    let mail_off = send_enrollment_code(&world.state, &ask(&second, &second_preauth, &second_id))
        .await
        .unwrap();

    assert_eq!(no_address, Sent::MethodUnavailable);
    assert_eq!(mail_off, Sent::MailUnavailable);
    // An account that has the step already is told the method is not available.
    add_method(&world.db(), with, EMAIL, Some("MAYA@EXAMPLE.TEST"));
    let again = send_enrollment_code(&world.state, &ask(&second, &second_preauth, &second_id))
        .await
        .unwrap();
    assert_eq!(again, Sent::MethodUnavailable);
}

#[tokio::test]
async fn the_address_a_code_was_mailed_to_is_the_one_in_the_account_trimmed() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let world = World::new();
    found(
        &world.state,
        Saved {
            port: smtp.port,
            ..Saved::default()
        },
    )
    .await;
    let user = world.user(Some("  Maya@Example.test "));
    let method = add_method(&world.db(), user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();

    let sent = send_login_code(
        &world.state,
        &ask(&challenge, &preauth, &id),
        Some(&method.to_string()),
    )
    .await
    .unwrap();

    assert_eq!(
        sent,
        Sent::Delivered {
            address: "Maya@Example.test".into(),
            retry_after_seconds: 45
        }
    );
    assert!(
        smtp.mails()[0]
            .commands
            .iter()
            .any(|c| c.to_lowercase().contains("maya@example.test"))
    );
    // The address in the answer is the one in the mail, even if the account's changes after.
    world
        .db()
        .execute("UPDATE \"Users\" SET \"Email\" = 'new@example.test'", [])
        .unwrap();
}

#[tokio::test]
async fn an_instance_without_passkeys_says_so_only_after_the_challenge_and_the_name_were_looked_at()
{
    let world = World::new();
    let user = world.user(None);
    let (signing, signing_preauth) = world.challenge(user, PURPOSE_MFA);
    let (enrolling, enrolling_preauth) = world.challenge(user, PURPOSE_ENROLLMENT);
    let (signing_id, enrolling_id) = (signing.id.to_string(), enrolling.id.to_string());
    let for_signing = ask(&signing, &signing_preauth, &signing_id);
    let for_enrolling = ask(&enrolling, &enrolling_preauth, &enrolling_id);
    let none: Option<&stellarr_webauthn::RelyingParty> = None;

    // Options for a second step: the challenge for it is looked at, then passkeys.
    assert_eq!(
        passkey_options(&world.state, &for_signing, none)
            .await
            .unwrap(),
        PasskeyOptions::PasskeysUnavailable
    );
    assert!(matches!(
        passkey_options(&world.state, &for_enrolling, none)
            .await
            .unwrap(),
        PasskeyOptions::Invalid(_)
    ));
    // Options for adding one.
    assert_eq!(
        start_passkey(&world.state, &for_enrolling, none)
            .await
            .unwrap(),
        PasskeyStarted::PasskeysUnavailable
    );
    assert!(matches!(
        start_passkey(&world.state, &for_signing, none)
            .await
            .unwrap(),
        PasskeyStarted::Invalid(_)
    ));
    // The answer to either.
    let answered = answer(
        &world.state,
        &for_signing,
        Answer::PasskeysUnavailable,
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(answered, Answered::PasskeysUnavailable));
    let answered = answer(
        &world.state,
        &for_enrolling,
        Answer::PasskeysUnavailable,
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(answered, Answered::Invalid(_)));
    // Adding one: the challenge, then the name, then passkeys.
    let too_long = "n".repeat(65);
    let invalid_name = confirm_passkey(
        &world.state,
        &for_enrolling,
        none,
        Some(&too_long),
        None,
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(invalid_name, Enrolled::InvalidName));
    let unavailable = confirm_passkey(
        &world.state,
        &for_enrolling,
        none,
        Some("Key"),
        None,
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(unavailable, Enrolled::PasskeysUnavailable));
    let wrong_challenge = confirm_passkey(
        &world.state,
        &for_signing,
        none,
        Some("Key"),
        None,
        false,
        nothing,
    )
    .await
    .unwrap();
    assert!(matches!(wrong_challenge, Enrolled::Invalid(_)));
    // Nothing of it cost an attempt.
    assert_eq!(
        world.count("SELECT SUM(\"Attempts\") FROM \"AuthChallenges\""),
        0
    );
}

#[tokio::test]
async fn a_change_of_address_revokes_the_old_mail_methods_and_says_so_without_alerting() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    let other = world.user(Some("lee@example.test"));
    let old = add_method(&world.db(), user, EMAIL, Some("OLD@EXAMPLE.TEST"));
    let pending = add_pending_method(&world.db(), user, EMAIL);
    world
        .db()
        .execute(
            "UPDATE \"MfaMethods\" SET \"EmailAddress\" = 'OLDER@EXAMPLE.TEST' WHERE \"Id\" = ?1",
            [pending],
        )
        .unwrap();
    let kept = add_method(&world.db(), user, EMAIL, Some("NEW@EXAMPLE.TEST"));
    let app = add_method(&world.db(), user, "totp", None);
    let theirs = add_method(&world.db(), other, EMAIL, Some("OLD@EXAMPLE.TEST"));
    let revoked_already = add_method(&world.db(), user, EMAIL, Some("GONE@EXAMPLE.TEST"));
    world
        .db()
        .execute(
            "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
            params![revoked_already, Timestamp::now()],
        )
        .unwrap();
    // An address the provider vouched for, so that an alert would be possible.
    world
        .db()
        .execute(
            "UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"",
            [],
        )
        .unwrap();
    let (challenge, _) = world.challenge(user, PURPOSE_MFA);
    let now = Timestamp::now();
    world
        .db()
        .execute(
            "INSERT INTO \"EmailOtpCodes\" VALUES (?1, ?2, ?3, ?4, 'login', 'a', 'x', 0, 5, ?5, ?5, NULL, ?5)",
            params![Id::random(), user, old, challenge.id, now],
        )
        .unwrap();
    let mut connection = world.db();
    let tx = connection.transaction().unwrap();

    let revoked = email_address_changed(&tx, &CONTEXT, user, "NEW@EXAMPLE.TEST", now).unwrap();
    tx.commit().unwrap();

    assert_eq!(revoked, vec![old, pending]);
    let live = |method: Id| -> bool {
        world
            .db()
            .query_row(
                "SELECT \"RevokedAt\" IS NULL FROM \"MfaMethods\" WHERE \"Id\" = ?1",
                [method],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert!(!live(old) && !live(pending));
    assert!(live(kept) && live(app) && live(theirs));
    assert_eq!(
        events_of(&world.db(), user, "method-removed"),
        vec!["email changed", "email changed"]
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"SecurityNotifications\""),
        0
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"ConsumedAt\" IS NULL"),
        0
    );
    // Nothing about sessions, devices or the account's version.
    assert_eq!(world.count("SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = (SELECT \"UserId\" FROM \"MfaMethods\" WHERE \"Id\" = (SELECT \"Id\" FROM \"MfaMethods\" LIMIT 1))"), 1);
    // A second call finds nothing more to do.
    let mut connection = world.db();
    let tx = connection.transaction().unwrap();
    assert!(
        email_address_changed(&tx, &CONTEXT, user, "NEW@EXAMPLE.TEST", now)
            .unwrap()
            .is_empty()
    );
}

// ---- the desktop application's sign-in through the browser ----

const DESKTOP_SECRET: &str = "a-secret-of-the-desktop-0123456789ABCDEFGHI";

impl World {
    /// A request the application made, and, if `approver` is given, approved
    /// by a session of that account as the browser's was when it did.
    fn desktop_request(&self, approver: Option<(Id, Id, Option<u64>)>) -> Id {
        let id = Id::random();
        let now = Timestamp::now();
        let db = self.db();
        db.execute(
            "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, 1, ?3, ?4)",
            params![id, super::tokens::hash(DESKTOP_SECRET), now, now.plus(Duration::from_secs(600))],
        )
        .unwrap();
        if let Some((user, source, passed)) = approver {
            let completed = passed.map(|s| now.minus(Duration::from_secs(s)));
            db.execute(
                "UPDATE \"DesktopBrowserLogins\" SET \"UserId\" = ?2, \"ApprovedSessionId\" = ?3, \"PrimarySource\" = 'oidc',
                    \"MfaCompletedAt\" = ?4, \"MfaEvidence\" = ?5, \"ApprovedAt\" = ?6 WHERE \"Id\" = ?1",
                params![id, user, source, completed, completed.map(|_| "totp"), now],
            )
            .unwrap();
        }
        id
    }
}

async fn exchange(world: &World, id: Id, secret: &str) -> Exchanged {
    exchange_browser_login(&world.state, &CONTEXT, &id.to_string(), secret)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_request_that_is_not_approved_yet_is_pending_and_costs_no_write() {
    let world = World::new();
    let id = world.desktop_request(None);

    assert!(matches!(
        exchange(&world, id, DESKTOP_SECRET).await,
        Exchanged::Pending
    ));

    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 0);
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NULL"),
        1
    );
}

#[tokio::test]
async fn an_unknown_wrong_short_long_expired_or_used_request_is_one_answer() {
    let world = World::new();
    let user = world.user(None);
    let source = caller(&world, user, None).session;
    let id = world.desktop_request(Some((user, source, None)));
    let expired = world.desktop_request(None);
    world
        .db()
        .execute(
            "UPDATE \"DesktopBrowserLogins\" SET \"ExpiresAt\" = ?2 WHERE \"Id\" = ?1",
            params![expired, Timestamp::now()],
        )
        .unwrap();
    let used = world.desktop_request(Some((user, source, None)));
    world
        .db()
        .execute(
            "UPDATE \"DesktopBrowserLogins\" SET \"RedeemedAt\" = ?2 WHERE \"Id\" = ?1",
            params![used, Timestamp::now()],
        )
        .unwrap();

    for (what, outcome) in [
        (
            "unknown id",
            exchange(&world, Id::random(), DESKTOP_SECRET).await,
        ),
        (
            "wrong secret",
            exchange(&world, id, "a-different-secret-of-the-same-sort-12345").await,
        ),
        (
            "short secret",
            exchange(&world, id, "short-secret-15ch").await,
        ),
        ("empty secret", exchange(&world, id, "").await),
        ("long secret", exchange(&world, id, &"x".repeat(513)).await),
        ("expired", exchange(&world, expired, DESKTOP_SECRET).await),
        ("used", exchange(&world, used, DESKTOP_SECRET).await),
        (
            "no id",
            exchange_browser_login(&world.state, &CONTEXT, "nonsense", DESKTOP_SECRET)
                .await
                .unwrap(),
        ),
    ] {
        assert!(matches!(outcome, Exchanged::Invalid), "{what}");
    }
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 1);
}

#[tokio::test]
async fn an_approved_request_gives_a_session_with_what_the_approving_session_had() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    let source = caller(&world, user, Some(120)).session;
    let authenticated: Timestamp = world
        .db()
        .query_row(
            "SELECT \"AuthenticatedAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [source],
            |r| r.get(0),
        )
        .unwrap();
    let id = world.desktop_request(Some((user, source, Some(120))));

    let Exchanged::SignedIn(signed) = exchange(&world, id, DESKTOP_SECRET).await else {
        panic!("a session was expected")
    };

    assert_eq!(signed.user, user);
    assert!(signed.cookie.starts_with("stellarr-dev-session="));
    let (primary, evidence, completed, auth, short, version): (String, Option<String>, Option<Timestamp>, Timestamp, bool, i64) = world
        .db()
        .query_row(
            "SELECT \"PrimarySource\", \"MfaEvidence\", \"MfaCompletedAt\", \"AuthenticatedAt\", \"ShortLived\", \"SecurityVersion\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [signed.session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        (primary.as_str(), evidence.as_deref(), short, version),
        ("oidc", Some("totp"), false, 1)
    );
    assert!(completed.is_some());
    assert_eq!(auth, authenticated);
    assert_eq!(
        world.count(
            "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        events_of(&world.db(), user, "login-succeeded"),
        vec!["desktop"]
    );
    // Used once: the same secret opens nothing more.
    assert!(matches!(
        exchange(&world, id, DESKTOP_SECRET).await,
        Exchanged::Invalid
    ));
}

#[tokio::test]
async fn two_exchanges_of_one_request_at_the_same_moment_make_one_session() {
    let world = World::new();
    let user = world.user(None);
    let source = caller(&world, user, None).session;
    let id = world.desktop_request(Some((user, source, None)));

    let (a, b) = tokio::join!(
        exchange(&world, id, DESKTOP_SECRET),
        exchange(&world, id, DESKTOP_SECRET)
    );

    let signed = [&a, &b]
        .iter()
        .filter(|o| matches!(o, Exchanged::SignedIn(_)))
        .count();
    let invalid = [&a, &b]
        .iter()
        .filter(|o| matches!(o, Exchanged::Invalid))
        .count();
    assert_eq!((signed, invalid), (1, 1));
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 2);
}

#[tokio::test]
async fn a_session_that_passed_no_second_step_gives_a_session_that_counts_as_none() {
    let world = World::new();
    let user = world.user(None);
    let source = caller(&world, user, None).session;
    let id = world.desktop_request(Some((user, source, None)));

    let Exchanged::SignedIn(signed) = exchange(&world, id, DESKTOP_SECRET).await else {
        panic!("a session was expected")
    };

    let (completed, evidence): (Option<Timestamp>, Option<String>) = world
        .db()
        .query_row(
            "SELECT \"MfaCompletedAt\", \"MfaEvidence\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [signed.session],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((completed, evidence), (None, None));
}

#[tokio::test]
async fn an_approval_that_no_longer_stands_gives_nothing() {
    let world = World::new();
    let user = world.user(None);
    let source = caller(&world, user, None).session;
    let row = |world: &World| world.desktop_request(Some((user, source, None)));

    // An account with a second step needs one the approval did not pass.
    world.with_app(user).await;
    let without = row(&world);
    assert!(matches!(
        exchange(&world, without, DESKTOP_SECRET).await,
        Exchanged::Forbidden
    ));
    world
        .db()
        .execute(
            "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?1",
            [Timestamp::now()],
        )
        .unwrap();
    // The approving session ended.
    let ended = row(&world);
    world
        .db()
        .execute(
            "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1",
            [Timestamp::now()],
        )
        .unwrap();
    assert!(matches!(
        exchange(&world, ended, DESKTOP_SECRET).await,
        Exchanged::Forbidden
    ));
    world
        .db()
        .execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = NULL", [])
        .unwrap();
    // The account changed since (a version the row does not carry).
    let stale = row(&world);
    world
        .db()
        .execute("UPDATE \"Users\" SET \"SessionVersion\" = 2", [])
        .unwrap();
    assert!(matches!(
        exchange(&world, stale, DESKTOP_SECRET).await,
        Exchanged::Forbidden
    ));
    world
        .db()
        .execute("UPDATE \"Users\" SET \"SessionVersion\" = 1", [])
        .unwrap();
    // Disabled, or waiting for recovery.
    for column in ["IsDisabled", "SecurityRecoveryRequired"] {
        let fresh = row(&world);
        world
            .db()
            .execute(&format!("UPDATE \"Users\" SET \"{column}\" = 1"), [])
            .unwrap();
        assert!(
            matches!(
                exchange(&world, fresh, DESKTOP_SECRET).await,
                Exchanged::Forbidden
            ),
            "{column}"
        );
        world
            .db()
            .execute(&format!("UPDATE \"Users\" SET \"{column}\" = 0"), [])
            .unwrap();
    }
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 1);
}

#[tokio::test]
async fn an_approved_request_whose_approvers_password_changed_gives_nothing() {
    let world = World::new();
    let user = world.user(None);
    let mine = caller(&world, user, None);
    let id = world.desktop_request(Some((user, mine.session, None)));
    world
        .state
        .database()
        .write(move |tx| {
            recovery::credentials_changed(tx, user, Some(mine.session), Timestamp::now())
        })
        .await
        .map_err(|e: DbError| e)
        .unwrap();

    assert!(matches!(
        exchange(&world, id, DESKTOP_SECRET).await,
        Exchanged::Invalid
    ));
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 1);
}

#[tokio::test]
async fn the_version_the_approval_carries_must_be_the_accounts_and_a_secret_must_be_of_a_length_that_is_read()
 {
    let world = World::new();
    let user = world.user(None);
    let source = caller(&world, user, None).session;
    // The session is at the account's version but the approval was not.
    let id = world.desktop_request(Some((user, source, None)));
    world
        .db()
        .execute(
            "UPDATE \"DesktopBrowserLogins\" SET \"SecurityVersion\" = 7",
            [],
        )
        .unwrap();
    assert!(matches!(
        exchange(&world, id, DESKTOP_SECRET).await,
        Exchanged::Forbidden
    ));
    // A secret that is not of a length that is read opens nothing, even if a
    // row holds its digest.
    for secret in ["fifteen-chars-x", &"y".repeat(513)] {
        let short = world.desktop_request(Some((user, source, None)));
        world
            .db()
            .execute(
                "UPDATE \"DesktopBrowserLogins\" SET \"SecretHash\" = ?2 WHERE \"Id\" = ?1",
                params![short, super::tokens::hash(secret)],
            )
            .unwrap();
        assert!(matches!(
            exchange(&world, short, secret).await,
            Exchanged::Invalid
        ));
    }
}

#[tokio::test]
async fn an_approval_from_a_session_at_another_version_than_the_accounts_gives_nothing() {
    let world = World::new();
    let user = world.user(None);
    let source = caller(&world, user, None).session;
    // The row carries the account's version; the session that approved it does
    // not, so it is not a session of the account as it stands.
    let id = world.desktop_request(Some((user, source, None)));
    world
        .db()
        .execute("UPDATE \"UserSessions\" SET \"SecurityVersion\" = 0", [])
        .unwrap();
    assert!(matches!(
        exchange(&world, id, DESKTOP_SECRET).await,
        Exchanged::Forbidden
    ));
    // Nothing was made, and the approval was not spent on it.
    assert_eq!(world.count("SELECT COUNT(*) FROM \"UserSessions\""), 1);
    assert_eq!(
        world.count(
            "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NOT NULL"
        ),
        0
    );
}

// ---- linked identities ----

fn link_identity(world: &World, user: Id, provider: Option<&str>) -> Id {
    let id = Id::random();
    world
        .db()
        .execute(
            "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\")
             VALUES (?1, ?2, 'https://idp.example.test', ?3, ?4, '2026-11-24 00:00:00')",
            params![id, user, id.to_string(), provider],
        )
        .unwrap();
    id
}

fn default_provider() -> super::ways_in::OwnedProviders {
    super::ways_in::OwnedProviders {
        configured: vec!["default".into()],
        default_id: "default".into(),
    }
}

#[tokio::test]
async fn a_linked_identity_is_taken_away_when_the_account_keeps_a_way_in() {
    let world = World::new();
    let user = world.user(Some("maya@example.test"));
    world
        .db()
        .execute(
            "UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"",
            [],
        )
        .unwrap();
    let identity = link_identity(&world, user, None);
    let other = world.user(None);
    let theirs = link_identity(&world, other, None);
    let me = caller(&world, user, None);
    let version_before = world.count("SELECT SUM(\"SessionVersion\") FROM \"Users\"");
    let another = add_session(&world.db(), user, Timestamp::now(), None);
    let providers = default_provider();

    // Not another account's identity.
    assert_eq!(
        remove_identity(&world.state, me, &CONTEXT, theirs, "Work", &providers, None)
            .await
            .unwrap(),
        IdentityRemoved::NotFound
    );
    assert_eq!(
        remove_identity(
            &world.state,
            me,
            &CONTEXT,
            Id::random(),
            "Work",
            &providers,
            None
        )
        .await
        .unwrap(),
        IdentityRemoved::NotFound
    );
    let removed = remove_identity(
        &world.state,
        me,
        &CONTEXT,
        identity,
        "Work",
        &providers,
        None,
    )
    .await
    .unwrap();

    assert_eq!(removed, IdentityRemoved::Done);
    assert_eq!(
        world.count(&format!(
            "SELECT COUNT(*) FROM \"ExternalIdentities\" WHERE \"Id\" = '{}'",
            identity.to_db_text()
        )),
        0
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"ExternalIdentities\""),
        1
    );
    assert_eq!(
        events_of(&world.db(), user, "identity-unlinked"),
        vec!["Work"]
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"SecurityNotifications\""),
        1
    );
    // As on the first server: no session ends and no version moves.
    assert_eq!(
        world.count("SELECT SUM(\"SessionVersion\") FROM \"Users\""),
        version_before
    );
    let revoked: bool = world
        .db()
        .query_row(
            "SELECT \"RevokedAt\" IS NOT NULL FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [another],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!revoked);
}

#[tokio::test]
async fn the_last_way_in_is_not_taken_away_and_the_guard_comes_first() {
    let world = World::new();
    let user = world.user(None);
    world
        .db()
        .execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL", [])
        .unwrap();
    let identity = link_identity(&world, user, None);
    world.with_app(user).await;
    let stale = caller(&world, user, Some(3600));
    let fresh = caller(&world, user, Some(30));
    let providers = default_provider();

    assert_eq!(
        remove_identity(
            &world.state,
            stale,
            &CONTEXT,
            identity,
            "Work",
            &providers,
            None
        )
        .await
        .unwrap(),
        IdentityRemoved::NotAllowed
    );
    assert_eq!(
        remove_identity(
            &world.state,
            fresh,
            &CONTEXT,
            identity,
            "Work",
            &providers,
            None
        )
        .await
        .unwrap(),
        IdentityRemoved::LastWayIn
    );
    // A provider that is no longer set up has no button to press.
    let none = super::ways_in::OwnedProviders {
        configured: vec![],
        default_id: "default".into(),
    };
    assert_eq!(
        remove_identity(&world.state, fresh, &CONTEXT, identity, "Work", &none, None)
            .await
            .unwrap(),
        IdentityRemoved::LastWayIn
    );
    assert_eq!(
        world.count("SELECT COUNT(*) FROM \"ExternalIdentities\""),
        1
    );
}

#[tokio::test]
async fn a_password_decides_for_an_account_without_a_second_step_and_only_if_unchanged() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    let identity = link_identity(&world, user, None);
    let session = add_session(
        &world.db(),
        user,
        Timestamp::now().minus(Duration::from_secs(3600)),
        None,
    );
    let old = Caller { user, session };
    let providers = default_provider();

    assert_eq!(
        may_start_identity_link(&world.state, old, None)
            .await
            .unwrap(),
        LinkStart::NotAllowed
    );
    assert_eq!(
        may_start_identity_link(&world.state, old, Some("not it"))
            .await
            .unwrap(),
        LinkStart::NotAllowed
    );
    assert_eq!(
        may_start_identity_link(&world.state, old, Some("correct horse 42"))
            .await
            .unwrap(),
        LinkStart::Allowed {
            security_version: 1
        }
    );
    assert_eq!(
        remove_identity(
            &world.state,
            old,
            &CONTEXT,
            identity,
            "Work",
            &providers,
            Some("not it")
        )
        .await
        .unwrap(),
        IdentityRemoved::NotAllowed
    );
    assert_eq!(
        remove_identity(
            &world.state,
            old,
            &CONTEXT,
            identity,
            "Work",
            &providers,
            Some("correct horse 42")
        )
        .await
        .unwrap(),
        IdentityRemoved::Done
    );
    assert_eq!(
        may_start_identity_link(
            &world.state,
            Caller {
                user: Id::random(),
                session
            },
            None
        )
        .await
        .unwrap(),
        LinkStart::NoAccount
    );
}

#[tokio::test]
async fn a_link_is_not_started_by_a_stale_session_of_an_account_with_a_second_step() {
    let world = World::new();
    let user = world.user(None);
    with_password(&world, user, "correct horse 42");
    world.with_app(user).await;
    let stale = caller(&world, user, Some(3600));
    let fresh = caller(&world, user, Some(30));

    // The password does not stand in for the second step.
    assert_eq!(
        may_start_identity_link(&world.state, stale, Some("correct horse 42"))
            .await
            .unwrap(),
        LinkStart::NotAllowed
    );
    assert_eq!(
        may_start_identity_link(&world.state, fresh, None)
            .await
            .unwrap(),
        LinkStart::Allowed {
            security_version: 1
        }
    );
}

#[tokio::test]
async fn a_providers_fresh_answer_moves_the_sign_in_forward_and_only_for_the_accounts_own_identity()
{
    let world = World::new();
    let user = world.user(None);
    let other = world.user(None);
    let identity = link_identity(&world, user, None);
    let (issuer, subject) = ("https://idp.example.test", identity.to_string());
    let old = Timestamp::now().minus(Duration::from_secs(3600));
    let session = add_session(&world.db(), user, old, None);
    let authenticated = |session: Id| -> Timestamp {
        world
            .db()
            .query_row(
                "SELECT \"AuthenticatedAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
                [session],
                |r| r.get(0),
            )
            .unwrap()
    };
    let now = Timestamp::now();
    let three_minutes = now.minus(Duration::from_secs(180));

    // Another account's session, or an identity the account does not have, or another issuer.
    let theirs = add_session(&world.db(), other, old, None);
    assert!(
        !reauthenticated_by_provider(&world.state, user, theirs, issuer, &subject, three_minutes)
            .await
            .unwrap()
    );
    assert!(
        !reauthenticated_by_provider(&world.state, other, theirs, issuer, &subject, three_minutes)
            .await
            .unwrap()
    );
    assert!(
        !reauthenticated_by_provider(
            &world.state,
            user,
            session,
            issuer,
            "someone-else",
            three_minutes
        )
        .await
        .unwrap()
    );
    assert!(
        !reauthenticated_by_provider(
            &world.state,
            user,
            session,
            "https://another.example.test",
            &subject,
            three_minutes
        )
        .await
        .unwrap()
    );
    // A sign-in at the provider that is not fresh proves nothing new.
    assert!(
        !reauthenticated_by_provider(
            &world.state,
            user,
            session,
            issuer,
            &subject,
            now.minus(Duration::from_secs(301))
        )
        .await
        .unwrap()
    );
    assert_eq!(authenticated(session), old);

    assert!(
        reauthenticated_by_provider(&world.state, user, session, issuer, &subject, three_minutes)
            .await
            .unwrap()
    );
    // The session counts as signed in when the provider says, not when it answered.
    assert_eq!(authenticated(session), three_minutes);
    // Never earlier than a sign-in it already counts.
    assert!(
        reauthenticated_by_provider(
            &world.state,
            user,
            session,
            issuer,
            &subject,
            now.minus(Duration::from_secs(250))
        )
        .await
        .unwrap()
    );
    assert_eq!(authenticated(session), three_minutes);
    // A provider's clock a little ahead counts as now.
    assert!(
        reauthenticated_by_provider(
            &world.state,
            user,
            session,
            issuer,
            &subject,
            now.plus(Duration::from_secs(600))
        )
        .await
        .unwrap()
    );
    assert!(authenticated(session) <= Timestamp::now() && authenticated(session) >= now);
}

#[tokio::test]
async fn a_providers_answer_refreshes_nothing_of_a_session_or_account_that_is_not_in_good_standing()
{
    let world = World::new();
    let user = world.user(None);
    let identity = link_identity(&world, user, None);
    let subject = identity.to_string();
    let old = Timestamp::now().minus(Duration::from_secs(3600));
    let session = add_session(&world.db(), user, old, None);
    let at = Timestamp::now().minus(Duration::from_secs(10));
    let attempt = |world: &World| {
        let state = world.state.clone();
        let subject = subject.clone();
        async move {
            reauthenticated_by_provider(
                &state,
                user,
                session,
                "https://idp.example.test",
                &subject,
                at,
            )
            .await
            .unwrap()
        }
    };

    for (statement, why) in [
        (
            "UPDATE \"UserSessions\" SET \"RevokedAt\" = '2026-11-24 00:00:00'",
            "revoked session",
        ),
        (
            "UPDATE \"UserSessions\" SET \"SecurityVersion\" = 0",
            "another security version",
        ),
        (
            "UPDATE \"Users\" SET \"IsDisabled\" = 1",
            "disabled account",
        ),
        (
            "UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1",
            "account waiting for recovery",
        ),
    ] {
        world.db().execute(statement, []).unwrap();
        assert!(!attempt(&world).await, "{why}");
        world
            .db()
            .execute(
                "UPDATE \"UserSessions\" SET \"RevokedAt\" = NULL, \"SecurityVersion\" = 1",
                [],
            )
            .unwrap();
        world
            .db()
            .execute(
                "UPDATE \"Users\" SET \"IsDisabled\" = 0, \"SecurityRecoveryRequired\" = 0",
                [],
            )
            .unwrap();
    }
    assert!(attempt(&world).await);
    // It does not make a session count as having passed a second step.
    let completed: Option<Timestamp> = world
        .db()
        .query_row(
            "SELECT \"MfaCompletedAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
            [session],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(completed, None);
}

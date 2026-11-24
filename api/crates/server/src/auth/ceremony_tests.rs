//! The ceremonies as a route calls them: nothing here reaches below the calls
//! of `auth::ceremony`, and each test looks at the rows afterwards.

use std::time::Duration;

use rusqlite::{Connection, params};
use stellarr_webauthn::Algorithm;

use super::ceremony::*;
use super::ceremony::{PasswordCheck, check_password, may_change_security};
use super::email_codes::{ENROLLMENT, LOGIN};
use super::email_send::Sent;
use super::fixtures::{add_challenge, add_method, add_session, add_user, preauth_of};
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
        Answer::Code { method, code },
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
            method: Some(pending.0),
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

    let answered = code_answer(
        &world,
        &challenge,
        &preauth,
        Some(method),
        &now_code(&secret),
        true,
    )
    .await;

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
    let replay = code_answer(
        &world,
        &again,
        &again_preauth,
        Some(method),
        &now_code(&secret),
        false,
    )
    .await;
    assert!(matches!(replay, Answered::Refused));
}

#[tokio::test]
async fn a_refused_link_leaves_the_proof_spent_and_the_person_signed_in() {
    let world = World::new();
    let user = world.user(None);
    let (method, secret) = world.with_app(user).await;
    let (challenge, preauth) = world.challenge(user, PURPOSE_MFA);
    let id = challenge.id.to_string();

    let answered = answer(
        &world.state,
        &ask(&challenge, &preauth, &id),
        Answer::Code {
            method: Some(method),
            code: &now_code(&secret),
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
        code_answer(
            &world,
            &again,
            &again_preauth,
            Some(method),
            &now_code(&secret),
            false
        )
        .await,
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
            method: Some(method),
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

    let sent = send_login_code(&world.state, &ask(&challenge, &preauth, &id), Some(method))
        .await
        .unwrap();

    assert_eq!(sent, Sent::Delivered);
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
        Some(method),
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
            method: Some(method),
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
            method: Some(method),
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
            method: Some(method),
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
            method: Some(method),
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
    assert_eq!(
        send_enrollment_code(&world.state, &ask(&challenge, &preauth, &id))
            .await
            .unwrap(),
        Sent::Delivered
    );
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

    let answered = answer(
        &world.state,
        &Ask {
            context: &CONTEXT,
            preauth: Some(&preauth),
            challenge_id: &id,
        },
        Answer::Code {
            method: Some(method),
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
            method: Some(method),
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

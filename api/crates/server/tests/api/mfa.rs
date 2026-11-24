//! The second half of a sign-in that has a second step: the challenge, an
//! authenticator app, a mailed code, a recovery code, a remembered browser,
//! setting one up when the instance asks, and the refusals of each.

use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::steps::{Instance, cookie_names, code, code_in, cookies, title};
use crate::support::{Client, PASSWORD, Request};

const WRONG: &str = "The security code is invalid or has expired.";
const NOT_VALID: &str = "This sign-in has expired. Sign in again.";

async fn post(client: &Client, path: &str, body: Value) -> crate::support::Response {
    client.post_json(path, &body).await
}

/// A sign-in of an account that has an app, waiting for it.
async fn waiting(instance: &Instance, name: &str) -> (Client, String) {
    let (client, challenge) = instance.start(name).await;
    assert_eq!(challenge["status"], "mfa_required");
    (client, challenge["challengeId"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn an_account_is_asked_to_set_a_second_step_up_and_is_signed_in_by_it() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();

    let (client, challenge) = instance.start("maya").await;
    assert_eq!(challenge["status"], "mfa_enrollment_required");
    assert_eq!(challenge["purpose"], "enrollment");
    let id = challenge["challengeId"].as_str().unwrap();
    // A reload resumes it, and says whose it is.
    let pending = client.get("/api/auth/mfa/pending").await;
    assert_eq!(pending.status.as_u16(), 200);
    assert_eq!(pending.json::<Value>()["account"]["username"], "maya");
    assert_eq!(pending.header("cache-control"), Some("no-store"));

    let too_long = post(&client, "/api/auth/mfa/enroll/totp/start", json!({"challengeId": id, "name": "x".repeat(65)})).await;
    assert_eq!(too_long.status.as_u16(), 400);
    assert_eq!(too_long.json::<Value>()["errors"]["name"][0], "A method name must be 1 to 64 characters long.");
    let started = post(&client, "/api/auth/mfa/enroll/totp/start", json!({"challengeId": id, "name": "Phone"})).await;
    assert_eq!(started.status.as_u16(), 200);
    let started = started.json::<Value>();
    assert_eq!(started["type"], "totp");
    let secret = started["secret"].as_str().unwrap();
    let method = started["methodId"].as_str().unwrap();
    assert!(started["otpAuthUri"].as_str().unwrap().starts_with("otpauth://totp/Stellarr%3Amaya?secret="));

    // A wrong first code spends an attempt and is written down; a name that
    // is too long spends nothing.
    let wrong = post(&client, "/api/auth/mfa/enroll/totp/confirm", json!({"challengeId": id, "methodId": method, "code": "000000"})).await;
    assert_eq!((wrong.status.as_u16(), title(&wrong).as_str()), (401, WRONG));
    let long_name = post(
        &client,
        "/api/auth/mfa/enroll/totp/confirm",
        json!({"challengeId": id, "methodId": method, "code": code(secret, 0), "name": "x".repeat(65)}),
    )
    .await;
    assert_eq!(long_name.status.as_u16(), 400);
    let unknown = post(&client, "/api/auth/mfa/enroll/totp/confirm", json!({"challengeId": id, "methodId": Id::random().to_string(), "code": code(secret, 0)})).await;
    assert_eq!(unknown.status.as_u16(), 409);

    let confirmed = post(
        &client,
        "/api/auth/mfa/enroll/totp/confirm",
        json!({"challengeId": id, "methodId": method, "code": code(secret, 0), "name": "Phone", "rememberDevice": true}),
    )
    .await;
    assert_eq!(confirmed.status.as_u16(), 200, "{}", confirmed.text());
    let body = confirmed.json::<Value>();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["user"]["username"], "maya");
    assert_eq!(body["recoveryCodes"].as_array().unwrap().len(), 10);
    assert!(body["trustedDeviceExpiresAt"].is_string());
    assert!(body["identityLink"].is_null());
    // The session, the remembered browser, and the challenge's cookie taken away.
    assert_eq!(
        cookie_names(&confirmed),
        ["stellarr-dev-session", "stellarr-dev-trust", "stellarr-dev-preauth"]
    );
    assert!(cookies(&confirmed)[2].contains("expires=Thu, 01 Jan 1970"));
    assert_eq!(confirmed.header("cache-control"), Some("no-cache,no-store"));
    assert_eq!(client.get("/api/session").await.json::<Value>()["user"]["username"], "maya");

    // The challenge is spent.
    assert_eq!(client.get("/api/auth/mfa/pending").await.status, 401);
    let again = post(&client, "/api/auth/mfa/enroll/totp/confirm", json!({"challengeId": id, "methodId": method, "code": code(secret, 1)})).await;
    assert_eq!((again.status.as_u16(), title(&again).as_str()), (401, NOT_VALID));

    assert_eq!(
        instance.events("maya"),
        [
            "mfa-failed totp-enrollment",
            "trusted-device-added Unknown device",
            "method-added totp",
            "mfa-succeeded totp"
        ]
    );
}

#[tokio::test]
async fn a_code_a_recovery_code_and_a_remembered_browser_are_proofs_of_their_own() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let app = instance.enrolled("maya", true).await;
    app.client.post_json("/api/auth/logout", &json!({})).await;

    // The remembered browser stands in for the code, and is never a fresh proof.
    let challenge = instance.server.login(&app.client, "maya", PASSWORD).await.json::<Value>();
    let id = challenge["challengeId"].as_str().unwrap().to_string();
    assert_eq!(challenge["trustedDeviceAvailable"], true);
    let client = &app.client;
    let answered = post(client, "/api/auth/mfa/verify", json!({"challengeId": id, "useTrustedDevice": true, "rememberDevice": true})).await;
    assert_eq!(answered.status.as_u16(), 200, "{}", answered.text());
    // Not remembered again by being used.
    assert_eq!(cookie_names(&answered), ["stellarr-dev-session", "stellarr-dev-preauth"]);
    assert!(answered.json::<Value>()["trustedDeviceExpiresAt"].is_null());
    let (evidence, fresh): (String, bool) = instance
        .database()
        .query_row(
            "SELECT \"MfaEvidence\", \"MfaCompletedAt\" IS NOT NULL FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC, rowid DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((evidence.as_str(), fresh), ("trusted_device", false));

    // A browser that is not remembered cannot use it.
    let (stranger, id) = waiting(&instance, "maya").await;
    let refused = post(&stranger, "/api/auth/mfa/verify", json!({"challengeId": id, "useTrustedDevice": true})).await;
    assert_eq!((refused.status.as_u16(), title(&refused).as_str()), (401, WRONG));

    // The app's code works once: its period is spent with it.
    let right = code(&app.secret, 1);
    let ok = post(&stranger, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": app.method, "code": right})).await;
    assert_eq!(ok.status.as_u16(), 200, "{}", ok.text());
    assert_eq!(cookie_names(&ok), ["stellarr-dev-session", "stellarr-dev-preauth"]);
    let (second, id) = waiting(&instance, "maya").await;
    let replay = post(&second, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": app.method, "code": right})).await;
    assert_eq!((replay.status.as_u16(), title(&replay).as_str()), (401, WRONG));

    // A recovery code, typed the way people type them, works once.
    let typed = app.codes[0].to_lowercase().replace('-', " ");
    let used = post(&second, "/api/auth/mfa/verify", json!({"challengeId": id, "code": typed})).await;
    assert_eq!(used.status.as_u16(), 200, "{}", used.text());
    let (third, id) = waiting(&instance, "maya").await;
    let twice = post(&third, "/api/auth/mfa/verify", json!({"challengeId": id, "code": app.codes[0]})).await;
    assert_eq!(twice.status.as_u16(), 401);
    assert!(instance.events("maya").contains(&"recovery-code-used recovery".to_string()));
}

#[tokio::test]
async fn nothing_is_answered_that_the_challenge_does_not_allow() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let app = instance.enrolled("maya", false).await;
    let method = app.method.clone();
    let (client, id) = waiting(&instance, "maya").await;

    // Not a body, not the right body, no code, a code of spaces.
    let nothing = client.send(Request::new(http::Method::POST, "/api/auth/mfa/verify")).await;
    assert_eq!(nothing.status.as_u16(), 400);
    assert_eq!(nothing.header("cache-control"), Some("no-store"));
    let malformed = client.send(Request::new(http::Method::POST, "/api/auth/mfa/verify").body("application/json", "{nope")).await;
    assert_eq!((malformed.status.as_u16(), malformed.header("cache-control")), (400, None));
    let other_type = client.send(Request::new(http::Method::POST, "/api/auth/mfa/verify").body("text/plain", "x")).await;
    assert_eq!(other_type.status.as_u16(), 415);
    for code in [json!(null), json!("   ")] {
        let empty = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": code})).await;
        assert_eq!(empty.status.as_u16(), 400);
        assert_eq!(empty.json::<Value>()["errors"]["code"][0], WRONG);
    }
    // A method that is not the account's, and one that is not an id (a
    // recovery code, then).
    for method in [Id::random().to_string(), "nope".to_string()] {
        let refused = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": code(&app.secret, 0)})).await;
        assert_eq!((refused.status.as_u16(), title(&refused).as_str()), (401, WRONG));
    }

    // Another browser's cookie, no cookie, a cookie with a wrong secret, an id
    // that is not one, or none: the same refusal.
    let (other, _) = waiting(&instance, "maya").await;
    let nobody = instance.server.client();
    let cookie = client.cookie("stellarr-dev-preauth").unwrap();
    let broken = instance.server.client().with_header("Cookie", &format!("stellarr-dev-preauth={cookie}x"));
    for (browser, challenge) in [(&other, id.clone()), (&nobody, id.clone()), (&broken, id.clone()), (&client, "nope".to_string()), (&client, String::new())] {
        let refused = post(browser, "/api/auth/mfa/verify", json!({"challengeId": challenge, "methodId": method, "code": code(&app.secret, 0)})).await;
        assert_eq!((refused.status.as_u16(), title(&refused).as_str()), (401, NOT_VALID));
    }
    // Written with no dashes it is the same challenge.
    let spelled = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id.replace('-', ""), "methodId": method, "code": "000000"})).await;
    assert_eq!((spelled.status.as_u16(), title(&spelled).as_str()), (401, WRONG));

    // A sign-in's challenge is not an enrolment's, nor the other way round.
    let wrong_purpose = post(&client, "/api/auth/mfa/enroll/totp/start", json!({"challengeId": id})).await;
    assert_eq!(wrong_purpose.status.as_u16(), 401);
    let no_passkey = post(&client, "/api/auth/mfa/passkey/options", json!({"challengeId": id})).await;
    assert_eq!(no_passkey.status.as_u16(), 409);

    // Cancelled, a challenge answers nothing, and cancelling is always allowed.
    let cancelled = client.post_json("/api/auth/mfa/cancel", &json!({})).await;
    assert_eq!(cancelled.status.as_u16(), 204);
    assert!(cookies(&cancelled)[0].contains("expires=Thu, 01 Jan 1970"));
    let after = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": code(&app.secret, 0)})).await;
    assert_eq!(after.status.as_u16(), 401);
    assert_eq!(nobody.post_json("/api/auth/mfa/cancel", &json!({})).await.status, 204);
    assert_eq!(nobody.get("/api/auth/mfa/pending").await.status, 401);
}

#[tokio::test]
async fn five_wrong_answers_close_a_challenge_and_ten_close_the_account_for_a_while() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let app = instance.enrolled("maya", false).await;
    let method = app.method.clone();

    let (client, id) = waiting(&instance, "maya").await;
    for _ in 0..5 {
        let wrong = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": "000000"})).await;
        assert_eq!(wrong.status.as_u16(), 401);
    }
    let late = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": code(&app.secret, 1)})).await;
    assert_eq!((late.status.as_u16(), title(&late).as_str()), (401, NOT_VALID));

    let (client, id) = waiting(&instance, "maya").await;
    for _ in 0..5 {
        post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": "000000"})).await;
    }
    // The account has spent its budget: neither the right code, nor a recovery
    // code, nor a remembered browser is looked at.
    let (client, id) = waiting(&instance, "maya").await;
    for body in [
        json!({"challengeId": id, "methodId": method, "code": code(&app.secret, 1)}),
        json!({"challengeId": id, "code": app.codes[0]}),
        json!({"challengeId": id, "useTrustedDevice": true}),
    ] {
        let spent = post(&client, "/api/auth/mfa/verify", body).await;
        assert_eq!(spent.status.as_u16(), 429, "{}", spent.text());
        assert_eq!(title(&spent), "Too many attempts. Sign in again.");
    }
    // The enrolment of a recovered account counts the same budget.
    let events = instance.events("maya");
    assert_eq!(events.iter().filter(|e| e.starts_with("mfa-failed")).count(), 10);
}

#[tokio::test]
async fn an_account_that_changes_between_the_two_steps_is_not_signed_in() {
    let instance = Instance::with(&["maya", "noor", "omar"], &[]).await;
    instance.require();
    let maya = instance.enrolled("maya", false).await;
    let noor = instance.enrolled("noor", false).await;
    let omar = instance.enrolled("omar", false).await;

    let (client, id) = waiting(&instance, "maya").await;
    instance.database().execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'maya'", []).unwrap();
    let disabled = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": maya.method, "code": code(&maya.secret, 1)})).await;
    assert_eq!((disabled.status.as_u16(), title(&disabled).as_str()), (401, NOT_VALID));

    // The password changed under it: the version moved.
    let (client, id) = waiting(&instance, "noor").await;
    instance.database().execute("UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1 WHERE \"Username\" = 'noor'", []).unwrap();
    let changed = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": noor.method, "code": code(&noor.secret, 1)})).await;
    assert_eq!(changed.status.as_u16(), 401);

    // It ran out, and was spent.
    let (client, id) = waiting(&instance, "omar").await;
    let past = Timestamp::now().minus(std::time::Duration::from_secs(60));
    instance.database().execute("UPDATE \"AuthChallenges\" SET \"ExpiresAt\" = ?1 WHERE \"Id\" = ?2", rusqlite::params![past, Id::parse(&id).unwrap()]).unwrap();
    assert_eq!(client.get("/api/auth/mfa/pending").await.status, 401);
    let ran_out = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": omar.method, "code": code(&omar.secret, 1)})).await;
    assert_eq!((ran_out.status.as_u16(), title(&ran_out).as_str()), (401, NOT_VALID));
}

#[tokio::test]
async fn a_mailed_code_proves_the_address_and_then_signs_the_account_in() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();

    let (client, challenge) = instance.start("maya").await;
    let id = challenge["challengeId"].as_str().unwrap();
    let sent = post(&client, "/api/auth/mfa/enroll/email/start", json!({"challengeId": id})).await;
    assert_eq!(sent.status.as_u16(), 200, "{}", sent.text());
    let sent = sent.json::<Value>();
    assert_eq!((&sent["status"], &sent["retryAfterSeconds"]), (&json!("sent"), &json!(45)));
    assert!(sent["message"].as_str().unwrap().contains("maya@example.test"));
    // At once again: wait.
    let soon = post(&client, "/api/auth/mfa/enroll/email/start", json!({"challengeId": id})).await;
    assert_eq!(soon.status.as_u16(), 429);
    assert_eq!(soon.header("retry-after").map(|seconds| seconds.parse::<i64>().unwrap() > 0), Some(true));
    assert!(soon.json::<Value>()["retryAfterSeconds"].as_i64().unwrap() > 0);
    let mails = instance.smtp.until(1).await;
    let mailed = code_in(&mails[0]);

    let wrong = post(&client, "/api/auth/mfa/enroll/email/confirm", json!({"challengeId": id, "code": "000000"})).await;
    assert_eq!(wrong.status.as_u16(), 401);
    // A name that is too long does not spend the right code.
    let name = post(&client, "/api/auth/mfa/enroll/email/confirm", json!({"challengeId": id, "code": mailed, "name": "x".repeat(65)})).await;
    assert_eq!(name.status.as_u16(), 400);
    let done = post(&client, "/api/auth/mfa/enroll/email/confirm", json!({"challengeId": id, "code": mailed, "name": "Mail"})).await;
    assert_eq!(done.status.as_u16(), 200, "{}", done.text());
    assert_eq!(done.json::<Value>()["recoveryCodes"].as_array().unwrap().len(), 10);
    client.post_json("/api/auth/logout", &json!({})).await;

    // The next sign-in is answered by mail.
    let (client, challenge) = instance.start("maya").await;
    assert_eq!(challenge["emailCodeAvailable"], true);
    let id = challenge["challengeId"].as_str().unwrap();
    let method = challenge["methods"][0]["id"].as_str().unwrap();
    for body in [json!({"challengeId": id}), json!({"challengeId": id, "methodId": "nope"}), json!({"challengeId": id, "methodId": Id::random().to_string()})] {
        let none = post(&client, "/api/auth/mfa/email/send", body).await;
        assert_eq!(none.status.as_u16(), 409);
        assert_eq!(title(&none), "That security method is not available for this account.");
    }
    let sent = post(&client, "/api/auth/mfa/email/send", json!({"challengeId": id, "methodId": method})).await;
    assert_eq!(sent.status.as_u16(), 200, "{}", sent.text());
    let mails = instance.smtp.until(2).await;
    let mailed = code_in(&mails[1]);
    let signed = post(&client, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": mailed})).await;
    assert_eq!(signed.status.as_u16(), 200, "{}", signed.text());
    // The code worked once, for that challenge.
    let (other, id) = waiting(&instance, "maya").await;
    let again = post(&other, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": mailed})).await;
    assert_eq!(again.status.as_u16(), 401);

    // With mail off, a method that is not there is still said so first.
    instance.settings("\"MailingEnabled\" = 0");
    let unknown = post(&other, "/api/auth/mfa/email/send", json!({"challengeId": id, "methodId": Id::random().to_string()})).await;
    assert_eq!(title(&unknown), "That security method is not available for this account.");
    let off = post(&other, "/api/auth/mfa/email/send", json!({"challengeId": id, "methodId": method})).await;
    assert_eq!((off.status.as_u16(), title(&off).as_str()), (409, "Verify an e-mail method before using e-mail codes."));
}

#[tokio::test]
async fn a_mailed_code_is_not_sent_when_mail_cannot_be() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    instance.settings("\"MailingEnabled\" = 0");

    let (client, challenge) = instance.start("maya").await;
    let id = challenge["challengeId"].as_str().unwrap();
    let off = post(&client, "/api/auth/mfa/enroll/email/start", json!({"challengeId": id})).await;
    assert_eq!(off.status.as_u16(), 409);
    assert_eq!(title(&off), "Verify an e-mail method before using e-mail codes.");
    // Whoever has no challenge is told so, whatever the mail is doing.
    let stranger = post(&instance.server.client(), "/api/auth/mfa/email/send", json!({"challengeId": id, "methodId": Id::random().to_string()})).await;
    assert_eq!(stranger.status.as_u16(), 401);
    let count: i64 = instance.database().query_row("SELECT COUNT(*) FROM \"EmailOtpCodes\"", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    assert!(instance.smtp.mails().is_empty());
}

#[tokio::test]
async fn a_sign_in_that_asked_not_to_be_kept_ends_in_a_short_session() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();

    let client = instance.server.client();
    let response = client
        .post_json("/api/auth/login", &json!({"username": "maya", "password": PASSWORD, "keepSignedIn": false}))
        .await;
    let id = response.json::<Value>()["challengeId"].as_str().unwrap().to_string();
    let started = post(&client, "/api/auth/mfa/enroll/totp/start", json!({"challengeId": id})).await.json::<Value>();
    let secret = started["secret"].as_str().unwrap();
    let confirmed = post(&client, "/api/auth/mfa/enroll/totp/confirm", json!({"challengeId": id, "methodId": started["methodId"], "code": code(secret, 0), "rememberDevice": true})).await;
    assert_eq!(confirmed.status.as_u16(), 200, "{}", confirmed.text());
    // The session cookie is the browser's own: no `expires`.
    assert!(!cookies(&confirmed)[0].contains("expires="), "{}", cookies(&confirmed)[0]);
    let short: bool = instance
        .database()
        .query_row("SELECT \"ShortLived\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC, rowid DESC LIMIT 1", [], |row| row.get(0))
        .unwrap();
    assert!(short);

    // Left out, the session is kept.
    let (kept, id) = waiting(&instance, "maya").await;
    let method = started["methodId"].as_str().unwrap();
    let done = post(&kept, "/api/auth/mfa/verify", json!({"challengeId": id, "methodId": method, "code": code(secret, 1)})).await;
    assert!(cookies(&done)[0].contains("expires="), "{}", cookies(&done)[0]);
}

#[tokio::test]
async fn ten_codes_an_hour_are_mailed_and_the_next_is_refused_for_an_hour() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    instance.settings("\"EmailCodeCooldownSeconds\" = 0");
    let (client, challenge) = instance.start("maya").await;
    let id = challenge["challengeId"].as_str().unwrap();

    for _ in 0..10 {
        let sent = post(&client, "/api/auth/mfa/enroll/email/start", json!({"challengeId": id})).await;
        assert_eq!(sent.status, 200, "{}", sent.text());
        assert_eq!(sent.json::<Value>()["retryAfterSeconds"], 0);
    }
    let limited = post(&client, "/api/auth/mfa/enroll/email/start", json!({"challengeId": id})).await;
    assert_eq!(limited.status, 429);
    assert_eq!(limited.header("retry-after"), Some("3600"));
    assert_eq!(limited.json::<Value>()["retryAfterSeconds"], 3600);
    assert_eq!(title(&limited), "Too many codes have been sent. Try again later.");
    assert_eq!(instance.smtp.until(10).await.len(), 10);
}

#[tokio::test]
async fn verifying_counts_against_the_login_limit() {
    let instance = Instance::with(
        &["maya"],
        &[
            ("STELLARR_ENVIRONMENT", "Production"),
            ("Rtc__TurnSharedSecret", "a-secret"),
            ("RateLimiting__Login__PermitLimit", "3"),
        ],
    )
    .await;
    let client = instance.server.client();
    let body = json!({"challengeId": Id::random().to_string(), "code": "000000"});

    for _ in 0..3 {
        assert_eq!(post(&client, "/api/auth/mfa/verify", body.clone()).await.status, 401);
    }
    let refused = post(&client, "/api/auth/mfa/verify", body).await;
    assert_eq!(refused.status, 429);
}

#[tokio::test]
async fn a_sign_in_that_asked_to_link_an_identity_clears_the_cookie_of_the_identity() {
    use stellarr_api::auth::tokens;

    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let app = instance.enrolled("maya", false).await;
    let (client, id) = waiting(&instance, "maya").await;
    // An identity waits for this browser, and the sign-in asked to link it.
    let (identity, secret) = (Id::random(), "a secret only the browser has");
    let database = instance.database();
    database
        .execute(
            "INSERT INTO \"PendingExternalIdentities\" (\"Id\", \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"EmailVerified\",
                 \"Amr\", \"ShortLived\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, 'default', 'https://issuer.test', 'subject', 0, '[]', 0, ?3, ?4)",
            rusqlite::params![identity, tokens::hash(secret), Timestamp::now(), Timestamp::now().plus(std::time::Duration::from_secs(600))],
        )
        .unwrap();
    database
        .execute("UPDATE \"AuthChallenges\" SET \"PendingIdentityId\" = ?1 WHERE \"Id\" = ?2", rusqlite::params![identity, Id::parse(&id).unwrap()])
        .unwrap();
    let cookie = format!("stellarr-dev-pending-identity={}.{secret}", identity.to_string().replace('-', ""));

    let answered = client
        .send(
            Request::new(http::Method::POST, "/api/auth/mfa/verify")
                .json(&json!({"challengeId": id, "methodId": app.method, "code": code(&app.secret, 1)}))
                .header("Cookie", &cookie),
        )
        .await;

    assert_eq!(answered.status, 200, "{}", answered.text());
    // No provider is configured yet: the link is refused, the sign-in stands.
    assert_eq!(answered.json::<Value>()["identityLink"], "failed");
    assert!(
        cookies(&answered).iter().any(|line| line.starts_with("stellarr-dev-pending-identity=;") && line.contains("1970")),
        "{:?}",
        cookies(&answered)
    );
    let waiting: i64 = database.query_row("SELECT COUNT(*) FROM \"PendingExternalIdentities\"", [], |row| row.get(0)).unwrap();
    assert_eq!(waiting, 0);
}

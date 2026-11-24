//! Passkeys: as the second step of a sign-in, as a step to set up, and alone.
//! The authenticator is the software one of the WebAuthn crate's own tests.

#[path = "../../../webauthn/tests/support/mod.rs"]
mod authenticator;

use serde_json::{Value, json};
use stellarr_webauthn::{Algorithm, AssertionResponse, RegistrationResponse, b64};

use crate::steps::{Instance, cookie_names, cookies, title};
use crate::support::{Client, Response};

const REJECTED: &str = "That passkey was not accepted.";
const UNAVAILABLE: &str = "Passkeys need the server's public HTTPS address to be set.";

fn registration_json(response: &RegistrationResponse) -> Value {
    json!({
        "id": response.id, "rawId": response.raw_id, "type": response.kind,
        "response": {
            "attestationObject": response.response.attestation_object,
            "clientDataJSON": response.response.client_data_json,
            "transports": response.response.transports,
        }
    })
}

fn assertion_json(response: &AssertionResponse) -> Value {
    json!({
        "id": response.id, "rawId": response.raw_id, "type": response.kind,
        "response": {
            "authenticatorData": response.response.authenticator_data,
            "clientDataJSON": response.response.client_data_json,
            "signature": response.response.signature,
            "userHandle": response.response.user_handle,
        }
    })
}

/// The challenge in the options a server handed out.
fn challenge_of(options: &Response) -> Vec<u8> {
    b64::decode(options.json::<Value>()["challenge"].as_str().unwrap()).unwrap()
}

/// A passkey that has been set up for an account.
struct Passkey {
    credential: Vec<u8>,
    handle: String,
    counter: u32,
}

impl Passkey {
    /// An answer to the options the server just gave, signed by this passkey.
    fn answer(&mut self, options: &Response) -> Value {
        self.counter += 1;
        let mut spec = authenticator::AssertionSpec::new(&self.credential, &challenge_of(options));
        spec.sign_count = self.counter;
        spec.user_handle = Some(self.handle.clone());
        assertion_json(&spec.build(authenticator::key(Algorithm::Es256)))
    }
}

/// Sets a passkey up for `name` through the enrolment the instance asks for.
async fn enrolled(instance: &Instance, name: &str, credential: u8, remember: bool) -> (Client, Passkey, Value) {
    let (client, challenge) = instance.start(name).await;
    let id = challenge["challengeId"].as_str().unwrap().to_string();
    assert_eq!(challenge["passkeyAvailable"], true);
    let options = client.post_json("/api/auth/mfa/enroll/passkey/start", &json!({"challengeId": id})).await;
    assert_eq!(options.status, 200, "{}", options.text());
    assert_eq!(options.header("content-type"), Some("application/json"));
    assert_eq!(options.header("cache-control"), Some("no-store"));
    let mut spec = authenticator::RegistrationSpec::new(&challenge_of(&options));
    spec.credential_id = vec![credential; 32];
    let sent = registration_json(&spec.build(authenticator::key(Algorithm::Es256)));
    let confirmed = client
        .post_json(
            "/api/auth/mfa/enroll/passkey/confirm",
            &json!({"challengeId": id, "name": "Laptop", "response": sent, "rememberDevice": remember}),
        )
        .await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.text());
    let handle: Vec<u8> = instance
        .database()
        .query_row(
            "SELECT c.\"UserHandle\" FROM \"MfaMethods\" m JOIN \"WebAuthnCredentials\" c ON c.\"MethodId\" = m.\"Id\"
             JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\" WHERE u.\"Username\" = ?1 ORDER BY m.\"CreatedAt\" DESC LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .unwrap();
    let passkey = Passkey { credential: vec![credential; 32], handle: b64::encode(&handle), counter: 0 };
    (client, passkey, confirmed.json())
}

#[tokio::test]
async fn a_passkey_is_set_up_when_the_instance_asks_and_then_answers_a_second_step() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();

    let (client, mut passkey, confirmed) = enrolled(&instance, "maya", 3, false).await;
    assert_eq!(confirmed["status"], "ok");
    assert_eq!(confirmed["recoveryCodes"].as_array().unwrap().len(), 10);
    assert_eq!(client.get("/api/session").await.json::<Value>()["user"]["username"], "maya");
    client.post_json("/api/auth/logout", &json!({})).await;

    // The sign-in that follows offers the passkey, and its options name it.
    let (client, challenge) = instance.start("maya").await;
    assert_eq!(challenge["methods"][0]["type"], "passkey");
    let id = challenge["challengeId"].as_str().unwrap();
    let options = client.post_json("/api/auth/mfa/passkey/options", &json!({"challengeId": id})).await;
    assert_eq!(options.status, 200, "{}", options.text());
    assert_eq!(options.header("content-type"), Some("application/json"));
    assert_eq!(options.json::<Value>()["allowCredentials"].as_array().unwrap().len(), 1);

    // A wrong answer spends the options, right or wrong: the next needs new ones.
    let mut spec = authenticator::AssertionSpec::new(&passkey.credential, &challenge_of(&options));
    spec.sign_count = 9;
    spec.client.origin = "https://elsewhere.test".into();
    let wrong = client
        .post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": assertion_json(&spec.build(authenticator::key(Algorithm::Es256)))}))
        .await;
    assert_eq!((wrong.status.as_u16(), title(&wrong).as_str()), (401, "The security code is invalid or has expired."));
    let stale = client.post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": passkey.answer(&options)})).await;
    assert_eq!(stale.status, 401);

    let options = client.post_json("/api/auth/mfa/passkey/options", &json!({"challengeId": id})).await;
    let answer = passkey.answer(&options);
    let done = client.post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": answer, "rememberDevice": true})).await;
    assert_eq!(done.status, 200, "{}", done.text());
    assert_eq!(cookie_names(&done), ["stellarr-dev-session", "stellarr-dev-trust", "stellarr-dev-preauth"]);
    let (evidence, fresh): (String, bool) = instance
        .database()
        .query_row(
            "SELECT \"MfaEvidence\", \"MfaCompletedAt\" IS NOT NULL FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC, rowid DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((evidence.as_str(), fresh), ("passkey", true));
    // One signed answer opens one session: the same bytes again open nothing.
    let (other, challenge) = instance.start("maya").await;
    let id = challenge["challengeId"].as_str().unwrap();
    other.post_json("/api/auth/mfa/passkey/options", &json!({"challengeId": id})).await;
    let replay = other.post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": answer})).await;
    assert_eq!(replay.status, 401);
}

#[tokio::test]
async fn what_is_not_a_passkey_answer_is_a_bad_request_or_a_refusal_as_the_first_server_says() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let (_, passkey, _) = enrolled(&instance, "maya", 4, false).await;
    let _ = passkey;
    let (client, challenge) = instance.start("maya").await;
    let id = challenge["challengeId"].as_str().unwrap();

    for response in [json!(5), json!("x"), json!([]), json!({"rawId": 5}), json!({"rawId": "!!"}), json!({"type": "nonsense"}),
                     json!({"rawId": "AAAA", "type": "public-key", "response": {}}),
                     json!({"rawId": "AAAA", "type": "public-key", "response": {"authenticatorData": "AAAA", "clientDataJSON": "AAAA", "signature": "!!"}})] {
        let bad = client.post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": response})).await;
        assert_eq!(bad.status, 400, "{response}");
        assert!(bad.body.is_empty());
    }
    // An answer with nothing in it is a wrong answer, which counts.
    for response in [json!(null), json!({}), json!({"id": "AAAA", "rawId": null, "type": "public-key"})] {
        let nothing = client.post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": response})).await;
        assert_eq!(nothing.status, 401, "{response}");
    }
    assert_eq!(instance.events("maya").iter().filter(|e| e.starts_with("mfa-failed passkey")).count(), 3);
}

#[tokio::test]
async fn a_passkey_alone_signs_in_with_a_second_step_passed_and_every_refusal_is_the_same() {
    let instance = Instance::with(&["maya", "noor", "omar"], &[]).await;
    instance.require();
    let (_, mut maya, _) = enrolled(&instance, "maya", 5, false).await;
    let (_, mut noor, _) = enrolled(&instance, "noor", 6, false).await;
    let (_, mut omar, _) = enrolled(&instance, "omar", 7, false).await;

    let browser = instance.server.client();
    let available = browser.get("/api/auth/passkey").await;
    assert_eq!(available.json::<Value>(), json!({"available": true}));
    assert_eq!(available.header("cache-control"), Some("no-store"));

    let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
    assert_eq!(options.status, 200);
    assert_eq!(options.header("content-type"), Some("application/json"));
    assert_eq!(options.json::<Value>()["allowCredentials"], json!([]));
    assert_eq!(cookie_names(&options), ["stellarr-dev-passkey"]);
    let signed = browser.post_json("/api/auth/passkey/login", &json!({"response": maya.answer(&options)})).await;
    assert_eq!(signed.status, 200, "{}", signed.text());
    assert_eq!(signed.json::<Value>()["username"], "maya");
    assert_eq!(cookie_names(&signed), ["stellarr-dev-session", "stellarr-dev-passkey"]);
    assert!(cookies(&signed)[0].contains("expires="));
    assert_eq!(browser.get("/api/session").await.json::<Value>()["user"]["username"], "maya");
    let (evidence, fresh): (String, bool) = instance
        .database()
        .query_row("SELECT \"MfaEvidence\", \"MfaCompletedAt\" IS NOT NULL FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC, rowid DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!((evidence.as_str(), fresh), ("passkey", true));
    assert!(instance.events("maya").contains(&"login-succeeded passkey".to_string()));

    // Asked not to be kept: a session of the browser's own.
    let short = instance.server.client();
    let options = short.post_json("/api/auth/passkey/options", &json!({})).await;
    let signed = short.post_json("/api/auth/passkey/login", &json!({"response": maya.answer(&options), "keepSignedIn": false})).await;
    assert_eq!(signed.status, 200);
    assert!(!cookies(&signed)[0].contains("expires="));

    // Refusals: all the same answer, and what the options cost is spent.
    let nothing_to_say = |response: Response| {
        assert_eq!((response.status.as_u16(), title(&response).as_str()), (401, REJECTED));
        assert!(cookies(&response).iter().any(|line| line.starts_with("stellarr-dev-passkey=;")));
    };
    // No cookie: the options of another browser.
    let options = instance.server.client().post_json("/api/auth/passkey/options", &json!({})).await;
    nothing_to_say(instance.server.client().post_json("/api/auth/passkey/login", &json!({"response": maya.answer(&options)})).await);
    // The account's own handle is needed.
    let browser = instance.server.client();
    let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
    let mut other_handle = maya.answer(&options);
    other_handle["response"]["userHandle"] = json!(noor.handle);
    nothing_to_say(browser.post_json("/api/auth/passkey/login", &json!({"response": other_handle})).await);
    // One set of options answers once, right or wrong.
    let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
    let answer = maya.answer(&options);
    assert_eq!(browser.post_json("/api/auth/passkey/login", &json!({"response": answer})).await.status, 200);
    nothing_to_say(browser.post_json("/api/auth/passkey/login", &json!({"response": answer})).await);
    // A credential nobody knows, and no answer at all.
    let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
    let mut unknown = authenticator::AssertionSpec::new(&[9; 32], &challenge_of(&options));
    unknown.user_handle = Some(maya.handle.clone());
    nothing_to_say(browser.post_json("/api/auth/passkey/login", &json!({"response": assertion_json(&unknown.build(authenticator::key(Algorithm::Es256)))})).await);
    let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
    drop(options);
    nothing_to_say(browser.post_json("/api/auth/passkey/login", &json!({})).await);
    // A disabled account, and one that waits for an operator's recovery: the failure is written.
    instance.database().execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'noor'", []).unwrap();
    instance.database().execute("UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1 WHERE \"Username\" = 'omar'", []).unwrap();
    for passkey in [&mut noor, &mut omar] {
        let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
        nothing_to_say(browser.post_json("/api/auth/passkey/login", &json!({"response": passkey.answer(&options)})).await);
    }
    assert!(instance.events("noor").contains(&"login-failed passkey".to_string()));
    assert!(instance.events("omar").contains(&"login-failed passkey".to_string()));
    // A body that is not an answer is a bad request.
    assert_eq!(browser.post_json("/api/auth/passkey/login", &json!({"response": 5})).await.status, 400);
}

#[tokio::test]
async fn a_wrong_answer_to_the_options_of_an_enrolment_is_refused_and_counts() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let (client, challenge) = instance.start("maya").await;
    let id = challenge["challengeId"].as_str().unwrap();

    // Before any options, and with options that were not answered: refused.
    let early = client
        .post_json("/api/auth/mfa/enroll/passkey/confirm", &json!({"challengeId": id, "response": {"id": "AAAA", "rawId": "AAAA", "type": "public-key", "response": {"attestationObject": "AAAA", "clientDataJSON": "AAAA"}}}))
        .await;
    assert_eq!((early.status.as_u16(), title(&early).as_str()), (401, REJECTED));
    let long = client.post_json("/api/auth/mfa/enroll/passkey/confirm", &json!({"challengeId": id, "name": "x".repeat(65)})).await;
    assert_eq!(long.status, 400);
    let options = client.post_json("/api/auth/mfa/enroll/passkey/start", &json!({"challengeId": id})).await;
    // The options are for this challenge's account, and for this address.
    let mut spec = authenticator::RegistrationSpec::new(&challenge_of(&options));
    spec.client.origin = "https://elsewhere.test".into();
    let elsewhere = client
        .post_json("/api/auth/mfa/enroll/passkey/confirm", &json!({"challengeId": id, "response": registration_json(&spec.build(authenticator::key(Algorithm::Es256)))}))
        .await;
    assert_eq!((elsewhere.status.as_u16(), title(&elsewhere).as_str()), (401, REJECTED));
    assert_eq!(instance.events("maya").iter().filter(|e| e.starts_with("mfa-failed passkey-enrollment")).count(), 2);
    // A person who has no passkey is told so.
    let none = client.post_json("/api/auth/mfa/passkey/options", &json!({"challengeId": id})).await;
    assert_eq!(none.status, 401, "an enrolment is not a sign-in");
}

#[tokio::test]
async fn passkeys_are_not_offered_without_a_secure_public_address_and_the_challenge_is_looked_at_first() {
    let instance = Instance::with(&["maya"], &[("Operations__PublicUrl", "http://chat.test")]).await;
    instance.require();
    let browser = instance.server.client();

    assert_eq!(browser.get("/api/auth/passkey").await.json::<Value>(), json!({"available": false}));
    let options = browser.post_json("/api/auth/passkey/options", &json!({})).await;
    assert_eq!((options.status.as_u16(), title(&options).as_str()), (409, UNAVAILABLE));
    let login = browser.post_json("/api/auth/passkey/login", &json!({})).await;
    assert_eq!(login.status, 409);
    assert!(cookies(&login).is_empty());

    let (client, challenge) = instance.start("maya").await;
    assert_eq!(challenge["passkeyAvailable"], false);
    let id = challenge["challengeId"].as_str().unwrap();
    // A challenge that cannot be used is refused before the instance's passkeys are.
    let gone = browser.post_json("/api/auth/mfa/enroll/passkey/start", &json!({"challengeId": id})).await;
    assert_eq!(gone.status, 401);
    let start = client.post_json("/api/auth/mfa/enroll/passkey/start", &json!({"challengeId": id})).await;
    assert_eq!((start.status.as_u16(), title(&start).as_str()), (409, UNAVAILABLE));
    // The name is looked at before the instance is.
    let name = client.post_json("/api/auth/mfa/enroll/passkey/confirm", &json!({"challengeId": id, "name": "x".repeat(65)})).await;
    assert_eq!(name.status, 400);
    let confirm = client.post_json("/api/auth/mfa/enroll/passkey/confirm", &json!({"challengeId": id})).await;
    assert_eq!(confirm.status, 409);
}

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


// ---- the same conversation with the first server ----

/// What the first server and this one say to the same passkey conversation.
///
/// Not part of the ordinary run. It needs the first server prepared by
/// `oracle-second-step.py prepare` (a fresh one, on port 5222, whose public
/// address is `http://localhost:5222`, with the accounts `shared`, `other`,
/// `disabled` and `empty` and a second step asked of everybody); the container
/// is `stellarr-port-oracle-secondstep`. It starts a server of its own whose
/// public address is `http://localhost`. Both are asked by the same code, with
/// the software authenticator signing for each address, and what differs by
/// nature (ids, moments, challenges, secrets) is replaced before the two
/// transcripts are compared.
///
/// ```sh
/// python3 -I oracle-second-step.py prepare
/// cargo test -p stellarr-api --test api passkey_oracle -- --ignored --nocapture
/// oracle stop secondstep
/// ```
mod oracle {
    use std::net::SocketAddr;
    use std::process::Command;

    use rusqlite::Connection;

    use super::*;

    const CONTAINER: &str = "stellarr-port-oracle-secondstep";
    const FIRST_SERVER_PASSWORD: &str = "Corona!2026-long";

    struct Side {
        name: &'static str,
        address: SocketAddr,
        password: &'static str,
        rp_id: &'static str,
        origin: &'static str,
    }

    /// Replaces what differs by nature: runs of 20 or more letters, digits,
    /// dashes and underscores (ids, challenges, handles, trace ids, keys),
    /// moments, and colours.
    fn scrub(text: &str) -> String {
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        while i < chars.len() {
            if chars[i..].starts_with(&['2', '0']) && chars.get(i + 4) == Some(&'-') && chars.get(i + 10) == Some(&'T') {
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                out.push_str("<time>");
            } else if word(chars[i]) {
                let start = i;
                while i < chars.len() && word(chars[i]) {
                    i += 1;
                }
                if i - start >= 20 {
                    out.push_str("<b64>");
                } else {
                    out.extend(&chars[start..i]);
                }
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        let mut cleaned = String::new();
        let mut rest = out.as_str();
        while let Some(at) = rest.find("oklch(") {
            cleaned.push_str(&rest[..at]);
            rest = &rest[at..];
            rest = &rest[rest.find(')').map_or(rest.len(), |end| end + 1)..];
            cleaned.push_str("<colour>");
        }
        cleaned.push_str(rest);
        cleaned
    }

    fn note(lines: &mut Vec<String>, label: &str, response: &Response) {
        lines.push(format!("-- {label}"));
        lines.push(format!("   => {}", response.status.as_u16()));
        for name in ["cache-control", "pragma", "content-type", "retry-after"] {
            if let Some(value) = response.header(name) {
                lines.push(format!("   {name}: {value}"));
            }
        }
        let mut set: Vec<String> = cookies(response)
            .iter()
            .map(|line| {
                let mut parts = line.split("; ");
                let (name, value) = parts.next().unwrap().split_once('=').unwrap();
                // The first server runs in Production, which names the session
                // cookie for HTTPS and marks it secure even over plain HTTP.
                let name = if name.ends_with("-session") { "session" } else { name };
                let rest: Vec<String> = parts
                    .filter(|part| *part != "secure")
                    .map(|part| match part.strip_prefix("expires=") {
                        Some(date) if date.contains("1970") => "expires=1970".to_string(),
                        Some(_) => "expires=<later>".to_string(),
                        None => part.to_string(),
                    })
                    .collect();
                format!("{name}={}; {}", if value.is_empty() { "" } else { "<value>" }, rest.join("; "))
            })
            .collect();
        // The order of the lines is reported apart: it is the order the
        // cookies are set in, which no browser looks at.
        if set.len() > 1 {
            let order: Vec<&str> = set.iter().map(|line| line.split('=').next().unwrap()).collect();
            lines.push(format!("   cookies set in the order {}", order.join(", ")));
        }
        set.sort();
        for line in set {
            lines.push(format!("   set-cookie: {line}"));
        }
        if !response.body.is_empty() {
            lines.push(format!("   body: {}", scrub(&response.text())));
        }
    }

    async fn login(side: &Side, client: &Client, name: &str) -> Value {
        client
            .post_json("/api/auth/login", &json!({"username": name, "password": side.password}))
            .await
            .json()
    }

    fn spec_for(side: &Side, credential: u8, challenge: &[u8]) -> authenticator::AssertionSpec {
        let mut spec = authenticator::AssertionSpec::new(&[credential; 32], challenge);
        spec.rp_id = side.rp_id.into();
        spec.client.origin = side.origin.into();
        spec
    }

    async fn conversation(side: &Side) -> Vec<String> {
        let mut lines = Vec::new();
        let key = authenticator::key(Algorithm::Es256);
        let browser = || Client::browser(side.address);

        // Forced enrolment of a passkey.
        let client = browser();
        let challenge = login(side, &client, "shared").await;
        let id = challenge["challengeId"].as_str().unwrap().to_string();
        let options = client.post_json("/api/auth/mfa/enroll/passkey/start", &json!({"challengeId": id})).await;
        note(&mut lines, "options to add a passkey", &options);
        let handle = options.json::<Value>()["user"]["id"].as_str().unwrap().to_string();
        let mut registration = authenticator::RegistrationSpec::new(&challenge_of(&options));
        registration.rp_id = side.rp_id.into();
        registration.client.origin = side.origin.into();
        registration.credential_id = vec![21; 32];
        let mut wrong = registration.clone();
        wrong.client.origin = "https://elsewhere.test".into();
        let refused = client
            .post_json("/api/auth/mfa/enroll/passkey/confirm", &json!({"challengeId": id, "response": registration_json(&wrong.build(key))}))
            .await;
        note(&mut lines, "an answer made for another address", &refused);
        // The options went with the wrong answer: new ones.
        let options = client.post_json("/api/auth/mfa/enroll/passkey/start", &json!({"challengeId": id})).await;
        registration.client = authenticator::ClientDataSpec::new("webauthn.create", &challenge_of(&options));
        registration.client.origin = side.origin.into();
        let confirmed = client
            .post_json(
                "/api/auth/mfa/enroll/passkey/confirm",
                &json!({"challengeId": id, "name": "Laptop", "response": registration_json(&registration.build(key)), "rememberDevice": true}),
            )
            .await;
        note(&mut lines, "the passkey is added and the sign-in finished", &confirmed);
        client.post_json("/api/auth/logout", &json!({})).await;

        // The passkey as the second step.
        let mut counter = 0u32;
        let challenge = login(side, &client, "shared").await;
        lines.push(format!("sign-in: {}", scrub(&challenge.to_string())));
        let id = challenge["challengeId"].as_str().unwrap().to_string();
        let options = client.post_json("/api/auth/mfa/passkey/options", &json!({"challengeId": id})).await;
        note(&mut lines, "options for the second step", &options);
        let mut spec = spec_for(side, 21, &challenge_of(&options));
        spec.user_handle = Some(handle.clone());
        spec.sign_count = counter + 1;
        let mut elsewhere = spec.clone();
        elsewhere.client.origin = "https://elsewhere.test".into();
        let wrong = client
            .post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": assertion_json(&elsewhere.build(key))}))
            .await;
        note(&mut lines, "an answer for another address", &wrong);
        let spent = client
            .post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": assertion_json(&spec.build(key))}))
            .await;
        note(&mut lines, "the right answer to options a wrong one spent", &spent);
        let options = client.post_json("/api/auth/mfa/passkey/options", &json!({"challengeId": id})).await;
        let mut spec = spec_for(side, 21, &challenge_of(&options));
        counter += 1;
        spec.user_handle = Some(handle.clone());
        spec.sign_count = counter;
        let done = client
            .post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id, "response": assertion_json(&spec.build(key))}))
            .await;
        note(&mut lines, "the right answer", &done);
        let nothing = client.post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": id})).await;
        note(&mut lines, "an answer on a spent challenge", &nothing);
        for (label, body) in [("a number", json!(5)), ("a bad rawId", json!({"rawId": "!!"})), ("nothing in it", json!({})), ("no members of the inner answer", json!({"rawId": "AAAA", "type": "public-key", "response": {}}))] {
            let other = browser();
            let challenge = login(side, &other, "shared").await;
            let response = other
                .post_json("/api/auth/mfa/passkey/verify", &json!({"challengeId": challenge["challengeId"], "response": body}))
                .await;
            note(&mut lines, &format!("a passkey answer that is {label}"), &response);
        }

        // The passkey alone.
        let client = browser();
        note(&mut lines, "are passkeys offered", &client.get("/api/auth/passkey").await);
        for (label, keep) in [("kept", json!(null)), ("not kept", json!(false)), ("kept, said so", json!(true))] {
            let options = client.post_json("/api/auth/passkey/options", &json!({})).await;
            note(&mut lines, &format!("options of a sign-in, {label}"), &options);
            counter += 1;
            let mut spec = spec_for(side, 21, &challenge_of(&options));
            spec.user_handle = Some(handle.clone());
            spec.sign_count = counter;
            let mut body = json!({"response": assertion_json(&spec.build(key))});
            if !keep.is_null() {
                body["keepSignedIn"] = keep;
            }
            let signed = client.post_json("/api/auth/passkey/login", &body).await;
            note(&mut lines, &format!("sign in with the passkey alone, {label}"), &signed);
        }
        // Refusals of a sign-in with a passkey alone.
        let earlier = client.post_json("/api/auth/passkey/options", &json!({})).await;
        counter += 1;
        let mut spec = spec_for(side, 21, &challenge_of(&earlier));
        spec.user_handle = Some(handle.clone());
        spec.sign_count = counter;
        let answer = assertion_json(&spec.build(key));
        let nobody = browser();
        note(&mut lines, "no cookie at all", &nobody.post_json("/api/auth/passkey/login", &json!({"response": answer.clone()})).await);
        note(&mut lines, "no cookie, no answer", &nobody.post_json("/api/auth/passkey/login", &json!({})).await);
        note(&mut lines, "an answer that is not one", &client.post_json("/api/auth/passkey/login", &json!({"response": 5})).await);
        // The cookie holds the newest options: an answer to older ones is no answer.
        let _newest = client.post_json("/api/auth/passkey/options", &json!({})).await;
        note(&mut lines, "an answer to the options of an earlier try", &client.post_json("/api/auth/passkey/login", &json!({"response": answer})).await);
        let options = client.post_json("/api/auth/passkey/options", &json!({})).await;
        counter += 1;
        let mut spec = spec_for(side, 21, &challenge_of(&options));
        spec.user_handle = Some("AAAAAAAAAAAAAAAAAAAAAA".into());
        spec.sign_count = counter;
        note(&mut lines, "another account's handle", &client.post_json("/api/auth/passkey/login", &json!({"response": assertion_json(&spec.build(key))})).await);
        let options = client.post_json("/api/auth/passkey/options", &json!({})).await;
        let mut unknown = spec_for(side, 99, &challenge_of(&options));
        unknown.user_handle = Some(handle.clone());
        note(&mut lines, "a credential nobody knows", &client.post_json("/api/auth/passkey/login", &json!({"response": assertion_json(&unknown.build(key))})).await);
        let options = client.post_json("/api/auth/passkey/options", &json!({})).await;
        counter += 1;
        let mut spec = spec_for(side, 21, &challenge_of(&options));
        spec.user_handle = Some(handle.clone());
        spec.sign_count = counter;
        let answer = assertion_json(&spec.build(key));
        note(&mut lines, "a right answer", &client.post_json("/api/auth/passkey/login", &json!({"response": answer.clone()})).await);
        note(&mut lines, "the same answer again", &client.post_json("/api/auth/passkey/login", &json!({"response": answer})).await);
        lines
    }

    /// What the conversation left in the database.
    fn rows(connection: &Connection) -> Vec<String> {
        let mut lines = Vec::new();
        let mut one = |label: &str, sql: &str| {
            let mut statement = connection.prepare(sql).unwrap();
            let count = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    Ok((0..count)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join(" "))
                })
                .unwrap();
            for row in rows {
                lines.push(format!("{label}: {}", scrub(&row.unwrap())));
            }
        };
        one("credential", "SELECT \"SignCount\", \"BackupEligible\", \"BackedUp\", length(\"UserHandle\"), length(\"CredentialId\"), length(\"PublicKey\"), \"Transports\", length(\"AaGuid\") FROM \"WebAuthnCredentials\"");
        one("method", "SELECT \"Type\", \"Name\", \"ConfirmedAt\" IS NOT NULL, \"RevokedAt\" IS NOT NULL FROM \"MfaMethods\" WHERE \"Type\" = 'passkey'");
        one("event", "SELECT \"Kind\", \"Detail\" FROM \"SecurityEvents\" WHERE \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'shared') ORDER BY \"CreatedAt\", rowid");
        one("challenge", "SELECT \"Purpose\", \"Attempts\", \"ConsumedAt\" IS NOT NULL, \"WebAuthnOptions\" IS NOT NULL FROM \"AuthChallenges\" WHERE \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'shared') ORDER BY \"CreatedAt\", rowid");
        one("session", "SELECT \"PrimarySource\", \"MfaEvidence\", \"MfaCompletedAt\" IS NOT NULL, \"ShortLived\", \"RevokedAt\" IS NOT NULL FROM \"UserSessions\" WHERE \"PrimarySource\" <> 'registration' AND \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'shared') ORDER BY \"CreatedAt\", rowid");
        one("sign-in options kept", "SELECT COUNT(*), SUM(\"ConsumedAt\" IS NOT NULL) FROM \"PasskeySignIns\"");
        lines
    }

    /// The first server's database, as it is now: the container is stopped
    /// for the copy and started again.
    fn first_server_database() -> Connection {
        let run = |args: &[&str]| assert!(Command::new("docker").args(args).status().unwrap().success(), "{args:?}");
        run(&["stop", "-t", "10", CONTAINER]);
        let path = std::env::temp_dir().join(format!("second-step-oracle-{}.db", std::process::id()));
        run(&["cp", &format!("{CONTAINER}:/tmp/data/stellarr.db"), path.to_str().unwrap()]);
        run(&["start", CONTAINER]);
        Connection::open(path).unwrap()
    }

    #[tokio::test]
    #[ignore = "needs the first server; see the header of this module"]
    async fn passkey_oracle() {
        let address: SocketAddr = std::env::var("ORACLE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:5222".into()).parse().unwrap();
        let instance = Instance::with(&["shared", "other", "disabled", "empty"], &[("Operations__PublicUrl", "http://localhost")]).await;
        instance.require();
        let ours = Side { name: "this server", address: instance.server.address(), password: crate::support::PASSWORD, rp_id: "localhost", origin: "http://localhost" };
        let theirs = Side { name: "first server", address, password: FIRST_SERVER_PASSWORD, rp_id: "localhost", origin: "http://localhost:5222" };

        let mut transcripts = Vec::new();
        for side in [&theirs, &ours] {
            let mut lines = conversation(side).await;
            lines.push("== left in the database".to_string());
            let connection = if side.name == "first server" { first_server_database() } else { instance.database() };
            lines.extend(rows(&connection));
            transcripts.push(lines);
        }
        let (first, second) = (&transcripts[0], &transcripts[1]);
        let (mut differences, mut known) = (0, 0);
        for index in 0..first.len().max(second.len()) {
            let (a, b) = (first.get(index), second.get(index));
            // Two differences are known and reported, not failed on: the order
            // the cookies of an answer are set in (the ceremony hands them back
            // session first), and the order of the members of the options for
            // adding a passkey (the WebAuthn crate writes them in its own).
            let order = |line: Option<&String>| line.is_some_and(|line| line.starts_with("   cookies set in the order"));
            let creation = |line: Option<&String>| line.is_some_and(|line| line.contains("\"pubKeyCredParams\""));
            if a != b && ((order(a) && order(b)) || (creation(a) && creation(b))) {
                known += 1;
                continue;
            }
            if a != b {
                differences += 1;
                println!("first server: {}\nthis server:  {}\n", a.map_or("(nothing)", String::as_str), b.map_or("(nothing)", String::as_str));
            }
        }
        println!("{} lines compared, {differences} differ, {known} known differences left out", first.len().max(second.len()));
        assert_eq!(differences, 0);
    }
}

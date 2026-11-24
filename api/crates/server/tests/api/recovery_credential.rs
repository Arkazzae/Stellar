//! The credential an operator hands over after recovering an account: it is
//! exchanged for the right to set a new second step up, and for nothing else.

use serde_json::{Value, json};
use stellarr_api::auth::policy::SecurityPolicy;
use stellarr_api::auth::recovery;
use stellarr_api::values::{Id, Timestamp};

use crate::steps::{Instance, code, cookie_names, cookies, title};
use crate::support::PASSWORD;

const INVALID: &str = "The recovery credential is invalid or has expired.";

/// What `security recover` does to an account, and the credential it prints.
fn recovered(instance: &Instance, name: &str) -> String {
    let mut connection = instance.database();
    let user: Id = connection
        .query_row("SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = ?1", [name], |row| row.get(0))
        .unwrap();
    let tx = connection.transaction().unwrap();
    let done = recovery::recover(&tx, user, "lost phone", &SecurityPolicy::default(), Timestamp::now(), |_| Ok(()))
        .unwrap()
        .expect("the account is recovered");
    tx.commit().unwrap();
    done.credential
}

async fn redeem(instance: &Instance, credential: &str) -> crate::support::Response {
    instance
        .server
        .client()
        .post_json("/api/auth/recovery/redeem", &json!({"credential": credential}))
        .await
}

#[tokio::test]
async fn the_credential_gives_a_challenge_to_set_a_new_second_step_up_and_a_session_only_after_it() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.require();
    let app = instance.enrolled("maya", true).await;
    let credential = recovered(&instance, "maya");

    // Everything the account had is gone, and its password signs nobody in.
    assert!(app.client.get("/api/session").await.json::<Value>()["user"].is_null());
    let waiting = instance.server.login(&instance.server.client(), "maya", PASSWORD).await;
    assert_eq!(waiting.json::<Value>()["status"], "security_recovery_required");

    // What is not a credential.
    for bad in ["", "short", &"a".repeat(513), &"a".repeat(43)] {
        let refused = redeem(&instance, bad).await;
        assert_eq!((refused.status.as_u16(), title(&refused).as_str()), (400, INVALID), "{bad}");
    }

    let client = instance.server.client();
    let redeemed = client
        .post_json("/api/auth/recovery/redeem", &json!({"credential": format!("  {credential}  "), "keepSignedIn": false}))
        .await;
    assert_eq!(redeemed.status, 200, "{}", redeemed.text());
    assert_eq!(redeemed.header("cache-control"), Some("no-store"));
    assert_eq!(cookie_names(&redeemed), ["stellarr-dev-preauth"]);
    let body = redeemed.json::<Value>();
    assert_eq!((&body["status"], &body["purpose"]), (&json!("mfa_enrollment_required"), &json!("recovery")));
    let id = body["challengeId"].as_str().unwrap();
    assert!(instance.events("maya").contains(&"cli-recovery recovery credential redeemed".to_string()));

    // It is single use, and it opens no session by itself.
    assert_eq!(redeem(&instance, &credential).await.status, 400);
    assert_eq!(client.get("/api/session").await.json::<Value>()["user"], Value::Null);
    let answered = client.post_json("/api/auth/mfa/verify", &json!({"challengeId": id, "code": "123456"})).await;
    assert_eq!(answered.status, 401);

    // The new app finishes it, with the sign-in's own choice of a short session.
    let started = client.post_json("/api/auth/mfa/enroll/totp/start", &json!({"challengeId": id})).await.json::<Value>();
    let secret = started["secret"].as_str().unwrap();
    let confirmed = client
        .post_json("/api/auth/mfa/enroll/totp/confirm", &json!({"challengeId": id, "methodId": started["methodId"], "code": code(secret, 0)}))
        .await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.text());
    assert_eq!(confirmed.json::<Value>()["recoveryCodes"].as_array().unwrap().len(), 10);
    assert!(!cookies(&confirmed)[0].contains("expires="));
    let waiting: bool = instance
        .database()
        .query_row("SELECT \"SecurityRecoveryRequired\" FROM \"Users\" WHERE \"Username\" = 'maya'", [], |row| row.get(0))
        .unwrap();
    assert!(!waiting);
    assert_eq!(instance.server.login(&client, "maya", PASSWORD).await.json::<Value>()["status"], "mfa_required");
}

#[tokio::test]
async fn a_credential_that_ran_out_or_is_of_a_disabled_account_or_an_older_recovery_is_refused() {
    let instance = Instance::with(&["maya", "noor", "omar"], &[]).await;
    instance.require();
    for name in ["maya", "noor", "omar"] {
        instance.enrolled(name, false).await;
    }
    let ran_out = recovered(&instance, "maya");
    let disabled = recovered(&instance, "noor");
    let older = recovered(&instance, "omar");
    let newer = recovered(&instance, "omar");
    let database = instance.database();
    database.execute("UPDATE \"CliRecoveryCredentials\" SET \"ExpiresAt\" = ?1 WHERE \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'maya')", [Timestamp::now().minus(std::time::Duration::from_secs(5))]).unwrap();
    database.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'noor'", []).unwrap();

    for credential in [&ran_out, &disabled, &older] {
        assert_eq!(redeem(&instance, credential).await.status, 400);
    }
    assert_eq!(redeem(&instance, &newer).await.status, 200);
}

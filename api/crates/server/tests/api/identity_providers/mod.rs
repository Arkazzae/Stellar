//! Signing in, signing up and linking through an identity provider, against a
//! provider of our own over TLS (see `fake`).

mod certs;
mod fake;

use serde_json::{Value, json};
use stellarr_oidc::form;

use self::fake::FakeProvider;
use crate::support::{Client, PASSWORD, Person, Response, Server};

struct World {
    server: Server,
    provider: FakeProvider,
    owner: Person,
}

async fn world_with(extra: &[(String, String)]) -> World {
    let provider = FakeProvider::start().await;
    let mut settings = provider.settings("Test IdP");
    settings.extend_from_slice(extra);
    let pairs: Vec<(&str, &str)> = settings.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let server = Server::start_with(&pairs).await;
    let owner = server.found("owner").await;
    World { server, provider, owner }
}

async fn world() -> World {
    world_with(&[]).await
}

fn link(server: &Server, user: &str, issuer: &str, subject: &str, provider: Option<&str>) {
    server
        .database()
        .execute(
            "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, '2026-01-01 00:00:00')",
            rusqlite::params![stellarr_api::values::Id::random().to_db_text(), user.to_uppercase(), issuer, subject, provider],
        )
        .unwrap();
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// A round trip that has gone to the provider and not yet come back.
struct Begun {
    state: String,
    query: Vec<(String, String)>,
    location: String,
}

async fn begin(client: &Client, provider: &FakeProvider, path: &str) -> Begun {
    let response = client.get(path).await;
    assert_eq!(response.status, 302, "{}", response.text());
    let location = response.header("location").unwrap().to_string();
    assert!(location.starts_with(&provider.issuer()), "{location}");
    let query = form::parse(location.split_once('?').unwrap().1).unwrap();
    let get = |name: &str| query.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or_default();
    provider.set_nonce(&get("nonce"));
    Begun { state: get("state"), query, location }
}

async fn answer(client: &Client, begun: &Begun, route: &str) -> Response {
    client
        .get(&format!("{route}?code={}&state={}", fake::CODE, form::encode_component(&begun.state)))
        .await
}

async fn sign_in(client: &Client, provider: &FakeProvider, return_url: &str) -> Response {
    let begun = begin(client, provider, &format!("/api/auth/oidc/login?returnUrl={}", form::encode_component(return_url))).await;
    answer(client, &begun, "/api/auth/oidc/callback").await
}

async fn who(client: &Client) -> Value {
    client.get("/api/session").await.json::<Value>()["user"].clone()
}

fn location(response: &Response) -> String {
    response.header("location").unwrap_or("").to_string()
}

#[tokio::test]
async fn nothing_is_offered_until_a_provider_is_set_up() {
    let server = Server::start().await;
    let client = server.client();
    let status = client.get("/api/auth/oidc").await;
    assert_eq!(status.text(), r#"{"enabled":false,"displayName":"Main account","providers":[]}"#);
    assert_eq!(status.header("cache-control"), Some("no-store"));
    let login = client.get("/api/auth/oidc/login").await;
    assert_eq!(login.status, 404);
    assert_eq!(login.header("pragma"), Some("no-cache"));
    assert_eq!(client.get("/api/auth/oidc/callback").await.status, 404);
}

#[tokio::test]
async fn a_provider_is_offered_and_people_are_sent_to_it() {
    let w = world().await;
    let client = w.server.client();
    let status: Value = client.get("/api/auth/oidc").await.json();
    assert_eq!(
        status,
        json!({"enabled": true, "displayName": "Test IdP", "providers": [
            {"id": "default", "displayName": "Test IdP", "loginUrl": "/api/auth/oidc/login", "signUp": false}]})
    );
    let begun = begin(&client, &w.provider, "/api/auth/oidc/login?returnUrl=%2Fx").await;
    let get = |name: &str| begun.query.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    assert_eq!(get("client_id"), Some(fake::CLIENT_ID));
    assert_eq!(get("response_type"), Some("code"));
    assert_eq!(get("scope"), Some("openid profile email"));
    assert_eq!(get("code_challenge_method"), Some("S256"));
    assert_eq!(get("max_age"), None);
    assert_eq!(get("redirect_uri"), Some(format!("http://{}/api/auth/oidc/callback", w.server.address()).as_str()));
    assert!(begun.location.starts_with(&format!("{}/authorize?", w.provider.issuer())));
    let cookie = client
        .send(crate::support::Request::new(http::Method::GET, "/api/auth/oidc/login"))
        .await;
    let line = cookie.headers.get_all("set-cookie").iter().next().unwrap().to_str().unwrap().to_string();
    assert!(line.starts_with("stellarr-oidc-") && line.contains("; path=/api/auth/oidc/callback; secure; samesite=none; httponly"), "{line}");
}

#[tokio::test]
async fn a_linked_account_signs_in_and_is_sent_only_where_this_origin_goes() {
    let w = world().await;
    link(&w.server, &w.owner.id, &w.provider.issuer(), &w.provider.subject(), None);
    for (asked, expected) in [
        ("/invite/abc?source=oidc", "/invite/abc?source=oidc"),
        ("https://evil.example/", "/"),
        ("//evil.example/", "/"),
        ("/\\evil.example/", "/"),
        ("/.//evil.example", "/"),
        ("/%2e//evil.example", "/"),
        ("/a/..//evil.example", "/"),
        ("/%2F%2Fevil.example", "/"),
        ("/invite/v1.2", "/invite/v1.2"),
        ("javascript:alert(1)", "/"),
        ("/a\u{1}b", "/"),
    ] {
        let client = w.server.client();
        let back = sign_in(&client, &w.provider, asked).await;
        assert_eq!(back.status, 302);
        assert_eq!(location(&back), expected, "{asked}");
        assert_eq!(who(&client).await["id"], json!(w.owner.id));
    }
    let sessions = count(&w.server, "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"PrimarySource\" = 'oidc'");
    assert_eq!(sessions, 11);
}

#[tokio::test]
async fn an_identity_nobody_linked_waits_for_its_browser_and_can_be_dropped() {
    let w = world().await;
    w.provider.script.lock().unwrap().claims.insert("email".into(), json!("ola@example.test"));
    w.provider.script.lock().unwrap().claims.insert("email_verified".into(), json!(true));
    w.provider.script.lock().unwrap().claims.insert("name".into(), json!("Ola Nowak"));
    let client = w.server.client();
    let back = sign_in(&client, &w.provider, "/invite/x").await;
    assert_eq!(location(&back), "/login?oidc=unlinked&returnUrl=%2Finvite%2Fx");
    assert!(who(&client).await.is_null());
    let waiting: Value = client.get("/api/auth/oidc/pending").await.json();
    assert_eq!(
        waiting,
        json!({"provider": {"id": "default", "displayName": "Test IdP"}, "displayName": "Ola Nowak",
               "email": "ola@example.test", "username": "ola.nowak", "signUp": false})
    );
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"PendingExternalIdentities\" WHERE \"EmailVerified\" = 1"), 1);
    // Another browser has nothing waiting.
    assert_eq!(w.server.client().get("/api/auth/oidc/pending").await.status, 404);
    let dropped = client.send(crate::support::Request::new(http::Method::DELETE, "/api/auth/oidc/pending")).await;
    assert_eq!(dropped.status, 204);
    assert_eq!(client.get("/api/auth/oidc/pending").await.status, 404);
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"PendingExternalIdentities\""), 0);
}

#[tokio::test]
async fn a_refusal_a_replay_and_an_answer_for_another_browser_end_on_the_login_page() {
    let w = world().await;
    link(&w.server, &w.owner.id, &w.provider.issuer(), &w.provider.subject(), None);
    let client = w.server.client();
    let begun = begin(&client, &w.provider, "/api/auth/oidc/login").await;
    let denied = client
        .get(&format!("/api/auth/oidc/callback?error=access_denied&state={}", form::encode_component(&begun.state)))
        .await;
    assert_eq!(location(&denied), "/login?oidc_error=cancelled");
    // Posted, the same refusal reads as a failure.
    let begun = begin(&client, &w.provider, "/api/auth/oidc/login").await;
    let posted = client
        .send(crate::support::Request::new(http::Method::POST, "/api/auth/oidc/callback").body(
            "application/x-www-form-urlencoded",
            &format!("error=access_denied&state={}", form::encode_component(&begun.state)),
        ))
        .await;
    assert_eq!(location(&posted), "/login?oidc_error=failed");
    // An answer for another browser's request.
    let begun = begin(&client, &w.provider, "/api/auth/oidc/login").await;
    let stranger = w.server.client();
    assert_eq!(location(&answer(&stranger, &begun, "/api/auth/oidc/callback").await), "/login?oidc_error=failed");
    assert!(who(&stranger).await.is_null());
    // The same answer twice.
    let begun = begin(&client, &w.provider, "/api/auth/oidc/login").await;
    assert_eq!(location(&answer(&client, &begun, "/api/auth/oidc/callback").await), "/");
    assert_eq!(location(&answer(&client, &begun, "/api/auth/oidc/callback").await), "/login?oidc_error=failed");
    // No code, no state, not a callback of this provider.
    assert_eq!(location(&client.get("/api/auth/oidc/callback").await), "/login?oidc_error=failed");
    assert_eq!(client.get("/api/auth/oidc/work/callback").await.status, 404);
    assert_eq!(client.get("/api/auth/oidc/default/callback").await.status, 404);
}

#[tokio::test]
async fn a_provider_that_misbehaves_signs_nobody_in() {
    use self::fake::Signing;
    let w = world().await;
    link(&w.server, &w.owner.id, &w.provider.issuer(), &w.provider.subject(), None);
    type Change = Box<dyn Fn(&mut fake::Script)>;
    let cases: Vec<(&str, Change)> = vec![
        ("a stranger's key", Box::new(|s| s.signing = Signing::Stranger)),
        ("an unknown key id", Box::new(|s| s.signing = Signing::StrangerUnderOwnId)),
        ("no signature", Box::new(|s| s.signing = Signing::Unsigned)),
        ("the wrong algorithm", Box::new(|s| s.alg = "HS256".into())),
        ("an expired token", Box::new(|s| { s.claims.insert("exp".into(), json!(1)); })),
        ("another audience", Box::new(|s| { s.claims.insert("aud".into(), json!("someone-else")); })),
        ("another issuer", Box::new(|s| { s.claims.insert("iss".into(), json!("https://evil.test")); })),
        ("no nonce", Box::new(|s| { s.claims.insert("nonce".into(), Value::Null); })),
        ("another nonce", Box::new(|s| { s.claims.insert("nonce".into(), json!("not-ours")); })),
        ("no issue time", Box::new(|s| { s.claims.insert("iat".into(), Value::Null); })),
        ("a huge subject", Box::new(|s| { s.claims.insert("sub".into(), json!("x".repeat(10_000))); })),
        ("no access token", Box::new(|s| s.no_access_token = true)),
        ("a token endpoint that says no", Box::new(|s| { s.instead.insert("/token".into(), fake::Instead { status: Some(500), ..Default::default() }); })),
    ];
    for (what, change) in cases {
        let provider = FakeProvider::start().await;
        // Each case has a provider and a server of its own: the keys a server
        // reads once are kept.
        let mut settings = provider.settings("Test IdP");
        settings.push(("Testing__Marker".into(), what.into()));
        let pairs: Vec<(&str, &str)> = settings.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let server = Server::start_with(&pairs).await;
        let owner = server.found("owner").await;
        link(&server, &owner.id, &provider.issuer(), &provider.subject(), None);
        change(&mut provider.script.lock().unwrap());
        let client = server.client();
        let back = sign_in(&client, &provider, "/").await;
        assert_eq!(location(&back), "/login?oidc_error=failed", "{what}");
        assert!(who(&client).await.is_null(), "{what}");
    }
    drop(w);
}

#[tokio::test]
async fn the_second_step_of_the_provider_counts_only_when_trusted_and_recent() {
    for (trusted, amr, age, signs_in) in [
        (true, vec!["pwd", "mfa"], 10, true),
        (false, vec!["pwd", "mfa"], 10, false),
        (true, vec!["pwd"], 10, false),
        (true, vec!["pwd", "mfa"], 3 * 3600, false),
    ] {
        let extra = if trusted { vec![("Oidc__TrustProviderMfa".to_string(), "true".to_string())] } else { vec![] };
        let w = world_with(&extra).await;
        w.server
            .database()
            .execute(
                "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\")
                 SELECT \"Id\", '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, '2026-11-24 00:00:00', 'everyone' FROM \"Servers\" LIMIT 1
                 ON CONFLICT (\"ServerId\") DO UPDATE SET \"MfaRequirement\" = 'everyone'",
                [],
            )
            .unwrap();
        link(&w.server, &w.owner.id, &w.provider.issuer(), &w.provider.subject(), None);
        {
            let mut script = w.provider.script.lock().unwrap();
            script.amr = Some(amr.iter().map(|a| a.to_string()).collect());
            script.auth_time = Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 - age);
        }
        let client = w.server.client();
        let asked = client.get("/api/auth/oidc/login").await;
        assert_eq!(asked.header("location").unwrap().contains("max_age=300"), trusted);
        let back = sign_in(&client, &w.provider, "/").await;
        if signs_in {
            assert_eq!(location(&back), "/");
            let evidence: String = w.server.database().query_row("SELECT \"MfaEvidence\" FROM \"UserSessions\" WHERE \"PrimarySource\" = 'oidc'", [], |r| r.get(0)).unwrap();
            assert_eq!(evidence, "provider_mfa");
        } else {
            assert!(location(&back).starts_with("/login?mfa_"), "{}", location(&back));
            assert!(who(&client).await.is_null());
        }
    }
}

#[tokio::test]
async fn an_account_links_itself_and_then_signs_in_through_the_provider() {
    let w = world().await;
    let client = w.server.sign_in(&w.owner).await;
    // A provider that is not set up, and a wrong password.
    let missing = client.post_json("/api/me/security/identities/work/link", &json!({"password": PASSWORD})).await;
    assert_eq!(missing.status, 404);
    // The account has no second step: a fresh sign-in is proof enough.
    let started = client.post_json("/api/me/security/identities/default/link", &json!({})).await;
    assert_eq!(started.status, 200, "{}", started.text());
    assert_eq!(started.header("cache-control"), Some("no-store"));
    let url = started.json::<Value>()["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("/api/auth/oidc/link?ticket="), "{url}");
    // Another browser cannot use the ticket.
    assert_eq!(location(&w.server.client().get(&url).await), "/link-account?result=failed");
    let begun = begin(&client, &w.provider, &url).await;
    assert!(begun.location.contains("redirect_uri="));
    let done = answer(&client, &begun, "/api/auth/oidc/callback").await;
    assert_eq!(location(&done), "/link-account?result=linked");
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"ExternalIdentities\""), 1);
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'identity-linked' AND \"Detail\" = 'Test IdP'"), 1);
    let fresh = w.server.client();
    assert_eq!(location(&sign_in(&fresh, &w.provider, "/").await), "/");
    assert_eq!(who(&fresh).await["id"], json!(w.owner.id));
}

#[tokio::test]
async fn a_link_answered_after_its_session_ended_or_elsewhere_links_nothing() {
    let w = world().await;
    let client = w.server.sign_in(&w.owner).await;
    let url = client.post_json("/api/me/security/identities/default/link", &json!({})).await.json::<Value>()["url"].as_str().unwrap().to_string();
    let begun = begin(&client, &w.provider, &url).await;
    w.server.database().execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = '2026-01-01 00:00:00'", []).unwrap();
    assert_eq!(location(&answer(&client, &begun, "/api/auth/oidc/callback").await), "/link-account?result=failed");
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"ExternalIdentities\""), 0);
}

#[tokio::test]
async fn an_identity_cannot_be_taken_over_and_the_last_way_in_cannot_be_removed() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    link(&w.server, &maya.id, &w.provider.issuer(), &w.provider.subject(), None);
    let client = w.server.sign_in(&w.owner).await;
    let url = client.post_json("/api/me/security/identities/default/link", &json!({})).await.json::<Value>()["url"].as_str().unwrap().to_string();
    let begun = begin(&client, &w.provider, &url).await;
    assert_eq!(location(&answer(&client, &begun, "/api/auth/oidc/callback").await), "/link-account?result=taken");
    // Maya has a password, so her identity can go; make it her only way in.
    let hers = w.server.sign_in(&maya).await;
    let id: String = w.server.database().query_row("SELECT \"Id\" FROM \"ExternalIdentities\"", [], |r| r.get(0)).unwrap();
    let id = id.to_lowercase();
    w.server.database().execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Username\" = 'maya'", []).unwrap();
    let refused = hers.post_json(&format!("/api/me/security/identities/{id}/remove"), &json!({})).await;
    assert_eq!(refused.status, 409, "{}", refused.text());
    assert_eq!(refused.json::<Value>()["code"], json!("last-way-in"));
    w.server.database().execute("UPDATE \"Users\" SET \"PasswordHash\" = (SELECT \"PasswordHash\" FROM \"Users\" WHERE \"Username\" = 'owner') WHERE \"Username\" = 'maya'", []).unwrap();
    let removed = hers.post_json(&format!("/api/me/security/identities/{id}/remove"), &json!({})).await;
    assert_eq!(removed.status, 204, "{}", removed.text());
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'identity-unlinked'"), 1);
    assert_eq!(hers.post_json(&format!("/api/me/security/identities/{id}/remove"), &json!({})).await.status, 404);
}

async fn invited_identity(w: &World) -> (Client, String) {
    w.server.database().execute("INSERT INTO \"IdentityProviderSettings\" VALUES ('default', 1, '2026-01-01 00:00:00')", []).unwrap();
    {
        let mut script = w.provider.script.lock().unwrap();
        script.claims.insert("email".into(), json!("ola@example.test"));
        script.claims.insert("email_verified".into(), json!(true));
        script.claims.insert("name".into(), json!("Ola Nowak"));
    }
    let client = w.server.client();
    assert_eq!(location(&sign_in(&client, &w.provider, "/").await), "/login?oidc=unlinked");
    (client, w.server.invite())
}

#[tokio::test]
async fn a_provider_allowed_to_create_accounts_makes_one_from_an_invite() {
    let w = world().await;
    let (client, invite) = invited_identity(&w).await;
    let waiting: Value = client.get("/api/auth/oidc/pending").await.json();
    assert_eq!(waiting["signUp"], json!(true));
    let made = client
        .post_json("/api/auth/register", &json!({"username": "ola.nowak", "displayName": "Ola", "email": "ola@example.test", "inviteCode": invite, "external": true}))
        .await;
    assert_eq!(made.status, 201, "{}", made.text());
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"Users\" WHERE \"PasswordHash\" IS NULL AND \"ProviderVerifiedEmail\" = 'OLA@EXAMPLE.TEST'"), 1);
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"ExternalIdentities\""), 1);
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"PendingExternalIdentities\""), 0);
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"PrimarySource\" = 'oidc'"), 1);
    assert_eq!(client.get("/api/auth/oidc/pending").await.status, 404);
}

#[tokio::test]
async fn a_sign_up_is_refused_for_what_it_lacks() {
    let w = world().await;
    let body = |invite: &str, email: &str| json!({"username": "ola.nowak", "displayName": "Ola", "email": email, "inviteCode": invite, "external": true});
    // Nothing waiting.
    let nobody = w.server.client().post_json("/api/auth/register", &body("x", "ola@example.test")).await;
    assert_eq!(nobody.status, 400);
    // Waiting, but the provider may not create accounts.
    let client = w.server.client();
    assert_eq!(location(&sign_in(&client, &w.provider, "/").await), "/login?oidc=unlinked");
    let closed = client.post_json("/api/auth/register", &body(&w.server.invite(), "ola@example.test")).await;
    assert_eq!(closed.status, 403, "{}", closed.text());
    // Allowed, an address that is only the provider's word is not verified, and a taken one is refused.
    w.server.database().execute("INSERT INTO \"IdentityProviderSettings\" VALUES ('default', 1, '2026-01-01 00:00:00')", []).unwrap();
    let taken = client.post_json("/api/auth/register", &body(&w.server.invite(), "owner@example.test")).await;
    assert_eq!(taken.status, 400);
    let made = client.post_json("/api/auth/register", &body(&w.server.invite(), "other@example.test")).await;
    assert_eq!(made.status, 201, "{}", made.text());
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"Users\" WHERE \"ProviderVerifiedEmail\" IS NOT NULL"), 0);
}

#[tokio::test]
async fn a_password_sign_in_links_the_identity_that_waits_for_the_browser() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let client = w.server.client();
    assert_eq!(location(&sign_in(&client, &w.provider, "/").await), "/login?oidc=unlinked");
    let back = client.post_json("/api/auth/login", &json!({"username": "maya", "password": PASSWORD, "linkPending": true})).await;
    assert_eq!(back.status, 200, "{}", back.text());
    assert_eq!(back.json::<Value>()["identityLink"], json!("linked"));
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"ExternalIdentities\""), 1);
    assert_eq!(count(&w.server, "SELECT COUNT(*) FROM \"PendingExternalIdentities\""), 0);
    // Nothing waits any more.
    let again = w.server.client().post_json("/api/auth/login", &json!({"username": "maya", "password": PASSWORD, "linkPending": true})).await;
    assert_eq!(again.json::<Value>()["identityLink"], json!("failed"));
    let _ = maya;
}

#[tokio::test]
async fn a_named_provider_has_routes_of_its_own_beside_the_original() {
    let work = FakeProvider::start().await;
    let other = FakeProvider::start().await;
    let mut settings = other.settings("Test IdP");
    settings.extend(work.named_settings("work", "Work"));
    // One file holds both authorities' certificates.
    let bundle = std::env::temp_dir().join(format!("stellarr-it-bundle-{}", stellarr_api::values::Id::random()));
    std::fs::write(&bundle, format!("{}{}", certs::CA, certs::CA)).unwrap();
    settings.retain(|(name, _)| name != "SSL_CERT_FILE");
    settings.push(("SSL_CERT_FILE".into(), bundle.display().to_string()));
    let pairs: Vec<(&str, &str)> = settings.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let server = Server::start_with(&pairs).await;
    let owner = server.found("owner").await;
    let client = server.client();
    let status: Value = client.get("/api/auth/oidc").await.json();
    let offered: Vec<(&str, &str, &str)> = status["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["displayName"].as_str().unwrap(), p["loginUrl"].as_str().unwrap()))
        .collect();
    assert_eq!(offered, [("default", "Test IdP", "/api/auth/oidc/login"), ("work", "Work", "/api/auth/oidc/work/login")]);
    link(&server, &owner.id, &work.issuer(), &work.subject(), Some("work"));
    // The same subject at another issuer is another person.
    other.script.lock().unwrap().subject = work.subject();
    assert_eq!(location(&sign_in(&client, &other, "/").await), "/login?oidc=unlinked");
    let begun = begin(&client, &work, "/api/auth/oidc/work/login").await;
    let back = answer(&client, &begun, "/api/auth/oidc/work/callback").await;
    assert_eq!(location(&back), "/");
    assert_eq!(who(&client).await["id"], json!(owner.id));
    assert!(work.requests().iter().any(|(method, path, _)| method == "POST" && path == "/token"));
    std::fs::remove_file(bundle).ok();
}

fn seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn a_fresh_sign_in_at_the_provider_moves_the_sessions_proof_forward() {
    let w = world().await;
    link(&w.server, &w.owner.id, &w.provider.issuer(), &w.provider.subject(), None);
    let client = w.server.sign_in(&w.owner).await;
    let old = "2026-01-01 00:00:00";
    w.server.database().execute("UPDATE \"UserSessions\" SET \"AuthenticatedAt\" = ?1", [old]).unwrap();
    let authenticated = |server: &Server| -> String {
        server.database().query_row("SELECT MAX(\"AuthenticatedAt\") FROM \"UserSessions\"", [], |r| r.get(0)).unwrap()
    };
    assert_eq!(client.post_json("/api/me/security/identities/work/reauthenticate", &json!({})).await.status, 404);
    let own = w.provider.subject();
    for (auth_time, subject, expected, moves) in [
        (Some(seconds_now() - 3600), None, "unconfirmed", false),
        (None, None, "unconfirmed", false),
        (Some(seconds_now()), Some("somebody-else"), "unconfirmed", false),
        (Some(seconds_now() - 30), Some(own.as_str()), "confirmed", true),
    ] {
        {
            let mut script = w.provider.script.lock().unwrap();
            script.auth_time = auth_time;
            if let Some(subject) = subject {
                script.subject = subject.to_string();
            }
        }
        let started = client.post_json("/api/me/security/identities/default/reauthenticate", &json!({})).await;
        assert_eq!(started.status, 200, "{}", started.text());
        let url = started.json::<Value>()["url"].as_str().unwrap().to_string();
        assert!(url.starts_with("/api/auth/oidc/reauthenticate?ticket="), "{url}");
        let begun = begin(&client, &w.provider, &format!("{url}&returnUrl=%2Fsettings")).await;
        let get = |name: &str| begun.query.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
        assert_eq!(get("max_age").as_deref(), Some("300"));
        assert_eq!(get("prompt").as_deref(), Some("login"));
        let back = answer(&client, &begun, "/api/auth/oidc/callback").await;
        assert_eq!(location(&back), format!("/link-account?result={expected}&returnUrl=%2Fsettings"));
        assert_eq!(authenticated(&w.server) != old, moves, "{expected}");
    }
    // A ticket for a fresh sign-in starts no link, and one for a link no fresh sign-in.
    let t = client.post_json("/api/me/security/identities/default/reauthenticate", &json!({})).await.json::<Value>()["url"].as_str().unwrap().replace("/reauthenticate?", "/link?");
    assert_eq!(location(&client.get(&t).await), "/link-account?result=failed");
    let l = client.post_json("/api/me/security/identities/default/link", &json!({})).await.json::<Value>()["url"].as_str().unwrap().replace("/link?", "/reauthenticate?");
    assert!(location(&client.get(&l).await).starts_with("/link-account?result=unconfirmed"));
}

#[tokio::test]
async fn two_removals_at_once_still_leave_a_way_in() {
    let other = FakeProvider::start().await;
    let provider = FakeProvider::start().await;
    let mut settings = provider.settings("First");
    settings.extend(other.named_settings("work", "Work"));
    let bundle = std::env::temp_dir().join(format!("stellarr-it-bundle-{}", stellarr_api::values::Id::random()));
    std::fs::write(&bundle, format!("{}{}", certs::CA, certs::CA)).unwrap();
    settings.retain(|(name, _)| name != "SSL_CERT_FILE");
    settings.push(("SSL_CERT_FILE".into(), bundle.display().to_string()));
    let pairs: Vec<(&str, &str)> = settings.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let server = Server::start_with(&pairs).await;
    let owner = server.found("owner").await;
    link(&server, &owner.id, &provider.issuer(), "one", None);
    link(&server, &owner.id, &other.issuer(), "two", Some("work"));
    let client = server.sign_in(&owner).await;
    server.database().execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL", []).unwrap();
    let ids: Vec<String> = {
        let connection = server.database();
        let mut statement = connection.prepare("SELECT \"Id\" FROM \"ExternalIdentities\"").unwrap();
        statement.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|id| id.unwrap().to_lowercase()).collect()
    };
    let (first, second) = (format!("/api/me/security/identities/{}/remove", ids[0]), format!("/api/me/security/identities/{}/remove", ids[1]));
    let nothing = json!({});
    let (a, b) = tokio::join!(client.post_json(&first, &nothing), client.post_json(&second, &nothing));
    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [204, 409], "{} {}", a.text(), b.text());
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ExternalIdentities\""), 1);
    std::fs::remove_file(bundle).ok();
}

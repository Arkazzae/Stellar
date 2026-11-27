//! Founding the instance and registering: what is written, what is refused and
//! what is said about it.

use http::Method;
use serde_json::{Value, json};

use crate::support::{PASSWORD, Request, Server};

const POLISH: (&str, &str) = ("STELLARR_LANGUAGE", "pl");

fn body(username: &str, code: Option<&str>) -> Value {
    json!({
        "username": username,
        "displayName": "Owner",
        "email": format!("{username}@example.test"),
        "password": PASSWORD,
        "setupCode": code,
    })
}

async fn errors_of(response: crate::support::Response) -> Value {
    assert_eq!(response.status, 400, "{}", response.text());
    assert_eq!(response.header("content-type"), Some("application/problem+json"));
    response.json::<Value>()["errors"].clone()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn an_empty_instance_says_it_waits_for_its_owner_and_a_founded_one_does_not() {
    let server = Server::start().await;
    let client = server.client();

    let before = client.get("/api/auth/registration").await;
    server.found("owner").await;
    let after = client.get("/api/auth/registration").await;

    assert_eq!(before.header("cache-control"), Some("no-store"));
    assert_eq!(before.json::<Value>(), json!({ "setupRequired": true }));
    assert_eq!(after.json::<Value>(), json!({ "setupRequired": false }));
}

#[tokio::test]
async fn the_first_registration_uses_the_configuration_of_the_wizard() {
    for (voice, channels) in [("spotkania", 2), ("", 1)] {
        let server = Server::start().await;
        let code = server.setup_code().await;
        let mut request = body("owner", Some(&code));
        request["initialServer"] = json!({
            "name": "  Nocna ekipa  ", "description": "  Wspólne gry  ", "textChannelName": "  GRY  ", "voiceChannelName": voice
        });

        let response = server.client().post_json("/api/auth/register", &request).await;

        assert_eq!(response.status, 201, "{}", response.text());
        let (name, description, public): (String, String, bool) = server
            .database()
            .query_row("SELECT \"Name\", \"Description\", \"IsPublic\" FROM \"Servers\"", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        assert_eq!((name.as_str(), description.as_str(), public), ("Nocna ekipa", "Wspólne gry", false));
        assert_eq!(count(&server, "SELECT count(*) FROM \"Channels\""), channels);
        let text: String = server.database().query_row("SELECT \"Name\" FROM \"Channels\" WHERE \"Kind\" = 0", [], |row| row.get(0)).unwrap();
        assert_eq!(text, "gry");
        assert_eq!(count(&server, "SELECT \"OwnerSlot\" FROM \"ServerMembers\""), 1);
    }
}

#[tokio::test]
async fn an_invalid_initial_server_creates_no_account_and_no_instance() {
    for (name, text, voice, field) in [(" ", "gry", "rozmowa", "server"), ("Ekipa", "ze spacją", "rozmowa", "channels"), ("Ekipa", "gry", "GRY", "channels")] {
        let server = Server::start().await;
        let code = server.setup_code().await;
        let mut request = body("owner", Some(&code));
        request["initialServer"] = json!({ "name": name, "textChannelName": text, "voiceChannelName": voice });

        let response = server.client().post_json("/api/auth/register", &request).await;

        let errors = errors_of(response).await;
        assert!(errors.get(field).is_some(), "{errors}");
        assert_eq!(count(&server, "SELECT count(*) FROM \"Users\""), 0);
        assert_eq!(count(&server, "SELECT count(*) FROM \"Servers\""), 0);
    }
}

#[tokio::test]
async fn the_first_registration_founds_the_instance_and_signs_the_caller_in() {
    let server = Server::start().await;
    let client = server.client();
    let code = server.setup_code().await;

    let response = client.post_json("/api/auth/register", &body("owner", Some(&code))).await;

    assert_eq!(response.status, 201);
    assert_eq!(response.header("location"), Some("/api/me"));
    assert_eq!(response.header("cache-control"), Some("no-cache,no-store"));
    assert_eq!(response.header("pragma"), Some("no-cache"));
    let user = response.json::<Value>();
    assert_eq!((user["username"].as_str(), user["presence"].as_str()), (Some("owner"), Some("online")));
    assert!(user["origin"].is_null());
    // The cookie it was handed signs in the next request.
    assert!(client.cookie("__Host-stellarr-session").or(client.cookie("stellarr-dev-session")).is_some());
    let session = client.get("/api/session").await.json::<Value>();
    assert_eq!(session["user"]["id"], user["id"]);

    let roles: Vec<(String, i64, bool)> = server
        .database()
        .prepare("SELECT \"Name\", \"Permissions\", \"IsOwner\" FROM \"ServerRoles\" ORDER BY \"Position\"")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(roles, [("Owner".to_string(), 131_071, true), ("Member".to_string(), 24064, false)]);
    let channels: Vec<(String, i64)> = server
        .database()
        .prepare("SELECT \"Name\", \"Kind\" FROM \"Channels\" ORDER BY \"Position\"")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(channels, [("general".to_string(), 0), ("voice".to_string(), 1)]);
    let (content, system): (String, bool) = server
        .database()
        .query_row("SELECT \"Content\", \"IsSystem\" FROM \"Messages\"", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!((content.as_str(), system), ("**Owner** has joined the server. Say hello!", true));
    let device: String = server.database().query_row("SELECT \"Device\" FROM \"UserSessions\"", [], |row| row.get(0)).unwrap();
    assert_eq!(device, "Unknown device");
    assert_eq!(count(&server, "SELECT count(*) FROM \"ServerMemberRoles\""), 1);
    assert_eq!(count(&server, "SELECT count(*) FROM \"ServerAdminSettings\" WHERE \"MentionsOnly\" = 1 AND \"AllMessages\" = 0"), 1);
}

#[tokio::test]
async fn the_welcome_is_announced_to_the_server_after_it_is_committed() {
    let server = Server::start().await;
    server.found("owner").await;

    let events = server.events();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "MessageCreated");
    assert!(events[0]["audience"]["server"].is_string());
    assert_eq!(events[0]["payload"]["author"]["id"], "system");
    assert_eq!(events[0]["payload"]["content"], "**Owner** has joined the server. Say hello!");
    let stored: String = server.database().query_row("SELECT \"Id\" FROM \"Messages\"", [], |row| row.get(0)).unwrap();
    assert_eq!(events[0]["payload"]["id"].as_str().unwrap().to_uppercase(), stored);
}

#[tokio::test]
async fn a_registered_account_signs_in_with_its_email_and_keeps_only_a_hash_of_the_password() {
    let server = Server::start().await;
    server.found("owner").await;

    let response = server.login(&server.client(), "OWNER@Example.Test", PASSWORD).await;

    assert_eq!(response.status, 200);
    let stored: String = server.database().query_row("SELECT \"PasswordHash\" FROM \"Users\"", [], |row| row.get(0)).unwrap();
    assert!(stored.starts_with("pbkdf2-sha256$210000$"));
    assert!(!stored.contains(PASSWORD));
}

#[tokio::test]
async fn a_later_registration_without_an_invite_is_refused_even_with_the_setup_code() {
    let server = Server::start().await;
    let code = server.setup_code().await;
    server.found("owner").await;

    let response = server.client().post_json("/api/auth/register", &body("maya", Some(&code))).await;

    let errors = errors_of(response).await;
    assert_eq!(errors["inviteCode"][0], "An invite is required to create an account on an existing instance.");
    assert_eq!(count(&server, "SELECT count(*) FROM \"Users\""), 1);
}

#[tokio::test]
async fn founding_without_the_setup_code_writes_nothing_and_leaves_the_code_valid() {
    for wrong in [None, Some(""), Some("AAAA-AAAA-AAAA")] {
        let server = Server::start_with(&[POLISH]).await;
        let code = server.setup_code().await;

        let response = server.client().post_json("/api/auth/register", &body("owner", wrong)).await;

        let errors = errors_of(response).await;
        assert_eq!(
            errors["setupCode"][0],
            "Wpisz kod konfiguracji z logu serwera. Przy każdym uruchomieniu serwer pokazuje nowy; użyj ostatniego."
        );
        assert_eq!(count(&server, "SELECT count(*) FROM \"Users\""), 0);
        let accepted = server.client().post_json("/api/auth/register", &body("owner", Some(&code))).await;
        assert_eq!(accepted.status, 201);
    }
}

#[tokio::test]
async fn the_setup_code_is_accepted_whatever_its_case_spaces_and_dashes() {
    let server = Server::start().await;
    let code = server.setup_code().await;
    let plain = code.replace('-', "");

    let typed = format!(" {} {} {} ", plain[..4].to_lowercase(), &plain[4..8], &plain[8..]);
    let response = server.client().post_json("/api/auth/register", &body("owner", Some(&typed))).await;

    assert_eq!(response.status, 201, "{}", response.text());
}

#[tokio::test]
async fn an_instance_with_accounts_but_no_server_cannot_be_founded_again() {
    let server = Server::start().await;
    server.found("owner").await;
    server.database().execute("DELETE FROM \"Servers\"", []).unwrap();
    let code = server.setup_code().await;

    let response = server.client().post_json("/api/auth/register", &body("second", Some(&code))).await;

    assert_eq!(errors_of(response).await["inviteCode"][0], "An invite is required to create an account on an existing instance.");
}

#[tokio::test]
async fn each_start_of_an_empty_instance_logs_a_new_code_and_only_that_one_founds_it() {
    let folder = crate::support::TempDir::new();
    let first = Server::start_in(folder.path(), &[]).await;
    let old = first.setup_code().await;
    assert!(first.stop().await);
    let second = Server::start_in(folder.path(), &[]).await;
    let new = second.setup_code().await;

    let refused = second.client().post_json("/api/auth/register", &body("owner", Some(&old))).await;
    let accepted = second.client().post_json("/api/auth/register", &body("owner", Some(&new))).await;

    assert_ne!(old, new);
    assert_eq!(refused.status, 400);
    assert_eq!(accepted.status, 201);
    assert!(second.stop().await);
    // Once there is an owner a start logs no code.
    let third = Server::start_in(folder.path(), &[]).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!third.log().iter().any(|line| line.contains("Setup code")), "{:?}", third.log());
}

#[tokio::test]
async fn an_instance_without_a_public_address_says_where_to_look_and_one_with_it_gives_the_address() {
    let without = Server::start().await;
    let with = Server::start_with(&[("Operations__PublicUrl", "https://chat.example.test/")]).await;

    let plain = without.setup_code().await;
    let named = with.log().into_iter().find(|line| line.contains("Setup code")).unwrap();

    assert_eq!(plain.len(), 14);
    assert!(without.log().iter().any(|line| line.contains("Open /setup at the address of this instance to create the owner account.")));
    assert!(named.contains("Open https://chat.example.test/setup to create the owner account."), "{named}");
}

#[tokio::test]
async fn every_invalid_field_is_answered_with_its_own_message() {
    let server = Server::start_with(&[POLISH]).await;
    let cases = [
        ("ab", "Name", "a@example.test", PASSWORD, "username", "Nazwa użytkownika musi mieć 3–32 znaki: litery, cyfry, kropkę, myślnik lub podkreślenie."),
        ("a-very-long-username-of-33-chars-xx", "Name", "a@example.test", PASSWORD, "username", "Nazwa użytkownika musi mieć 3–32 znaki: litery, cyfry, kropkę, myślnik lub podkreślenie."),
        ("with space", "Name", "a@example.test", PASSWORD, "username", "Nazwa użytkownika musi mieć 3–32 znaki: litery, cyfry, kropkę, myślnik lub podkreślenie."),
        ("valid.name", "   ", "a@example.test", PASSWORD, "displayName", "Nazwa widoczna musi mieć od 1 do 64 znaków."),
        ("valid.name", "x1234567890123456789012345678901234567890123456789012345678901234", "a@example.test", PASSWORD, "displayName", "Nazwa widoczna musi mieć od 1 do 64 znaków."),
        ("valid.name", "Name", "not-an-email", PASSWORD, "email", "Podaj poprawny adres e-mail."),
        ("valid.name", "Name", "Ala <ala@example.test>", PASSWORD, "email", "Podaj poprawny adres e-mail."),
        ("valid.name", "Name", "", PASSWORD, "email", "Podaj poprawny adres e-mail."),
        ("valid.name", "Name", "a@example.test", "short12", "password", "Hasło musi mieć co najmniej 8 znaków."),
    ];
    for (username, display_name, email, password, field, message) in cases {
        let response = server
            .client()
            .post_json("/api/auth/register", &json!({ "username": username, "displayName": display_name, "email": email, "password": password }))
            .await;

        assert_eq!(errors_of(response).await, json!({ field: [message] }), "{username} {display_name} {email}");
    }
    assert_eq!(count(&server, "SELECT count(*) FROM \"Users\""), 0);
}

#[tokio::test]
async fn a_password_longer_than_the_limit_is_refused_and_one_at_the_limit_is_not() {
    let server = Server::start_with(&[POLISH]).await;
    let code = server.setup_code().await;
    let mut request = body("owner", Some(&code));
    request["password"] = json!("p".repeat(257));

    let refused = server.client().post_json("/api/auth/register", &request).await;
    request["password"] = json!("p".repeat(256));
    let accepted = server.client().post_json("/api/auth/register", &request).await;

    assert_eq!(errors_of(refused).await, json!({ "password": ["Hasło może mieć najwyżej 256 znaków."] }));
    assert_eq!(accepted.status, 201);
}

#[tokio::test]
async fn every_invalid_field_is_reported_at_once() {
    let server = Server::start().await;

    let response = server
        .client()
        .post_json("/api/auth/register", &json!({ "username": "x", "displayName": "", "email": "nope", "password": "1" }))
        .await;

    let errors = errors_of(response).await;
    let mut keys: Vec<&String> = errors.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["displayName", "email", "password", "username"]);
}

#[tokio::test]
async fn a_username_and_an_address_are_unique_whatever_the_letter_case() {
    let server = Server::start().await;
    server.found("owner").await;
    let invite = server.invite();

    let response = server
        .client()
        .post_json(
            "/api/auth/register",
            &json!({ "username": "OWNER", "displayName": "Other", "email": "Owner@Example.TEST", "password": PASSWORD, "inviteCode": invite }),
        )
        .await;

    let errors = errors_of(response).await;
    assert_eq!(errors["username"][0], "This username is already taken.");
    assert_eq!(errors["email"][0], "An account with this e-mail address already exists.");
}

#[tokio::test]
async fn an_invite_adds_the_account_to_the_invited_server_as_a_member() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let invite = server.invite();

    let maya = server.client();
    let response = maya
        .post_json(
            "/api/auth/register",
            &json!({ "username": "maya", "displayName": "Maya Lin", "email": "maya@example.test", "password": PASSWORD, "inviteCode": format!(" {} ", invite.to_uppercase()) }),
        )
        .await;

    assert_eq!(response.status, 201, "{}", response.text());
    let (role, held, uses): (String, i64, i64) = server
        .database()
        .query_row(
            "SELECT r.\"Name\", (SELECT count(*) FROM \"ServerMemberRoles\" WHERE \"UserId\" = u.\"Id\"),
                    (SELECT \"Uses\" FROM \"ServerInvites\" LIMIT 1)
             FROM \"Users\" u JOIN \"ServerMembers\" m ON m.\"UserId\" = u.\"Id\" JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
             WHERE u.\"Username\" = 'maya'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((role.as_str(), held, uses), ("Member", 0, 1));
    assert_ne!(owner.id, response.json::<Value>()["id"]);
}

#[tokio::test]
async fn the_welcome_escapes_every_markup_character_in_the_display_name() {
    let server = Server::start().await;
    server.found("owner").await;

    let response = server
        .client()
        .post_json(
            "/api/auth/register",
            &json!({ "username": "maya", "displayName": "**x** _y_ [z](u) <@1>", "email": "maya@example.test", "password": PASSWORD, "inviteCode": server.invite() }),
        )
        .await;

    assert_eq!(response.status, 201);
    let content: String = server
        .database()
        .query_row("SELECT \"Content\" FROM \"Messages\" ORDER BY \"CreatedAt\" DESC LIMIT 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(content, "**\\*\\*x\\*\\* \\_y\\_ \\[z\\]\\(u\\) \\<\\@1\\>** has joined the server. Say hello!");
}

#[tokio::test]
async fn an_invite_that_cannot_be_used_creates_no_account() {
    for kind in ["unknown", "expired", "used-up", "server-gone"] {
        let server = Server::start_with(&[POLISH]).await;
        server.found("owner").await;
        let code = server.invite();
        let connection = server.database();
        match kind {
            "expired" => connection.execute("UPDATE \"ServerInvites\" SET \"ExpiresAt\" = '2020-01-01 00:00:00'", []).unwrap(),
            "used-up" => connection.execute("UPDATE \"ServerInvites\" SET \"MaxUses\" = 1, \"Uses\" = 1", []).unwrap(),
            "server-gone" => {
                connection.execute("PRAGMA foreign_keys = OFF", []).ok();
                connection.execute("UPDATE \"ServerInvites\" SET \"ServerId\" = '11111111-1111-1111-1111-111111111111'", []).unwrap()
            }
            _ => 0,
        };
        let used = if kind == "unknown" { "nope".to_string() } else { code };

        let response = server
            .client()
            .post_json(
                "/api/auth/register",
                &json!({ "username": "maya", "displayName": "Maya", "email": "maya@example.test", "password": PASSWORD, "inviteCode": used }),
            )
            .await;

        let errors = errors_of(response).await;
        let message = errors["inviteCode"][0].as_str().unwrap().to_string();
        let expected = if kind == "server-gone" {
            "Serwer wskazany przez zaproszenie już nie istnieje."
        } else {
            "Zaproszenie wygasło, osiągnęło limit użyć albo jest nieprawidłowe."
        };
        assert_eq!(message, expected, "{kind}");
        assert_eq!(count(&server, "SELECT count(*) FROM \"Users\""), 1, "{kind}");
    }
}

#[tokio::test]
async fn simultaneous_first_registrations_found_one_instance_between_them() {
    let server = Server::start().await;
    let code = server.setup_code().await;
    let attempts: Vec<_> = (0..6)
        .map(|index| {
            let client = server.client();
            let request = body(&format!("owner{index}"), Some(&code));
            tokio::spawn(async move { client.post_json("/api/auth/register", &request).await.status })
        })
        .collect();

    let mut statuses = Vec::new();
    for attempt in attempts {
        statuses.push(attempt.await.unwrap().as_u16());
    }

    // The limit of five registrations a window refuses none of them in the
    // Testing environment; exactly one found the instance and the rest need
    // an invite.
    assert_eq!(statuses.iter().filter(|status| **status == 201).count(), 1, "{statuses:?}");
    assert_eq!(count(&server, "SELECT count(*) FROM \"Users\""), 1);
    assert_eq!(count(&server, "SELECT count(*) FROM \"Servers\""), 1);
}

#[tokio::test]
async fn simultaneous_registrations_of_one_account_answer_created_and_validation() {
    let server = Server::start().await;
    server.found("owner").await;
    let attempts: Vec<_> = (0..5)
        .map(|_| {
            let client = server.client();
            let request = json!({ "username": "maya", "displayName": "Maya", "email": "maya@example.test", "password": PASSWORD, "inviteCode": server.invite() });
            tokio::spawn(async move { client.post_json("/api/auth/register", &request).await.status })
        })
        .collect();

    let mut statuses = Vec::new();
    for attempt in attempts {
        statuses.push(attempt.await.unwrap().as_u16());
    }

    assert_eq!(statuses.iter().filter(|status| **status == 201).count(), 1, "{statuses:?}");
    assert!(statuses.iter().all(|status| *status == 201 || *status == 400), "{statuses:?}");
    assert_eq!(count(&server, "SELECT count(*) FROM \"Users\" WHERE \"Username\" = 'maya'"), 1);
}

#[tokio::test]
async fn registration_attempts_are_limited_per_address() {
    let server = Server::start_with(&[
        ("STELLARR_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret"),
    ])
    .await;
    let client = server.client();

    let mut statuses = Vec::new();
    for _ in 0..7 {
        statuses.push(client.post_json("/api/auth/register", &json!({})).await.status.as_u16());
    }

    assert_eq!(statuses, [400, 400, 400, 400, 400, 429, 429]);
}

#[tokio::test]
async fn an_external_registration_has_no_identity_waiting_for_this_browser() {
    let server = Server::start_with(&[POLISH]).await;

    let response = server
        .client()
        .post_json("/api/auth/register", &json!({ "username": "owner", "displayName": "O", "email": "owner@example.test", "external": true }))
        .await;

    assert_eq!(response.status, 400);
    assert!(response.json::<Value>()["title"].as_str().unwrap().starts_with("Logowanie u dostawcy"), "{}", response.text());
}

#[tokio::test]
async fn a_body_that_is_not_json_is_refused_before_anything_is_counted() {
    let server = Server::start().await;

    let text = server.client().send(Request::new(Method::POST, "/api/auth/register").body("text/plain", "x")).await;
    let broken = server.client().send(Request::new(Method::POST, "/api/auth/register").body("application/json", "{bad")).await;

    assert_eq!([text.status, broken.status], [415, 400]);
}

/// Registers `ab{name}cd` and says what came of it, as the probe of the first
/// server did: `created`, or the field that was refused.
async fn try_name(server: &Server, number: usize, name: &str) -> String {
    let response = server
        .client()
        .post_json(
            "/api/auth/register",
            &json!({
                "username": format!("ab{name}cd"), "displayName": "N", "email": format!("u{number}@example.test"),
                "password": PASSWORD, "inviteCode": server.invite()
            }),
        )
        .await;
    if response.status == 201 {
        return "created".into();
    }
    let mut keys: Vec<String> = response.json::<Value>()["errors"].as_object().map(|e| e.keys().cloned().collect()).unwrap_or_default();
    keys.sort();
    keys.join(",")
}

/// Registers each name pair in one instance. A collision makes the second
/// registration return `username` because both share an uppercase key.
async fn collisions(pairs: &[(&str, &str, &str, &str)]) {
    let server = Server::start().await;
    server.found("owner").await;
    for (index, (first, second, first_result, second_result)) in pairs.iter().enumerate() {
        let a = try_name(&server, index * 2, first).await;
        let b = try_name(&server, index * 2 + 1, second).await;
        assert_eq!((a.as_str(), b.as_str()), (*first_result, *second_result), "{first} then {second}");
    }
}

#[tokio::test]
async fn username_collisions_follow_the_pinned_uppercase_mapping() {
    collisions(&[
        ("İ", "i", "created", "created"),
        ("ß", "ẞ", "created", "created"),
        ("ſ", "S", "created", "username"),
        ("σ", "Σ", "created", "username"),
        ("ς", "Σ", "username", "username"),
        ("я", "Я", "created", "username"),
        ("ᏸ", "Ᏸ", "created", "username"),
        ("ꭰ", "Ꭰ", "created", "username"),
        ("ა", "Ა", "created", "username"),
        ("µ", "Μ", "created", "username"),
        ("ǆ", "ǅ", "created", "username"),
        ("é", "É", "created", "username"),
        ("ŉ", "ʼN", "created", "created"),
    ])
    .await;
}

#[tokio::test]
async fn the_dotless_i_is_not_capitalized_and_so_collides_with_nothing() {
    // `bıll` and `bill` have distinct uppercase keys.
    collisions(&[("ı", "I", "created", "created")]).await;
}

#[tokio::test]
async fn user_names_may_hold_letters_and_numbers_and_not_the_signs_that_go_with_them() {
    let server = Server::start().await;
    server.found("owner").await;
    for (index, (name, accepted)) in [("ａｂｃ", true), ("名前ab", true), ("अआ12", true), ("क्षि", false), ("देवनागरी", false)].into_iter().enumerate() {
        let got = try_name(&server, 100 + index, name).await;

        assert_eq!(got == "created", accepted, "{name}: {got}");
    }
}

#[tokio::test]
async fn an_account_made_with_an_invite_to_a_channel_is_greeted_in_that_channel_even_a_hidden_one() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let connection = server.database();
    let instance: String = connection.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    let base: String = connection
        .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
        .unwrap();
    let staff = stellarr_api::values::Id::random().to_db_text();
    connection
        .execute(
            "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'staff', 0, 5, '', 0, 0, 0, NULL)",
            rusqlite::params![staff, instance],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
            rusqlite::params![stellarr_api::values::Id::random().to_db_text(), instance, staff, base],
        )
        .unwrap();
    let invite = owner
        .client
        .post_json("/api/admin/invites", &json!({ "channelId": staff.to_lowercase() }))
        .await;
    assert_eq!(invite.status, 201, "{}", invite.text());
    let url = invite.json::<Value>()["url"].as_str().unwrap().to_string();
    let code = url.rsplit('/').next().unwrap().to_string();

    let response = server
        .client()
        .post_json(
            "/api/auth/register",
            &json!({ "username": "maya", "displayName": "Maya", "email": "maya@example.test", "password": PASSWORD, "inviteCode": code }),
        )
        .await;

    assert_eq!(response.status, 201, "{}", response.text());
    let channel: String = server
        .database()
        .query_row(
            "SELECT c.\"Name\" FROM \"Messages\" m JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\" WHERE m.\"Content\" LIKE '%Maya%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(channel, "staff");
}

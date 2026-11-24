//! The door: which builds this instance will let sign in.
//!
//! The sign-in and session routes are written in a later group of endpoints.
//! The gate stands in front of their paths whether or not anything answers
//! there, so a refusal is tested as it is, and a client that is let through is
//! shown by getting an answer that is not the gate's 403.

use http::Method;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::support::{Request, Server};

const UNSUPPORTED: &str = "unsupported-client";
const MISSING_VERSION: &str = "Ta aplikacja nie podaje swojej wersji i nie jest obsługiwana.";

/// A language the instance speaks that is not the default, so that the texts
/// tested are known not to be the ones a missing setting would give.
const POLISH: (&str, &str) = ("STELLARR_LANGUAGE", "pl");

async fn problem_of(response: crate::support::Response) -> Value {
    assert_eq!(response.status, 403, "{}", response.text());
    assert_eq!(
        response.header("content-type"),
        Some("application/problem+json")
    );
    response.json()
}

fn login() -> Request {
    Request::new(Method::POST, "/api/auth/login")
        .json(&json!({ "username": "maja", "password": "Aurora!2026" }))
}

#[tokio::test]
async fn a_client_that_does_not_identify_itself_cannot_sign_in() {
    let server = Server::start().await;

    let response = server.anonymous().send(login()).await;

    assert!(response.headers.get("set-cookie").is_none());
    let problem = problem_of(response).await;
    assert_eq!(problem["code"], UNSUPPORTED);
}

#[tokio::test]
async fn a_client_from_another_product_cannot_sign_in() {
    let server = Server::start().await;

    let response = server.client_of("voicephy", "9.9.9").send(login()).await;

    assert_eq!(response.status, 403);
}

#[tokio::test]
async fn a_desktop_build_below_the_minimum_is_refused_and_told_the_floor() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "0.1.3")]).await;

    let response = server
        .client_of("desktop-linux", "0.1.1")
        .send(login())
        .await;

    let problem = problem_of(response).await;
    assert_eq!(problem["code"], "outdated-client");
    assert_eq!(problem["minimumVersion"], "0.1.3");
    assert_eq!(
        problem["title"],
        "This version of the app (0.1.1) is too old. 0.1.3 or newer is required."
    );
}

#[tokio::test]
async fn a_current_desktop_build_is_let_through() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "0.1.3")]).await;

    let response = server
        .client_of("desktop-linux", "0.1.3")
        .send(login())
        .await;

    assert_ne!(response.status, 403);
}

/// The paths of a sign-in and of the calls a session starts with.
#[tokio::test]
async fn unidentified_callers_are_refused_at_the_door_to_a_session() {
    let server = Server::start().await;
    let cases = [
        ("POST", "/api/auth/login"),
        ("POST", "/api/auth/register"),
        ("POST", "/api/auth/desktop/browser/start"),
        ("POST", "/api/auth/desktop/browser/exchange"),
        ("POST", "/api/auth/passkey/login"),
        ("GET", "/api/session"),
        ("GET", "/api/bootstrap"),
        ("GET", "/API/Session"),
    ];

    for (method, path) in cases {
        let mut request = Request::new(method.parse().unwrap(), path);
        if method == "POST" {
            request = request.json(&json!({}));
        }
        let response = server.anonymous().send(request).await;

        let problem = problem_of(response).await;
        assert_eq!(problem["code"], UNSUPPORTED, "{path}");
        assert_eq!(problem["downloadPath"], "/", "{path}");
        assert!(problem.get("minimumVersion").is_none(), "{path}");
    }
}

#[tokio::test]
async fn other_paths_need_no_identification() {
    let server = Server::start().await;

    for path in ["/health/live", "/api/auth/oidc", "/api/anything"] {
        assert_ne!(server.anonymous().get(path).await.status, 403, "{path}");
    }
}

#[tokio::test]
async fn supported_clients_are_admitted_whatever_their_letter_case() {
    let server = Server::start().await;

    for (kind, version) in [
        ("web", "2.0.0"),
        ("WEB", "2.0.0"),
        ("  web  ", " 2.0.0 "),
        ("cockpit", "0.1.0"),
        ("mobile-android", "1.4"),
        ("desktop-linux", "9.9.9.9"),
    ] {
        let response = server.client_of(kind, version).get("/api/session").await;

        assert_ne!(response.status, 403, "{kind} {version}");
    }
}

#[tokio::test]
async fn a_client_without_a_readable_version_is_refused() {
    let server = Server::start_with(&[POLISH]).await;

    for version in ["", "two", "2", "2.0.0-beta"] {
        let response = server.client_of("web", version).get("/api/session").await;

        let problem = problem_of(response).await;
        assert_eq!(problem["code"], UNSUPPORTED, "{version:?}");
        assert_eq!(problem["title"], MISSING_VERSION, "{version:?}");
    }
}

#[tokio::test]
async fn the_refusal_is_in_the_language_of_the_instance() {
    let english = Server::start().await;
    let polish = Server::start_with(&[POLISH]).await;

    let in_english = problem_of(english.anonymous().get("/api/session").await).await;
    let in_polish = problem_of(polish.anonymous().get("/api/session").await).await;

    assert_eq!(
        in_english["title"],
        "This app is no longer supported. Download the current Stellarr client."
    );
    assert_eq!(
        in_polish["title"],
        "Ta aplikacja nie jest już obsługiwana. Pobierz aktualnego klienta Stellarr."
    );
}

#[tokio::test]
async fn the_clients_an_operator_lists_are_admitted_next_to_the_default_ones() {
    let server = Server::start_with(&[
        ("Clients__Allowed__0", "web"),
        ("Clients__Allowed__1", "kiosk"),
    ])
    .await;

    for (kind, version, admitted) in [
        ("kiosk", "1.0.0", true),
        ("web", "2.0.0", true),
        ("mobile-android", "1.0.0", true),
        ("voicephy", "9.9.9", false),
    ] {
        let response = server.client_of(kind, version).get("/api/session").await;

        assert_eq!(response.status != 403, admitted, "{kind}");
    }
}

#[tokio::test]
async fn the_refusal_names_the_configured_download_page_or_none() {
    let with_page = Server::start_with(&[("Clients__DownloadPath", "/pobierz")]).await;
    let without_page = Server::start_with(&[("Clients__DownloadPath", "")]).await;

    let named = problem_of(with_page.anonymous().get("/api/session").await).await;
    let silent = problem_of(without_page.anonymous().get("/api/session").await).await;

    assert_eq!(named["downloadPath"], "/pobierz");
    assert!(silent.get("downloadPath").is_none());
}

/// An instance that wants to be reachable by script can say so.
#[tokio::test]
async fn identification_can_be_turned_off_for_an_instance() {
    let server = Server::start_with(&[("Clients__RequireIdentification", "false")]).await;

    let anonymous = server.anonymous().send(login()).await;
    let foreign = server.client_of("voicephy", "9.9.9").send(login()).await;

    assert_ne!(anonymous.status, 403);
    // Naming another product is still refused.
    assert_eq!(foreign.status, 403);
}

#[tokio::test]
async fn only_the_desktop_client_has_a_floor() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "5.0.0")]).await;

    assert_ne!(
        server
            .client_of("web", "2.0.0")
            .get("/api/session")
            .await
            .status,
        403
    );
    assert_eq!(
        server
            .client_of("desktop-linux", "4.9.9")
            .get("/api/session")
            .await
            .status,
        403
    );
}

#[tokio::test]
async fn a_version_with_fewer_numbers_ranks_below_the_floor_with_more() {
    let server = Server::start().await;

    // The floor is 0.1.0, and 0.1 is older than that.
    let response = server
        .client_of("desktop-linux", "0.1")
        .get("/api/session")
        .await;

    let problem = problem_of(response).await;
    assert_eq!(problem["code"], "outdated-client");
    assert_eq!(problem["minimumVersion"], "0.1.0");
}

fn insert_floor(server: &Server, floor: &str) {
    let connection = Connection::open(server.database_path()).unwrap();
    connection
        .execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
             VALUES ('3D82DA44-8AA5-4BDF-9EE1-2902DF99880C', 'Stellarr', '2026-11-24 00:33:18', 1, 1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\",
                 \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
                 \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"DesktopMinimumVersion\")
             VALUES ('3D82DA44-8AA5-4BDF-9EE1-2902DF99880C', '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, '2026-11-24 00:33:18', ?1)",
            [floor],
        )
        .unwrap();
}

#[tokio::test]
async fn a_floor_saved_by_the_operator_overrides_the_configured_one() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "0.1.0")]).await;
    insert_floor(&server, " 0.3.0 ");

    let behind = server
        .client_of("desktop-linux", "0.2.9")
        .get("/api/session")
        .await;
    let current = server
        .client_of("desktop-linux", "0.3.0")
        .get("/api/session")
        .await;

    let problem = problem_of(behind).await;
    assert_eq!(problem["code"], "outdated-client");
    assert_eq!(problem["minimumVersion"], "0.3.0");
    assert_eq!(
        problem["title"],
        "This version of the app (0.2.9) is too old. 0.3.0 or newer is required."
    );
    assert_ne!(current.status, 403);
}

#[tokio::test]
async fn an_unreadable_floor_falls_back_to_the_configured_one_instead_of_refusing_everyone() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "0.1.3")]).await;
    Connection::open(server.database_path())
        .unwrap()
        .execute("DROP TABLE \"OperationsSettings\"", [])
        .unwrap();

    let behind = server
        .client_of("desktop-linux", "0.1.1")
        .get("/api/session")
        .await;
    let current = server
        .client_of("desktop-linux", "0.1.3")
        .get("/api/session")
        .await;

    let problem = problem_of(behind).await;
    assert_eq!(problem["minimumVersion"], "0.1.3");
    assert_ne!(current.status, 403);
}

#[tokio::test]
async fn a_saved_floor_that_is_not_a_version_means_there_is_no_floor() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "0.1.3")]).await;
    insert_floor(&server, "later");

    let behind = server
        .client_of("desktop-linux", "0.1.1")
        .get("/api/session")
        .await;

    assert_ne!(behind.status, 403);
}

//! Personal sessions are account-scoped even when both users launch one release.
use crate::support::{Person, Request, Response, Server};
use http::Method;
use rusqlite::params;
use serde_json::{Value, json};

const ORIGIN: &str = "https://chat.test";
const INSTALL: &str = "9f77e982-01b8-4934-ada0-1fb14cc3dd71";
const VOICE_ONLY: &str = "18f8c6a1-26ea-471b-a6db-eb11c353095c";
const FRAME: &str = "05245c20-5a7a-4464-a422-595ff87e9e84";

async fn post(person: &Person, path: &str, data: Value) -> Response {
    person
        .client
        .send(
            Request::new(Method::POST, path)
                .header("Origin", ORIGIN)
                .header("X-Forwarded-Host", "chat.test")
                .header("X-Forwarded-Proto", "https")
                .json(&data),
        )
        .await
}
fn seed(server: &Server, id: &str, personal: bool) {
    let db = server.database();
    let mut manifest = json!({"id":if personal{"personal-demo"}else{"voice-demo"},"name":"Demo","version":"1.0.0","description":"Board","protocol":1});
    if personal {
        manifest["launchModes"] = json!(["voice", "personal"]);
    }
    let digest = "b".repeat(64);
    db.execute("INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled) VALUES(?1,'https://github.com/test/demo',?2,?3,1)",
        params![id.to_uppercase(),manifest["id"].as_str().unwrap(),digest]).unwrap();
    db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?1,?2,'commit','release',?3,200)",
        params![id.to_uppercase(),digest,manifest.to_string()]).unwrap();
}
async fn launch(who: &Person, id: &str) -> Response {
    post(
        who,
        "/api/activities/personal/launch",
        json!({"install":id,"device":"desktop"}),
    )
    .await
}
async fn room(who: &Person, token: &Value, mut action: Value) -> Response {
    action["frame"] = json!(FRAME);
    post(
        who,
        "/api/activities/personal/room",
        json!({"install":INSTALL,"access":token,"action":action}),
    )
    .await
}

#[tokio::test]
async fn independent_authenticated_personal_sessions_enforce_launch_modes_tokens_and_revocation() {
    let server = Server::start_with(&[
        ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
        ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
    ])
    .await;
    let alice = server.found("owner").await;
    let bob = server.register("visitor").await;
    seed(&server, INSTALL, true);
    seed(&server, VOICE_ONLY, false);
    let list = post(
        &bob,
        "/api/activities/personal/list",
        json!({"device":"desktop"}),
    )
    .await;
    assert_eq!(list.status, 200, "{}", list.text());
    let entries = list.json::<Value>();
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["install"], INSTALL);
    assert_eq!(launch(&bob, VOICE_ONLY).await.status, 403);
    let first = launch(&alice, INSTALL).await;
    let second = launch(&bob, INSTALL).await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert_eq!(second.status, 200, "{}", second.text());
    let a = first.json::<Value>();
    let b = second.json::<Value>();
    let open = json!({"op":"open","initialState":{"score":0}});
    let a_open = room(&alice, &a["access"], open.clone())
        .await
        .json::<Value>();
    let b_open = room(&bob, &b["access"], open.clone()).await.json::<Value>();
    assert_ne!(a_open["sessionId"], b_open["sessionId"]);
    assert_eq!(a_open["launchMode"], "personal");
    assert!(a_open["place"].is_null());
    assert_eq!(a_open["standalone"], false);
    assert_eq!(b_open["participants"].as_array().unwrap().len(), 1);
    assert_eq!(
        room(
            &alice,
            &a["access"],
            json!({"op":"write","sequence":1,"patch":{"score":8}})
        )
        .await
        .status,
        200
    );
    let snapshot = room(&bob, &b["access"], json!({"op":"read","after":-1}))
        .await
        .json::<Value>();
    assert_eq!(snapshot["changes"][0]["state"]["score"], 0);
    assert_eq!(
        room(&bob, &a["access"], json!({"op":"read","after":-1}))
            .await
            .status,
        403
    );
    let as_channel = format!("/api/activities/{}/launch", alice.id);
    assert_eq!(
        post(
            &alice,
            &as_channel,
            json!({"install":INSTALL,"device":"desktop"})
        )
        .await
        .status,
        403
    );
    let manage = format!("/api/cockpit/activities/{INSTALL}/manage");
    assert_eq!(
        post(&alice, &manage, json!({"action":"disable"}))
            .await
            .status,
        200
    );
    assert_eq!(
        room(&bob, &b["access"], json!({"op":"read","after":-1}))
            .await
            .status,
        410
    );
    assert_eq!(
        post(&alice, &manage, json!({"action":"enable"}))
            .await
            .status,
        200
    );
    let resumed = launch(&alice, INSTALL).await.json::<Value>();
    let fresh = room(&alice, &resumed["access"], open).await.json::<Value>();
    assert_ne!(a_open["sessionId"], fresh["sessionId"]);
    assert_eq!(fresh["changes"][0]["state"]["score"], 0);
    // A newly approved, voice-only release cannot change an existing personal
    // room's grants. After the update only the account with that pin may resume.
    {
        let db = server.database();
        let manifest = json!({"id":"personal-demo","name":"Demo","version":"2.0.0","description":"New voice version","protocol":1,"launchModes":["voice"]});
        let digest = "c".repeat(64);
        db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?1,?2,'commit2','release2',?3,201)",
            params![INSTALL.to_uppercase(),digest,manifest.to_string()]).unwrap();
        db.execute(
            "UPDATE ActivityInstalls SET Digest=?2 WHERE Id=?1",
            params![INSTALL.to_uppercase(), digest],
        )
        .unwrap();
    }
    let pinned = post(
        &alice,
        "/api/activities/personal/list",
        json!({"device":"desktop"}),
    )
    .await
    .json::<Value>();
    assert_eq!(pinned[0]["version"], "1.0.0");
    assert_eq!(launch(&alice, INSTALL).await.status, 200);
    assert_eq!(launch(&bob, INSTALL).await.status, 403);
    assert_eq!(
        post(
            &bob,
            "/api/activities/personal/list",
            json!({"device":"desktop"})
        )
        .await
        .json::<Value>(),
        json!([])
    );
}

#[tokio::test]
async fn root_relative_assets_follow_the_origin_of_each_pinned_build() {
    let server = Server::start_with(&[
        ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
        ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
    ])
    .await;
    let owner = server.found("owner").await;
    let visitor = server.register("visitor").await;
    seed(&server, INSTALL, true);

    let old_digest = "b".repeat(64);
    let new_digest = "c".repeat(64);
    let web_files = |installation: &str, digest: &str, version: &str| {
        let db = server.database();
        for (path, body) in [
            ("index.html", "<link rel='stylesheet' href='/assets/game.css'><script type='module' src='/assets/main.js'></script>".to_owned()),
            ("assets/game.css", format!("/* {version} */")),
            ("assets/main.js", "import('/assets/lazy.js')".to_owned()),
            ("assets/lazy.js", format!("export default '{version}'")),
        ] {
            db.execute("INSERT INTO ActivityFiles(Install,Digest,Path,Content) VALUES(?1,?2,?3,?4)",
                params![installation.to_uppercase(),digest,path,body.as_bytes()]).unwrap();
        }
    };
    web_files(INSTALL, &old_digest, "first");
    let frame = |response: Response| {
        assert_eq!(response.status, 200, "{}", response.text());
        let json = response.json::<Value>();
        let (host, ticket) = json["src"]
            .as_str()
            .unwrap()
            .strip_prefix("https://")
            .unwrap()
            .split_once('/')
            .unwrap();
        let label = host.strip_suffix(".games.test").unwrap();
        assert!(label.len() <= 63 && !label.contains('.'));
        (host.to_owned(), format!("/{ticket}"))
    };
    // The wildcard vhost forwards every path into this private API namespace,
    // retaining the public Host. Each request supplies explicit browser cookies.
    let get = |host: &str, path: &str, cookie: &str| {
        let addr = server.address();
        let request = format!(
            "GET /_activity-files{path} HTTP/1.1\r\nHost: {host}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
        );
        async move { crate::support::raw_request(addr, request.as_bytes()).await }
    };

    let (first_host, first_ticket) = frame(launch(&owner, INSTALL).await);
    let start = get(&first_host, &first_ticket, "").await;
    assert_eq!(start.status, 302, "{}", start.text());
    assert_eq!(start.header("location"), Some("/index.html"));
    let set_cookie = start.header("set-cookie").unwrap();
    assert!(set_cookie.contains("Path=/; Secure; HttpOnly; SameSite=None; Partitioned"));
    assert!(!set_cookie.to_lowercase().contains("domain="));
    let first_cookie = set_cookie.split(';').next().unwrap();
    assert_eq!(get(&first_host, &first_ticket, "").await.status, 403);

    {
        let db = server.database();
        let manifest = json!({"id":"personal-demo","name":"Demo","version":"2.0.0","description":"Game","protocol":1,"launchModes":["personal"]});
        db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?1,?2,'commit2','release2',?3,201)",
            params![INSTALL.to_uppercase(),new_digest,manifest.to_string()]).unwrap();
        db.execute(
            "UPDATE ActivityInstalls SET Digest=?2 WHERE Id=?1",
            params![INSTALL.to_uppercase(), new_digest],
        )
        .unwrap();
        // A second source distributes exactly the same ZIP and declares the
        // same activity ID. It receives an independent installation origin.
        db.execute("INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled) VALUES(?1,'https://github.com/other/demo','personal-demo',?2,1)",
            params![VOICE_ONLY.to_uppercase(),new_digest]).unwrap();
        db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?1,?2,'commit2','release2',?3,201)",
            params![VOICE_ONLY.to_uppercase(),new_digest,manifest.to_string()]).unwrap();
    }
    web_files(INSTALL, &new_digest, "second");
    web_files(VOICE_ONLY, &new_digest, "second");
    let (second_host, second_ticket) = frame(launch(&visitor, INSTALL).await);
    let (other_host, other_ticket) = frame(launch(&visitor, VOICE_ONLY).await);
    assert_ne!(first_host, second_host);
    assert_ne!(second_host, other_host);
    assert_eq!(get(&first_host, &second_ticket, "").await.status, 403);
    let start = get(&second_host, &second_ticket, "").await;
    assert_eq!(start.status, 302);
    let second_cookie = start
        .header("set-cookie")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let other = get(&other_host, &other_ticket, "").await;
    assert_eq!(other.status, 302);

    for (host, cookie, version) in [
        (&first_host, first_cookie, "first"),
        (&second_host, second_cookie, "second"),
    ] {
        for page in ["/", "/index.html"] {
            let index = get(host, page, cookie).await;
            assert_eq!(index.status, 200, "{}", index.text());
            assert!(index.text().contains("src='/assets/main.js'"));
        }
        let css = get(host, "/assets/game.css", cookie).await;
        assert_eq!(css.header("content-type"), Some("text/css"));
        assert_eq!(css.text(), format!("/* {version} */"));
        let main = get(host, "/assets/main.js", cookie).await;
        assert_eq!(main.header("content-type"), Some("text/javascript"));
        assert_eq!(main.text(), "import('/assets/lazy.js')");
        assert_eq!(
            get(host, "/assets/lazy.js", cookie).await.text(),
            format!("export default '{version}'")
        );
        assert_eq!(get(host, "/_stellarr/sdk/v1.js", cookie).await.status, 200);
        assert_eq!(get(host, "/assets/missing.js", cookie).await.status, 404);
        for path in [
            "/",
            "/index.html",
            "/assets/lazy.js",
            "/_stellarr/sdk/v1.js",
        ] {
            assert_eq!(get(host, path, "").await.status, 403);
        }
    }
    for (host, cookie) in [
        (&first_host, second_cookie),
        (&second_host, first_cookie),
        (&other_host, second_cookie),
    ] {
        assert_eq!(get(host, "/assets/lazy.js", cookie).await.status, 403);
    }
    assert_eq!(
        post(
            &owner,
            &format!("/api/cockpit/activities/{INSTALL}/manage"),
            json!({"action":"disable"})
        )
        .await
        .status,
        200
    );
    assert_eq!(
        get(&first_host, "/assets/lazy.js", first_cookie)
            .await
            .status,
        403
    );
    assert_eq!(
        get(&second_host, "/assets/lazy.js", second_cookie)
            .await
            .status,
        403
    );
    let other_cookie = other
        .header("set-cookie")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    assert_eq!(
        get(&other_host, "/assets/lazy.js", other_cookie)
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn a_static_activity_checks_its_own_launch_even_after_the_installation_is_enabled_again() {
    let server = Server::start_with(&[
        ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
        ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
    ])
    .await;
    let owner = server.found("owner").await;
    let other = server.register("viewer").await;
    seed(&server, INSTALL, true);
    let list = "/api/activities/personal/list";
    let manage = format!("/api/cockpit/activities/{INSTALL}/manage");
    let heartbeat = |token: &Value| json!({"device": "desktop", "lease": {"install": INSTALL, "access": token}});

    // This page never opens the SDK; only the surrounding picker makes requests.
    let first = launch(&owner, INSTALL).await.json::<Value>();
    let original = heartbeat(&first["access"]);
    assert_eq!(post(&owner, list, original.clone()).await.status, 200);
    assert_eq!(post(&other, list, original.clone()).await.status, 410);
    assert_eq!(
        post(&owner, &manage, json!({"action":"disable"}))
            .await
            .status,
        200
    );
    assert_eq!(
        post(&owner, &manage, json!({"action":"enable"}))
            .await
            .status,
        200
    );

    let next = launch(&owner, INSTALL).await.json::<Value>();
    let listed = post(&owner, list, json!({"device":"desktop"}))
        .await
        .json::<Value>();
    assert_eq!(listed[0]["running"], true);
    assert_eq!(post(&owner, list, original).await.status, 410);
    assert_eq!(
        post(&owner, list, heartbeat(&next["access"])).await.status,
        200
    );
}

//! Personal sessions are account-scoped even when both users launch one release.
//! A voice channel runs one activity at a time, hosted by whoever launched it.
use crate::support::{Person, Request, Response, Server};
use crate::voice::{Peer, Rig, VIEW_CHANNEL};
use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Id;

const ORIGIN: &str = "https://chat.test";
const ACTIVITIES: &[(&str, &str)] = &[
    ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
    ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
];
const INSTALL: &str = "9f77e982-01b8-4934-ada0-1fb14cc3dd71";
const VOICE_ONLY: &str = "18f8c6a1-26ea-471b-a6db-eb11c353095c";
const FRAME: &str = "05245c20-5a7a-4464-a422-595ff87e9e84";

/// A call from the web client, which names its origin.
fn request(path: &str) -> Request {
    Request::new(Method::POST, path)
        .header("Origin", ORIGIN)
        .header("X-Forwarded-Host", "chat.test")
        .header("X-Forwarded-Proto", "https")
}
async fn post(person: &Person, path: &str, data: Value) -> Response {
    person.client.send(request(path).json(&data)).await
}
async fn get(person: &Person, path: &str) -> Response {
    person.client.send(Request::new(Method::GET, path)).await
}
fn seed(server: &Server, id: &str, personal: bool) {
    let mut manifest = json!({"id":if personal{"personal-demo"}else{"voice-demo"},"name":if personal{"Demo"}else{"Duel"},"version":"1.0.0","description":"Board","protocol":1});
    if personal {
        manifest["launchModes"] = json!(["voice", "personal"]);
    }
    install(server, id, &manifest);
}
/// An installation of a release that this manifest describes.
fn install(server: &Server, id: &str, manifest: &Value) {
    let db = server.database();
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
/// A refusal the client explains by its code.
#[track_caller]
fn refused(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "{}", response.text());
    assert_eq!(response.json::<Value>()["code"], code, "{}", response.text());
}
/// The voice rig with an activity domain.
async fn voice_rig() -> Rig {
    Rig::start_with(ACTIVITIES).await
}
/// Starts, joins or swaps the activity of a voice channel.
async fn launch_in(who: &Person, channel: Id, body: Value) -> Response {
    post(who, &format!("/api/activities/{channel}/launch"), body).await
}
/// Ends the activity of a voice channel, with no body, as the client does.
async fn end_in(who: &Person, channel: Id) -> Response {
    who.client
        .send(request(&format!("/api/activities/{channel}/end")))
        .await
}
/// The next announcement of what runs in the channel that `accept` takes.
async fn announced(peer: &mut Peer, channel: Id, accept: impl Fn(&Value) -> bool) -> Value {
    let channel = channel.to_string();
    peer.next_where("VoiceActivityChanged", |d| {
        d["channelId"] == channel.as_str() && accept(&d["activity"])
    })
    .await["activity"]
        .clone()
}

async fn browser(rig: &Rig, person: &Person) -> Peer {
    use crate::support::{HubClient, upgrade_with};
    let cookie = format!(
        "stellarr-dev-session={}",
        person.client.cookie("stellarr-dev-session").unwrap()
    );
    let upgraded = upgrade_with(
        rig.server.address(),
        "",
        Some(&cookie),
        "chat.test",
        &[
            ("Origin", ORIGIN),
            ("X-Forwarded-Proto", "https"),
            ("Sec-WebSocket-Protocol", "stellarr.v1"),
        ],
    )
    .await;
    Peer {
        hub: HubClient::from_upgrade(upgraded).await,
    }
}

#[tokio::test]
async fn activity_cameras_share_only_the_activity_sfu_room_and_lose_access_on_departure() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    let guest = rig.person("guest").await;
    install(
        &rig.server,
        INSTALL,
        &json!({"id":"charades","name":"Charades","description":"Game",
        "version":"1.0.0","protocol":1,"permissions":["media.cameraTile"]}),
    );
    let mut alice_ws = browser(&rig, &rig.owner).await;
    let mut bob_ws = browser(&rig, &bob).await;
    let mut guest_ws = browser(&rig, &guest).await;
    for peer in [&mut alice_ws, &mut bob_ws, &mut guest_ws] {
        peer.join(rig.salon).await;
    }
    let call = alice_ws.open_media(rig.salon).await;
    let mut launches = Vec::new();
    for person in [&rig.owner, &bob] {
        let reply = launch_in(
            person,
            rig.salon,
            json!({"install":INSTALL, "device":"desktop"}),
        )
        .await;
        launches.push(reply.json::<Value>()["access"].clone());
    }
    let camera = |index: usize, action: Value| {
        json!([rig.salon,{
            "install":INSTALL, "access":launches[index], "action":action
        }])
    };
    let open = json!({"op":"open"});
    assert_eq!(
        guest_ws.ok("ActivityCamera", camera(0, open.clone())).await["status"],
        403
    );
    let alice = alice_ws.ok("ActivityCamera", camera(0, open.clone())).await["value"].clone();
    let bob_cam = bob_ws.ok("ActivityCamera", camera(1, open.clone())).await["value"].clone();
    let room = Id::parse(alice["channelId"].as_str().unwrap()).unwrap();
    let id = alice["sessionId"].as_str().unwrap();
    let viewer = bob_cam["sessionId"].as_str().unwrap();
    assert_eq!(bob_cam["channelId"], json!(room));
    assert_ne!(room, rig.salon);
    assert_eq!(
        rig.media.session_of(id),
        Some((room.to_string(), rig.owner.id.clone()))
    );
    assert_eq!(
        rig.media.session_of(viewer),
        Some((room.to_string(), bob.id.clone()))
    );
    guest_ws
        .refused(
            "OpenMediaSession",
            json!([room]),
            "Not seated in this voice room.",
        )
        .await;
    let subscriptions = r#"{"mids":[]}"#;
    guest_ws
        .media_refused(
            room,
            id,
            "subscriptions",
            subscriptions,
            "Invalid media signal.",
        )
        .await;
    alice_ws
        .media(room, id, "subscriptions", subscriptions)
        .await;
    bob_ws
        .media(room, viewer, "subscriptions", subscriptions)
        .await;
    alice_ws
        .media(rig.salon, &call, "subscriptions", subscriptions)
        .await;

    rig.media.signal(viewer, "tracks", r#"{"tracks":[]}"#);
    let event = bob_ws
        .next_where("MediaSignal", |event| event["sessionId"] == viewer)
        .await;
    assert_eq!(event["channelId"], json!(room));
    assert!(guest_ws.drain("MediaSignal").await.is_empty());

    let leave = format!("/api/activities/{}/leave", rig.salon);
    assert_eq!(
        post(
            &bob,
            &leave,
            json!({"install":INSTALL,"access":launches[1]})
        )
        .await
        .status,
        200
    );
    rig.media.delete_of(viewer).await;
    bob_ws
        .media_refused(
            room,
            viewer,
            "subscriptions",
            subscriptions,
            "Invalid media signal.",
        )
        .await;
    assert_eq!(
        bob_ws.ok("ActivityCamera", camera(1, open)).await["status"],
        403
    );
    alice_ws
        .media(room, id, "subscriptions", subscriptions)
        .await;
    assert_eq!(end_in(&rig.owner, rig.salon).await.status, 200);
    rig.media.delete_of(id).await;
    alice_ws
        .media(rig.salon, &call, "subscriptions", subscriptions)
        .await;
}

#[tokio::test]
async fn room_socket_delivers_to_the_subscribed_connection_and_wakes_host_commands() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    install(
        &rig.server,
        INSTALL,
        &json!({"id":"cards","name":"Cards","description":"Table",
        "version":"1.0.0","protocol":1,"authority":"host","seats":2}),
    );
    let mut alice_ws = browser(&rig, &rig.owner).await;
    let mut bob_ws = browser(&rig, &bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;
    let salon = rig.salon.to_string();
    let path = format!("/api/activities/{salon}/room");
    let open = json!({"op":"open","frame":FRAME,"initialState":{}});
    let mut grants = Vec::new();
    for person in [&rig.owner, &bob] {
        let launched = launch_in(
            person,
            rig.salon,
            json!({"device":"desktop","install":INSTALL}),
        )
        .await
        .json::<Value>();
        let grant = json!({"install":INSTALL,"access":launched["access"]});
        let mut body = grant.clone();
        body["action"] = open.clone();
        let opened = post(person, &path, body).await.json::<Value>();
        assert_eq!(opened["roomHub"], true);
        assert!(opened.get("roomEvents").is_none());
        grants.push(grant);
    }
    let args = |who: usize, mut action: Value| {
        let mut body = grants[who].clone();
        action["frame"] = json!(FRAME);
        body["action"] = action;
        json!([salon, body])
    };
    let denied = alice_ws.ok("ActivityCamera", args(0, json!({"op":"open"}))).await;
    assert_eq!(denied["status"], 403, "the manifest only requested room access");
    let read = json!({"op":"read","after":0,"roomEvents":true});
    assert_eq!(
        bob_ws.ok("ActivityRoom", args(0, read.clone())).await["status"],
        403
    );
    let alice = alice_ws.ok("ActivityRoom", args(0, read.clone())).await;
    assert_eq!(alice["value"]["host"], json!(rig.owner.id));
    bob_ws.ok("ActivityRoom", args(1, read.clone())).await;
    let send = json!({"op":"send","to":bob.id,"kind":"cursor","data":17});
    let sent = alice_ws.ok("ActivityRoom", args(0, send)).await;
    assert_eq!(sent, json!({"value":{"accepted":true}}));
    let event = bob_ws
        .next_where("ActivityRoomUpdate", |e| e["message"]["kind"] == "cursor")
        .await;
    assert_eq!(event["message"]["from"], json!(rig.owner.id));
    assert_eq!(event["message"]["data"], 17);
    assert_eq!(
        bob_ws.ok("ActivityRoom", args(1, read.clone())).await["value"]["messages"],
        json!([])
    );

    // A legacy HTTP submitter can wake a host on the hub. A second submit
    // returns the cached outcome, also after switching transports.
    let submit = json!({"op":"submit","kind":"guess","id":"turn","data":42});
    assert_eq!(
        post(&bob, &path, args(1, submit.clone())[1].clone())
            .await
            .json::<Value>()["pending"],
        true
    );
    let wake = alice_ws.next_where("ActivityRoomUpdate", |_| true).await;
    assert_eq!(wake, json!({"frame":FRAME}));
    let packet = alice_ws.ok("ActivityRoom", args(0, read.clone())).await;
    assert_eq!(packet["value"]["command"]["id"], "turn");
    let decision = json!({"op":"decide","from":bob.id,"id":"turn","epoch":packet["value"]["epoch"],
        "expected":0,"patch":{"score":1},"assignments":{bob.id.as_str():"secret"},"result":"yes"});
    assert_eq!(
        alice_ws.ok("ActivityRoom", args(0, decision)).await["value"]["revision"],
        1
    );
    let settled = bob_ws
        .next_where("ActivityRoomUpdate", |e| e["settled"] == "turn")
        .await;
    assert_eq!(settled, json!({"frame":FRAME,"settled":"turn"}));
    assert_eq!(
        bob_ws.ok("ActivityRoom", args(1, submit)).await["value"]["result"],
        "yes"
    );
    assert_eq!(
        bob_ws.ok("ActivityRoom", args(1, read)).await["value"]["assignment"],
        "secret"
    );

    let mut resumed = browser(&rig, &bob).await;
    let catch_up = resumed
        .ok(
            "ActivityRoom",
            args(1, json!({"op":"read","after":0,"roomEvents":true})),
        )
        .await;
    assert_eq!(catch_up["value"]["changes"][0]["state"]["score"], 1);
    alice_ws
        .ok(
            "ActivityRoom",
            args(
                0,
                json!({"op":"send","to":bob.id,"kind":"after-reconnect","data":1}),
            ),
        )
        .await;
    resumed
        .next_where("ActivityRoomUpdate", |e| {
            e["message"]["kind"] == "after-reconnect"
        })
        .await;
    let retired = bob_ws
        .hub
        .quiet_for(std::time::Duration::from_millis(30))
        .await;
    assert!(
        !retired
            .iter()
            .any(|e| e["data"]["message"]["kind"] == "after-reconnect")
    );
}

#[tokio::test]
async fn independent_authenticated_personal_sessions_enforce_launch_modes_tokens_and_revocation() {
    let server = Server::start_with(ACTIVITIES).await;
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
    let entries = list.json::<Value>()["activities"].clone();
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
    assert_eq!(pinned["activities"][0]["version"], "1.0.0");
    assert_eq!(launch(&alice, INSTALL).await.status, 200);
    assert_eq!(launch(&bob, INSTALL).await.status, 403);
    assert_eq!(
        post(
            &bob,
            "/api/activities/personal/list",
            json!({"device":"desktop"})
        )
        .await
        .json::<Value>()["activities"],
        json!([])
    );
}

#[tokio::test]
async fn root_relative_assets_follow_the_origin_of_each_pinned_build() {
    let server = Server::start_with(ACTIVITIES).await;
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
    let server = Server::start_with(ACTIVITIES).await;
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
    assert_eq!(listed["activities"][0]["running"], true);
    assert_eq!(post(&owner, list, original).await.status, 410);
    assert_eq!(
        post(&owner, list, heartbeat(&next["access"])).await.status,
        200
    );
}

#[tokio::test]
async fn voice_channel_runs_at_most_one_activity_joins_directly_and_clears_when_last_person_leaves()
{
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    let carol = rig.person("carol").await;
    seed(&rig.server, INSTALL, true);
    seed(&rig.server, VOICE_ONLY, false);

    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    let mut carol_ws = rig.connect(&carol).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;

    let salon = rig.salon.to_string();
    let launch_path = format!("/api/activities/{salon}/launch");
    let leave_path = format!("/api/activities/{salon}/leave");

    // Idle voice channel has nothing to join without an install id.
    assert_eq!(
        post(&rig.owner, &launch_path, json!({"device": "desktop"}))
            .await
            .status,
        404
    );

    // Starting the first activity announces it over realtime to everyone in the room.
    let alice_launch = post(
        &rig.owner,
        &launch_path,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(alice_launch.status, 200, "{}", alice_launch.text());
    let alice_session = alice_launch.json::<Value>();
    assert_eq!(alice_session["install"], INSTALL);

    let started = bob_ws
        .next_where("VoiceActivityChanged", |d| {
            d["channelId"] == salon.as_str() && !d["activity"].is_null()
        })
        .await;
    assert_eq!(started["activity"]["install"], INSTALL);
    assert_eq!(started["activity"]["name"], "Demo");

    // A second, different activity on the same voice channel is refused while one runs.
    refused(
        &post(
            &bob,
            &launch_path,
            json!({"install": VOICE_ONLY, "device": "desktop"}),
        )
        .await,
        409,
        "activity-running",
    );

    // Joining from the voice bar without specifying `install` joins the running channel activity.
    let bob_launch = post(&bob, &launch_path, json!({"device": "desktop"})).await;
    assert_eq!(bob_launch.status, 200, "{}", bob_launch.text());
    let bob_session = bob_launch.json::<Value>();
    assert_eq!(bob_session["install"], INSTALL);

    let room_path = format!("/api/activities/{salon}/room");
    let op = |session: &Value, mut action: Value| {
        action["frame"] = json!(FRAME);
        json!({"install":INSTALL, "access":session["access"], "action":action})
    };
    let open = json!({"op":"open", "initialState":{}});
    assert_eq!(
        post(&rig.owner, &room_path, op(&alice_session, open.clone()))
            .await
            .status,
        200
    );
    let peers = post(&bob, &room_path, op(&bob_session, open.clone()))
        .await
        .json::<Value>();
    assert_eq!(peers["participants"][0]["username"], rig.owner.username);
    let personal = launch(&rig.owner, INSTALL).await.json::<Value>();
    let access = &personal["access"];
    assert_eq!(room(&rig.owner, access, open).await.status, 200);
    let voice_save = op(
        &alice_session,
        json!({"op":"saveProgress", "expected":0, "data":{"mode":"voice"}}),
    );
    let solo_save = json!({"op":"saveProgress", "expected":0, "data":{"mode":"solo"}});
    let (v, s) = tokio::join!(
        post(&rig.owner, &room_path, voice_save),
        room(&rig.owner, access, solo_save)
    );
    let mut statuses = [v.status.as_u16(), s.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let load = json!({"op":"loadProgress"});
    let shared = post(&rig.owner, &room_path, op(&alice_session, load.clone()))
        .await
        .json::<Value>();
    assert_eq!(shared["revision"], 1);
    assert_eq!(
        shared,
        room(&rig.owner, access, load.clone()).await.json::<Value>()
    );
    let visitor = post(&bob, &room_path, op(&bob_session, load))
        .await
        .json::<Value>();
    assert!(visitor["data"].is_null());

    // Someone joining the voice channel later is immediately told about the live activity.
    carol_ws.join(rig.salon).await;
    let announced = carol_ws
        .next_where("VoiceActivityChanged", |d| {
            d["channelId"] == salon.as_str() && !d["activity"].is_null()
        })
        .await;
    assert_eq!(announced["activity"]["install"], INSTALL);

    // When Alice leaves the activity, Bob still holds a lease so the session stays running.
    let alice_left = post(
        &rig.owner,
        &leave_path,
        json!({"install": INSTALL, "access": alice_session["access"]}),
    )
    .await;
    assert_eq!(alice_left.status, 200);
    assert_eq!(alice_left.json::<Value>()["running"], true);

    // When Bob (the last participant) leaves, the room closes at once and announces `activity: null`.
    let bob_left = post(
        &bob,
        &leave_path,
        json!({"install": INSTALL, "access": bob_session["access"]}),
    )
    .await;
    assert_eq!(bob_left.status, 200);
    assert_eq!(bob_left.json::<Value>()["running"], false);

    let cleared = alice_ws
        .next_where("VoiceActivityChanged", |d| {
            d["channelId"] == salon.as_str() && d["activity"].is_null()
        })
        .await;
    assert!(cleared["activity"].is_null());

    // With the channel idle again, the other activity can now be launched.
    assert_eq!(
        post(
            &bob,
            &launch_path,
            json!({"install": VOICE_ONLY, "device": "desktop"})
        )
        .await
        .status,
        200
    );
}

#[tokio::test]
async fn installed_packages_share_the_instance_address_with_isolated_revocable_assets() {
    let server = crate::uploads::started(&[]).await;
    let owner = server.found("owner").await;
    install(&server, INSTALL, &json!({
        "id": "personal-demo", "name": "Demo", "version": "1.0.0", "description": "Board", "protocol": 1,
        "launchModes": ["personal"], "connects": [
            {"host":"media.example","why":"Player"}, {"host":"chat.test","why":"Instance API"}
        ]
    }));
    server
        .database()
        .execute(
            "INSERT INTO ActivityFiles(Install,Digest,Path,Content) VALUES(?1,?2,'index.html',?3)",
            params![
                INSTALL.to_uppercase(),
                "b".repeat(64),
                b"<p>Custom activity</p>".as_slice()
            ],
        )
        .unwrap();
    let launched = launch(&owner, INSTALL).await;
    assert_eq!(launched.status, 200, "{}", launched.text());
    let body = launched.json::<Value>();
    let path = body["src"].as_str().unwrap().strip_prefix(ORIGIN).unwrap();
    assert!(path.starts_with("/_activity/") && path.ends_with("/index.html"));
    assert!(!path.contains(body["access"].as_str().unwrap()));
    assert_eq!(body["embedHosts"], json!(["media.example", "chat.test"]));
    let browser = server.anonymous();
    let shell_path = path.replace("/index.html", "/_stellarr/embed.html");
    let shell = browser.get(&shell_path).await;
    assert_eq!(shell.status, 200);
    assert!(shell.text().contains("/_stellarr/sdk/embed-frame.js"));
    let shell_policy = shell.header("content-security-policy").unwrap();
    assert!(shell_policy.contains("default-src 'none'"));
    assert!(shell_policy.contains("frame-src https://media.example;"));
    let asset = browser.get(path).await;
    assert_eq!(asset.status, 200, "{}", asset.text());
    assert_eq!(asset.text(), "<p>Custom activity</p>");
    let policy = asset.header("content-security-policy").unwrap();
    assert!(policy.contains("sandbox allow-scripts allow-forms allow-presentation allow-downloads;"));
    assert!(policy.contains("frame-ancestors https://chat.test;"));
    assert_eq!(asset.header("access-control-allow-origin"), Some("*"));
    assert_eq!(asset.header("referrer-policy"), Some("strict-origin"));
    assert_eq!(asset.header("cache-control"), Some("no-store"));
    assert!(asset.header("set-cookie").is_none());
    let sdk = browser.get("/_stellarr/sdk/v1.js").await;
    assert_eq!(sdk.status, 200);
    assert_eq!(sdk.header("access-control-allow-origin"), Some("*"));
    let photo = owner
        .client
        .send(crate::uploads::image("face.png").post("/api/me/images/avatar"))
        .await;
    assert_eq!(photo.status, 201);
    let opened = room(
        &owner,
        &body["access"],
        json!({"op":"open","initialState":{}}),
    )
    .await
    .json::<Value>();
    let portrait = opened["me"]["avatar"].as_str().unwrap();
    let image_url = format!("{}{}", path.strip_suffix("/index.html").unwrap(), portrait);
    assert_eq!(
        browser.get(&image_url).await.body.as_ref() as &[u8],
        crate::uploads::PNG
    );
    let saved = room(
        &owner,
        &body["access"],
        json!({"op":"saveProgress","expected":0,"data":{"level":3}}),
    )
    .await;
    assert_eq!(saved.status, 200);
    assert_eq!(
        room(&owner, &body["access"], json!({"op":"loadProgress"}))
            .await
            .json::<Value>()["data"]["level"],
        3
    );
    let disabled = post(
        &owner,
        &format!("/api/cockpit/activities/{INSTALL}/manage"),
        json!({"action":"disable"}),
    )
    .await;
    assert_eq!(disabled.status, 200);
    assert_eq!(browser.get(path).await.status, 403);
    assert_eq!(browser.get(&image_url).await.status, 403);
    assert_eq!(browser.get(&shell_path).await.status, 403);
}

#[tokio::test]
async fn private_progress_survives_a_release_and_server_restart_on_a_second_device() {
    let folder = crate::support::TempDir::new();
    let env = [
        ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
        ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
    ];
    let server = Server::start_in(folder.path(), &env).await;
    let mut alice = server.found("owner").await;
    let bob = server.register("visitor").await;
    seed(&server, INSTALL, true);
    let phone = post(
        &alice,
        "/api/activities/personal/launch",
        json!({"install":INSTALL,"device":"phone"}),
    )
    .await
    .json::<Value>();
    let access = &phone["access"];
    let open = json!({"op":"open","initialState":{}});
    let load = json!({"op":"loadProgress"});
    assert_eq!(room(&alice, access, load.clone()).await.status, 403);
    let first = room(&alice, access, open.clone()).await.json::<Value>();
    assert_eq!(first["progress"], true);
    let save = |level| json!({"op":"saveProgress","expected":0,"data":{"level":level}});
    let (one, two) = tokio::join!(room(&alice, access, save(7)), room(&alice, access, save(9)));
    let (level, other) = if one.status == 200 {
        (7, two)
    } else {
        assert_eq!(two.status, 200);
        (9, one)
    };
    assert_eq!(other.status, 409);
    assert_eq!(
        room(&alice, access, save(level)).await.json::<Value>()["revision"],
        1
    );
    assert_eq!(
        room(
            &alice,
            access,
            json!({"op":"saveProgress","expected":1,"data":{"x":"a".repeat(65536)}})
        )
        .await
        .status,
        413
    );
    let bob_launch = launch(&bob, INSTALL).await.json::<Value>();
    let bob_open = room(&bob, &bob_launch["access"], open.clone())
        .await
        .json::<Value>();
    assert!(bob_open.get("data").is_none());
    assert_eq!(
        room(&bob, &bob_launch["access"], load.clone())
            .await
            .json::<Value>(),
        json!({"revision":0,"data":null})
    );
    assert_eq!(room(&bob, access, load.clone()).await.status, 403);

    let second = "c".repeat(64);
    {
        let db = server.database();
        db.execute("INSERT INTO ActivityVersions SELECT Install,?2,'v2','release2',json_set(Manifest,'$.version','2.0.0'),Bytes FROM ActivityVersions WHERE Install=?1", params![INSTALL.to_uppercase(), second]).unwrap();
        db.execute(
            "UPDATE ActivityInstalls SET Digest=?2 WHERE Id=?1",
            params![INSTALL.to_uppercase(), second],
        )
        .unwrap();
    }
    assert!(server.stop().await);
    let server = Server::start_in(folder.path(), &env).await;
    alice.client = server.sign_in(&alice).await;
    let desktop = launch(&alice, INSTALL).await.json::<Value>();
    assert_ne!(
        phone["src"].as_str().unwrap().split('/').nth(2),
        desktop["src"].as_str().unwrap().split('/').nth(2)
    );
    let opened = room(&alice, &desktop["access"], open).await.json::<Value>();
    assert_ne!(opened["sessionId"], first["sessionId"]);
    assert_eq!(opened["me"]["id"], first["me"]["id"]);
    let saved = json!({"revision":1,"data":{"level":level}});
    assert_eq!(
        room(&alice, &desktop["access"], load.clone())
            .await
            .json::<Value>(),
        saved
    );
    assert_eq!(
        post(
            &alice,
            &format!("/api/cockpit/activities/{INSTALL}/manage"),
            json!({"action":"uninstall"})
        )
        .await
        .status,
        200
    );
    seed(&server, VOICE_ONLY, true);
    let reinstalled = launch(&alice, VOICE_ONLY).await.json::<Value>();
    let in_reinstall = |action: Value| json!({"install":VOICE_ONLY,"access":reinstalled["access"],"action":action});
    let route = "/api/activities/personal/room";
    assert_eq!(
        post(
            &alice,
            route,
            in_reinstall(json!({"op":"open","initialState":{},"frame":FRAME}))
        )
        .await
        .status,
        200
    );
    assert_eq!(
        post(
            &alice,
            route,
            in_reinstall(json!({"op":"loadProgress","frame":FRAME}))
        )
        .await
        .json::<Value>(),
        saved
    );
    assert_eq!(
        post(
            &alice,
            &format!("/api/cockpit/activities/{VOICE_ONLY}/manage"),
            json!({"action":"uninstall"})
        )
        .await
        .status,
        200
    );
    seed(&server, INSTALL, true);
    server
        .database()
        .execute(
            "UPDATE ActivityInstalls SET Source='https://github.com/unrelated/demo' WHERE Id=?1",
            [INSTALL.to_uppercase()],
        )
        .unwrap();
    let unrelated = launch(&alice, INSTALL).await.json::<Value>();
    assert_eq!(
        room(
            &alice,
            &unrelated["access"],
            json!({"op":"open","initialState":{}})
        )
        .await
        .status,
        200
    );
    assert_eq!(
        room(&alice, &unrelated["access"], load)
            .await
            .json::<Value>(),
        json!({"revision":0,"data":null})
    );
}

#[tokio::test]
async fn activity_profiles_refresh_and_the_frame_can_load_a_members_current_avatar() {
    let server = crate::uploads::started(&[
        ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
        ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
    ])
    .await;
    let alice = server.found("owner").await;
    seed(&server, INSTALL, true);
    let launch = launch(&alice, INSTALL).await.json::<Value>();
    let access = &launch["access"];
    let initial = room(&alice, access, json!({"op":"open","initialState":{}}))
        .await
        .json::<Value>();
    assert!(initial["me"]["avatar"].is_null());
    let photo = alice
        .client
        .send(crate::uploads::image("face.png").post("/api/me/images/avatar"))
        .await;
    assert_eq!(photo.status, 201, "{}", photo.text());
    assert_eq!(
        alice
            .client
            .send(Request::new(Method::PATCH, "/api/me").json(&json!({"displayName":"New Name"})))
            .await
            .status,
        200
    );
    let latest = room(&alice, access, json!({"op":"read"}))
        .await
        .json::<Value>();
    let player = &latest["participants"][0];
    assert_eq!(player["name"], "New Name");
    let avatar = player["avatar"].as_str().unwrap();
    let src = launch["src"]
        .as_str()
        .unwrap()
        .strip_prefix("https://")
        .unwrap();
    let (host, start) = src.split_once('/').unwrap();
    let get = |path: &str, cookie: &str| {
        let request = format!(
            "GET /_activity-files/{path} HTTP/1.1\r\nHost: {host}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
        );
        let address = server.address();
        async move { crate::support::raw_request(address, request.as_bytes()).await }
    };
    let started = get(start, "").await;
    let cookie = started
        .header("set-cookie")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let path = avatar.trim_start_matches('/');
    let picture = get(path, cookie).await;
    assert_eq!(picture.status, 200);
    assert_eq!(picture.header("content-type"), Some("image/png"));
    assert_eq!(picture.body.as_slice(), crate::uploads::PNG);
    assert_eq!(get(path, "").await.status, 403);
    assert_eq!(
        alice
            .client
            .send(Request::new(Method::DELETE, "/api/me/images/avatar"))
            .await
            .status,
        200
    );
    let removed = room(&alice, access, json!({"op":"read"}))
        .await
        .json::<Value>();
    assert!(removed["participants"][0]["avatar"].is_null());
    assert_eq!(get(path, cookie).await.status, 403);
}

#[tokio::test]
async fn the_picker_names_the_owner_the_permissions_and_what_runs_only_in_voice() {
    let server = Server::start_with(ACTIVITIES).await;
    let owner = server.found("owner").await;
    let visitor = server.register("visitor").await;
    install(
        &server,
        INSTALL,
        &json!({"id":"personal-demo","name":"Demo","version":"1.0.0","description":"Board","protocol":1,
            "launchModes":["personal"],"permissions":["media.microphoneLevel"]}),
    );
    seed(&server, VOICE_ONLY, false);

    let listed = post(
        &visitor,
        "/api/activities/personal/list",
        json!({"device":"desktop"}),
    )
    .await;
    assert_eq!(listed.status, 200, "{}", listed.text());
    let listed = listed.json::<Value>();
    assert_eq!(
        listed["activities"][0]["permissions"],
        json!(["media.microphoneLevel"])
    );
    assert_eq!(listed["voiceOnly"], json!(["Duel"]));
    assert!(listed["live"].is_null());
    assert_eq!(
        listed["owner"],
        json!({"id": owner.id, "name": "Owner", "avatar": null})
    );
    assert_eq!(listed["viewerIsOwner"], false);
    let own = post(
        &owner,
        "/api/activities/personal/list",
        json!({"device":"desktop"}),
    )
    .await
    .json::<Value>();
    assert_eq!(own["viewerIsOwner"], true);
}

#[tokio::test]
async fn a_release_refuses_a_device_it_does_not_run_on() {
    let server = Server::start_with(ACTIVITIES).await;
    let owner = server.found("owner").await;
    install(
        &server,
        INSTALL,
        &json!({"id":"personal-demo","name":"Demo","version":"1.0.0","description":"Board","protocol":1,
            "launchModes":["personal"],"devices":["desktop"]}),
    );

    let phone = post(
        &owner,
        "/api/activities/personal/launch",
        json!({"install": INSTALL, "device": "phone"}),
    )
    .await;
    refused(&phone, 422, "unsupported-device");
}

#[tokio::test]
async fn whoever_launches_a_voice_activity_hosts_it_and_the_picker_shows_it_running() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;

    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());
    assert_eq!(launched.json::<Value>()["host"], rig.owner.id.as_str());

    let listed = post(
        &bob,
        &format!("/api/activities/{}/list", rig.salon),
        json!({"device": "desktop"}),
    )
    .await
    .json::<Value>();
    assert_eq!(listed["live"]["install"], INSTALL);
    assert_eq!(listed["live"]["host"], rig.owner.id.as_str());
}

#[tokio::test]
async fn members_who_see_the_channel_hear_of_its_activity_without_sitting_in_it() {
    let rig = voice_rig().await;
    let carol = rig.person("carol").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut carol_ws = rig.connect(&carol).await;
    alice_ws.join(rig.salon).await;

    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());

    let activity = announced(&mut carol_ws, rig.salon, |a| !a.is_null()).await;
    assert_eq!(
        activity,
        json!({"install": INSTALL, "name": "Demo", "icon": null, "host": rig.owner.id,
            "participants": [rig.owner.id], "seats": null})
    );
}

#[tokio::test]
async fn hosting_passes_to_whoever_has_been_inside_longest_when_the_host_leaves_the_activity() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    let carol = rig.person("carol").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    let mut carol_ws = rig.connect(&carol).await;
    for ws in [&mut alice_ws, &mut bob_ws, &mut carol_ws] {
        ws.join(rig.salon).await;
    }

    let alice = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await
    .json::<Value>();
    for who in [&bob, &carol] {
        let joined = launch_in(who, rig.salon, json!({"device": "desktop"})).await;
        assert_eq!(joined.status, 200, "{}", joined.text());
    }
    let left = post(
        &rig.owner,
        &format!("/api/activities/{}/leave", rig.salon),
        json!({"install": INSTALL, "access": alice["access"]}),
    )
    .await;
    assert_eq!(left.status, 200, "{}", left.text());

    let activity = announced(&mut bob_ws, rig.salon, |a| a["host"] == bob.id.as_str()).await;
    assert_eq!(activity["participants"], json!([bob.id, carol.id]));
}

#[tokio::test]
async fn hosting_passes_on_when_the_host_leaves_the_voice_channel() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;
    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());
    let joined = launch_in(&bob, rig.salon, json!({"device": "desktop"})).await;
    assert_eq!(joined.status, 200, "{}", joined.text());

    // The host's lease goes with the seat, at once rather than when it lapses.
    alice_ws.leave().await;

    let activity = announced(&mut bob_ws, rig.salon, |a| a["host"] == bob.id.as_str()).await;
    assert_eq!(activity["participants"], json!([bob.id]));
}

#[tokio::test]
async fn the_host_ends_the_activity_for_everyone_in_it() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;
    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());
    let joined = launch_in(&bob, rig.salon, json!({"device": "desktop"})).await;
    assert_eq!(joined.status, 200, "{}", joined.text());

    let ended = end_in(&rig.owner, rig.salon).await;
    assert_eq!(ended.status, 200, "{}", ended.text());
    assert_eq!(ended.json::<Value>(), json!({"ok": true}));
    announced(&mut bob_ws, rig.salon, Value::is_null).await;
}

#[tokio::test]
async fn only_the_host_ends_an_activity_and_only_one_that_runs() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;

    refused(&end_in(&rig.owner, rig.salon).await, 404, "not-found");
    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());
    refused(&end_in(&bob, rig.salon).await, 403, "not-host");
}

#[tokio::test]
async fn somebody_who_does_not_host_cannot_swap_the_activity() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    seed(&rig.server, INSTALL, true);
    seed(&rig.server, VOICE_ONLY, false);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;
    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());

    let swap = launch_in(
        &bob,
        rig.salon,
        json!({"install": VOICE_ONLY, "device": "desktop", "replace": true}),
    )
    .await;
    refused(&swap, 403, "not-host");
}

#[tokio::test]
async fn the_host_swaps_the_activity_for_everyone_and_hosts_the_next() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    seed(&rig.server, INSTALL, true);
    seed(&rig.server, VOICE_ONLY, false);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;
    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": INSTALL, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());
    let joined = launch_in(&bob, rig.salon, json!({"device": "desktop"})).await;
    assert_eq!(joined.status, 200, "{}", joined.text());

    let swapped = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": VOICE_ONLY, "device": "desktop", "replace": true}),
    )
    .await;
    assert_eq!(swapped.status, 200, "{}", swapped.text());
    assert_eq!(swapped.json::<Value>()["install"], VOICE_ONLY);

    // Those who were in the old activity hear it end, then what runs instead.
    announced(&mut bob_ws, rig.salon, Value::is_null).await;
    let next = announced(&mut bob_ws, rig.salon, |_| true).await;
    assert_eq!(next["install"], VOICE_ONLY);
    assert_eq!(next["participants"], json!([rig.owner.id]));
}

#[tokio::test]
async fn a_full_activity_turns_the_next_person_away() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    install(
        &rig.server,
        VOICE_ONLY,
        &json!({"id":"voice-demo","name":"Duel","version":"1.0.0","description":"Board","protocol":1,"seats":1}),
    );
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut bob_ws = rig.connect(&bob).await;
    alice_ws.join(rig.salon).await;
    bob_ws.join(rig.salon).await;
    let launched = launch_in(
        &rig.owner,
        rig.salon,
        json!({"install": VOICE_ONLY, "device": "desktop"}),
    )
    .await;
    assert_eq!(launched.status, 200, "{}", launched.text());

    refused(
        &launch_in(&bob, rig.salon, json!({"device": "desktop"})).await,
        409,
        "full",
    );
}

#[tokio::test]
async fn the_channel_list_learns_what_runs_in_the_voice_channels_a_member_sees() {
    let rig = voice_rig().await;
    let bob = rig.person("bob").await;
    let carol = rig.person("carol").await;
    seed(&rig.server, INSTALL, true);
    let mut alice_ws = rig.connect(&rig.owner).await;
    let mut carol_ws = rig.connect(&carol).await;
    alice_ws.join(rig.night).await;
    carol_ws.join(rig.salon).await;
    for (who, room) in [(&rig.owner, rig.night), (&carol, rig.salon)] {
        let launched = launch_in(who, room, json!({"install": INSTALL, "device": "desktop"})).await;
        assert_eq!(launched.status, 200, "{}", launched.text());
    }
    // Bob sees the salon, but no longer the night shift.
    rig.channel_override(rig.base_role(), rig.night, 0, VIEW_CHANNEL);

    let snapshot = get(&bob, &format!("/api/activities/live?server={}", rig.server_id)).await;
    assert_eq!(snapshot.status, 200, "{}", snapshot.text());
    let snapshot = snapshot.json::<Value>();
    assert_eq!(snapshot.as_array().map(Vec::len), Some(1), "{snapshot}");
    assert_eq!(snapshot[0]["channelId"], rig.salon.to_string().as_str());
    assert_eq!(snapshot[0]["activity"]["host"], carol.id.as_str());
    let elsewhere = get(&bob, &format!("/api/activities/live?server={}", Id::random())).await;
    assert_eq!(elsewhere.status, 403);
}

//! Voice rooms through the real binary: who may sit in one, what the room is
//! told, direct calls, the soundboard, and a moderator's hand on a seat.
//!
//! A media server stands behind the binary, played by [`FakeMedia`]; the tests
//! of the media sessions themselves are in `voice_media.rs`, and share what is
//! here.

use std::time::Duration;

use http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{FakeMedia, HubClient, Person, Request, SECRET, Server};

const COOKIE: &str = "stellarr-dev-session";
pub const QUIET: Duration = Duration::from_millis(300);

/// The binary with its media server, and the rows a test of voice stands on.
pub struct Rig {
    pub server: Server,
    pub media: FakeMedia,
    pub owner: Person,
    pub server_id: Id,
    /// The voice channel the instance starts with.
    pub salon: Id,
    /// A second voice channel.
    pub night: Id,
    /// The text channel the instance starts with.
    pub general: Id,
}

impl Rig {
    pub async fn start() -> Rig {
        Rig::start_with(&[]).await
    }

    /// Starts the binary with the media server behind it and waits for the
    /// event pump to have reset the media server, which is when voice works.
    pub async fn start_with(extra: &[(&str, &str)]) -> Rig {
        let media = FakeMedia::start().await;
        Rig::start_behind(media, extra, true).await
    }

    /// The same behind a media server that is already there, which need not be
    /// ready: `wait` says whether to wait for the pump.
    pub async fn start_behind(media: FakeMedia, extra: &[(&str, &str)], wait: bool) -> Rig {
        let url = media.url();
        let mut variables = vec![
            ("Rtc__MediaControlUrl", url.as_str()),
            ("Rtc__MediaControlSecret", SECRET),
            ("Testing__Workers", "voice-media-events,voice-media-closer,realtime-sessions"),
            ("Testing__MediaCommandTimeoutMs", "3000"),
            ("Testing__MediaEventWaitMs", "250"),
            ("Testing__MediaRetryPauseMs", "100"),
        ];
        variables.extend_from_slice(extra);
        let server = Server::start_with(&variables).await;
        let owner = server.found("owner").await;
        let db = server.database();
        let server_id: Id = db
            .query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |row| row.get(0))
            .unwrap();
        let channel = |kind: i64| -> Id {
            db.query_row(
                "SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = ?1 ORDER BY \"Position\" LIMIT 1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
        };
        let (general, salon) = (channel(0), channel(1));
        let night = add_channel(&db, server_id, "night-shift", 1, 0);
        drop(db);
        if wait {
            media.count_reaches("GET", "/v1/events", 1).await;
        }
        Rig {
            server,
            media,
            owner,
            server_id,
            salon,
            night,
            general,
        }
    }

    pub fn db(&self) -> Connection {
        self.server.database()
    }

    /// A person of the instance's server.
    pub async fn person(&self, name: &str) -> Person {
        self.server.register(name).await
    }

    pub async fn connect(&self, person: &Person) -> Peer {
        self.connect_to("", person).await
    }

    pub async fn connect_to(&self, query: &str, person: &Person) -> Peer {
        let cookie = format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"));
        let hub = HubClient::connect_to(self.server.address(), query, &cookie).await;
        let mut peer = Peer { hub };
        peer.settle().await;
        peer
    }

    /// The occupants the last snapshot of a room named, as account ids.
    pub fn occupants(&self, room: Id) -> Vec<String> {
        let room = room.to_string();
        let snapshot = self
            .server
            .events()
            .into_iter()
            .rev()
            .find(|event| {
                event["name"] == "VoiceParticipantChanged" && event["payload"]["channelId"] == room.as_str()
            })
            .map(|event| event["payload"].clone())
            .unwrap_or(Value::Null);
        snapshot["participants"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|p| p["userId"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Waits until the last snapshot of a room names exactly these people.
    pub async fn occupants_become(&self, room: Id, expected: &[&str]) {
        let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        want.sort();
        for _ in 0..200 {
            let mut have = self.occupants(room);
            have.sort();
            if have == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the room never held {want:?}; it holds {:?}", self.occupants(room));
    }

    /// Who is moderating: an account with a role of its own that holds what it is given,
    /// ranked above the base role.
    pub fn give_role(&self, user: &Person, name: &str, position: i64, permissions: i64) -> Id {
        // A lower position ranks higher, and the base role starts at 1, so it is
        // moved out of the way of the ranks a test wants to give.
        self.db()
            .execute(
                "UPDATE \"ServerRoles\" SET \"Position\" = 100 WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0",
                [self.server_id],
            )
            .unwrap();
        give_role(&self.db(), self.server_id, &user.id, name, position, permissions)
    }

    /// Takes a permission away from the base role everybody has.
    pub fn base_role_loses(&self, permission: i64) {
        self.db()
            .execute(
                "UPDATE \"ServerRoles\" SET \"Permissions\" = \"Permissions\" & ~?1 WHERE \"ServerId\" = ?2 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0",
                params![permission, self.server_id],
            )
            .unwrap();
    }

    pub fn base_role(&self) -> Id {
        self.db()
            .query_row(
                "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0 ORDER BY \"Position\" DESC LIMIT 1",
                [self.server_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// An override of a role at a channel.
    pub fn channel_override(&self, role: Id, channel: Id, allow: i64, deny: i64) {
        self.db()
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6)",
                params![Id::random(), self.server_id, channel, role, allow, deny],
            )
            .unwrap();
    }

    /// A direct conversation between two people.
    pub fn conversation(&self, first: &Person, second: &Person) -> Id {
        // Stored with the lower id first, as conversations are made.
        let (low, high) = {
            let (a, b) = (Id::parse(&first.id).unwrap(), Id::parse(&second.id).unwrap());
            if a < b { (a, b) } else { (b, a) }
        };
        let id = Id::random();
        self.db()
            .execute(
                "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\") VALUES (?1, ?2, ?3, ?4, 'Off')",
                params![id, low, high, Timestamp::now()],
            )
            .unwrap();
        id
    }

    pub fn block(&self, blocker: &Person, blocked: &Person) {
        self.db()
            .execute(
                "INSERT INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
                params![Id::parse(&blocker.id).unwrap(), Id::parse(&blocked.id).unwrap(), Timestamp::now()],
            )
            .unwrap();
    }

    pub fn befriend(&self, first: &Person, second: &Person) {
        let (a, b) = (Id::parse(&first.id).unwrap(), Id::parse(&second.id).unwrap());
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        self.db()
            .execute(
                "INSERT INTO \"Friendships\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
                params![Id::random(), a, b, Timestamp::now()],
            )
            .unwrap();
    }

    /// What a person takes: direct messages from anybody, or not.
    pub fn takes_direct_messages(&self, person: &Person, takes: bool) {
        set_preference(&self.db(), &person.id, "\"AllowDirectMessages\"", takes);
    }

    /// An HTTP call of a person, as the web client makes one.
    pub async fn call(&self, person: &Person, method: Method, path: &str, body: Option<Value>) -> crate::support::Response {
        let mut request = Request::new(method, path);
        if let Some(body) = body {
            request = request.json(&body);
        }
        person.client.send(request).await
    }
}

pub fn add_channel(db: &Connection, server: Id, name: &str, kind: i64, user_limit: i64) -> Id {
    let id = Id::random();
    db.execute(
        "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
         VALUES (?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(\"Position\"), 0) + 1 FROM \"Channels\" WHERE \"ServerId\" = ?2), '', 0, 0, ?5, NULL)",
        params![id, server, name, kind, user_limit],
    )
    .unwrap();
    id
}

pub fn give_role(db: &Connection, server: Id, user: &str, name: &str, position: i64, permissions: i64) -> Id {
    let role = Id::random();
    let user = Id::parse(user).unwrap();
    db.execute(
        "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#123456', ?4, ?5, 0, 0)",
        params![role, server, name, permissions, position],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"ServerMemberRoles\" (\"ServerId\", \"UserId\", \"RoleId\") VALUES (?1, ?2, ?3)",
        params![server, user, role],
    )
    .unwrap();
    db.execute(
        "UPDATE \"ServerMembers\" SET \"RoleId\" = ?1 WHERE \"ServerId\" = ?2 AND \"UserId\" = ?3",
        params![role, server, user],
    )
    .unwrap();
    role
}

/// Writes the preferences row of an account if it has none, and sets one flag.
pub fn set_preference(db: &Connection, user: &str, column: &str, value: bool) {
    let user = Id::parse(user).unwrap();
    db.execute(
        "INSERT OR IGNORE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
         VALUES (?1, 1, 1, 1, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 50)",
        [user],
    )
    .unwrap();
    db.execute(
        &format!("UPDATE \"UserPreferences\" SET {column} = ?1 WHERE \"UserId\" = ?2"),
        params![value, user],
    )
    .unwrap();
}

/// A hub connection of a person, with the calls a voice test makes.
pub struct Peer {
    pub hub: HubClient,
}

impl Peer {
    /// Calls a method and gives the completion.
    pub async fn call(&mut self, method: &str, arguments: Value) -> Value {
        self.hub.invoke(method, arguments).await
    }

    /// Calls a method that must succeed, and gives its result.
    pub async fn ok(&mut self, method: &str, arguments: Value) -> Value {
        let answer = self.call(method, arguments).await;
        assert!(answer.get("error").is_none(), "{method}: {answer}");
        answer["result"].clone()
    }

    /// Calls a method that must be refused with this text.
    pub async fn refused(&mut self, method: &str, arguments: Value, text: &str) {
        let answer = self.call(method, arguments).await;
        let error = answer["error"].as_str().unwrap_or_else(|| panic!("{method} was not refused: {answer}"));
        assert!(
            error.ends_with(&format!("HubException: {text}")),
            "{method}: {error}"
        );
    }

    /// Calls a method that must fail as a call whose arguments do not fit.
    pub async fn does_not_fit(&mut self, method: &str, arguments: Value) {
        let answer = self.call(method, arguments).await;
        assert_eq!(
            answer["error"],
            format!("Failed to invoke '{method}' due to an error on the server."),
            "{answer}"
        );
    }

    pub async fn join(&mut self, room: Id) {
        self.ok("JoinVoice", json!([room])).await;
    }

    pub async fn leave(&mut self) {
        self.ok("LeaveVoice", json!([])).await;
    }

    /// A state signal.
    pub async fn state(&mut self, room: Id, payload: &str, to: Option<&str>) -> Value {
        self.call("SendVoiceSignal", json!([signal(room, "state", payload, to)])).await
    }

    /// Asks for something and so waits until what the server sent before has
    /// arrived.
    pub async fn settle(&mut self) {
        self.ok("GetRtcClock", json!([])).await;
    }

    /// The next event of this name, with what it carries, past any others.
    pub async fn next(&mut self, name: &str) -> Value {
        self.next_where(name, |_| true).await
    }

    pub async fn next_where(&mut self, name: &str, accept: impl Fn(&Value) -> bool) -> Value {
        loop {
            let record = self.hub.record().await;
            if record["type"] == 1 && record["target"] == name && accept(&record["arguments"][0]) {
                return record["arguments"][0].clone();
            }
        }
    }

    /// Every event of this name that has reached the connection, once what was
    /// sent before has arrived.
    pub async fn drain(&mut self, name: &str) -> Vec<Value> {
        self.settle().await;
        self.hub
            .quiet_for(Duration::from_millis(30))
            .await
            .into_iter()
            .filter(|record| record["type"] == 1 && record["target"] == name)
            .map(|record| record["arguments"][0].clone())
            .collect()
    }
}

/// The argument of `SendVoiceSignal`.
pub fn signal(room: Id, kind: &str, payload: &str, to: Option<&str>) -> Value {
    json!({"protocolVersion": 1, "channelId": room, "toUserId": to, "kind": kind, "payload": payload})
}

pub fn in_room(room: Id) -> impl Fn(&Value) -> bool {
    let room = room.to_string();
    move |snapshot| snapshot["channelId"] == room.as_str()
}

/// Whether a snapshot says an account is muted; none when it is not in it.
pub fn muted_of(snapshot: &Value, user: &str) -> Option<bool> {
    snapshot["participants"]
        .as_array()?
        .iter()
        .find(|p| p["userId"] == user)
        .and_then(|p| p["muted"].as_bool())
}

pub fn occupants_of(snapshot: &Value) -> Vec<String> {
    snapshot["participants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["userId"].as_str().unwrap().to_string())
        .collect()
}

pub const CONNECT: i64 = 1 << 12;
pub const VIEW_CHANNEL: i64 = 1 << 14;
pub const MODERATE_MEMBERS: i64 = 1 << 5;
pub const ADMINISTRATOR: i64 = 1 << 8;
pub const MANAGE_SERVER: i64 = 1;
pub const NOT_ALLOWED: &str = "Not allowed to join this voice room.";

/// A second server with one voice channel and one member.
pub fn add_server(db: &Connection, member: &str) -> (Id, Id) {
    let server = Id::random();
    let role = Id::random();
    let user = Id::parse(member).unwrap();
    db.execute(
        "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"IconUrl\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Elsewhere', '', 0, NULL, ?2, 1, 1)",
        params![server, Timestamp::now()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, 'Member', '#777777', ?3, 1, 1, 0)",
        params![role, server, 0x7fff_i64 & !(1 << 8)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"OwnerSlot\", \"JoinedAt\") VALUES (?1, ?2, ?3, NULL, ?4)",
        params![server, user, role, Timestamp::now()],
    )
    .unwrap();
    let voice = add_channel(db, server, "elsewhere-voice", 1, 0);
    (server, voice)
}

#[tokio::test]
async fn joining_tells_the_server_who_is_in_the_call_but_nobody_else() {
    let rig = Rig::start().await;
    let (jo, nils, maja) = (rig.person("jonas").await, rig.person("nils").await, rig.person("maja").await);
    let (other, _) = add_server(&rig.db(), &nils.id);
    let mut observer = rig.connect(&jo).await;
    let mut elsewhere = rig.connect_to(&format!("?serverId={other}"), &nils).await;
    let mut maja_peer = rig.connect(&maja).await;

    maja_peer.join(rig.salon).await;
    let joined = observer.next_where("VoiceParticipantChanged", in_room(rig.salon)).await;
    maja_peer.leave().await;
    let left = observer.next_where("VoiceParticipantChanged", in_room(rig.salon)).await;

    assert_eq!(occupants_of(&joined), std::slice::from_ref(&maja.id));
    assert_eq!(joined["participants"][0]["displayName"], "Maja");
    assert_eq!(joined["participants"][0]["muted"], false);
    assert!(occupants_of(&left).is_empty());
    let (first, second) = (joined["sequence"].as_i64().unwrap(), left["sequence"].as_i64().unwrap());
    assert!(first > 0 && second > first);
    assert!(elsewhere.drain("VoiceParticipantChanged").await.is_empty());
}

#[tokio::test]
async fn joining_is_refused_for_rooms_the_account_cannot_use() {
    let rig = Rig::start().await;
    let (ilya, nils) = (rig.person("ilya").await, rig.person("nils").await);
    let (_, foreign) = add_server(&rig.db(), &nils.id);
    let mut peer = rig.connect(&ilya).await;

    for room in [rig.general, foreign, Id::random()] {
        peer.refused("JoinVoice", json!([room]), "Unknown voice room.").await;
    }
    peer.refused("JoinVoice", json!(["not-a-guid"]), "Invalid channelId.").await;
    peer.refused("JoinVoice", json!([null]), "Invalid channelId.").await;
    peer.does_not_fit("JoinVoice", json!([5])).await;
    peer.does_not_fit("JoinVoice", json!([])).await;
    peer.does_not_fit("JoinVoice", json!([rig.salon, rig.salon])).await;
    peer.does_not_fit("LeaveVoice", json!([1])).await;
    assert!(rig.server.events().iter().all(|e| e["name"] != "VoiceParticipantChanged"));
}

#[tokio::test]
async fn an_id_is_read_as_the_first_server_reads_one() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    let room = rig.salon.to_string();

    for spelling in [room.to_uppercase(), room.replace('-', ""), format!("{{{room}}}"), format!(" {room} ")] {
        peer.ok("JoinVoice", json!([spelling])).await;
    }
    rig.occupants_become(rig.salon, &[&maja.id]).await;
}

#[tokio::test]
async fn a_room_at_its_user_limit_refuses_newcomers_but_not_those_in_it() {
    let rig = Rig::start().await;
    rig.db()
        .execute("UPDATE \"Channels\" SET \"UserLimit\" = 1 WHERE \"Id\" = ?1", [rig.salon])
        .unwrap();
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);

    first.join(rig.salon).await;
    second.refused("JoinVoice", json!([rig.salon]), "Voice room is full.").await;
    first.join(rig.salon).await;
    rig.occupants_become(rig.salon, &[&maja.id]).await;

    first.leave().await;
    second.join(rig.salon).await;
    rig.occupants_become(rig.salon, &[&ada.id]).await;
}

#[tokio::test]
async fn a_newer_connection_of_the_same_account_replaces_the_older_one() {
    let rig = Rig::start().await;
    let (jo, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let mut observer = rig.connect(&jo).await;
    let mut older = rig.connect(&maja).await;
    let mut newer = rig.connect(&maja).await;

    older.join(rig.salon).await;
    newer.join(rig.salon).await;

    let notice = older.next("VoiceSessionReplaced").await;
    assert_eq!(notice, json!({"channelId": rig.salon}));
    older
        .refused("SendVoiceSignal", json!([signal(rig.salon, "state", "{\"muted\":false}", None)]), "Invalid voice signal.")
        .await;
    assert!(newer.drain("VoiceSessionReplaced").await.is_empty());
    observer.settle().await;
    rig.occupants_become(rig.salon, &[&maja.id]).await;
    let all = observer.drain("VoiceParticipantChanged").await;
    assert_eq!(occupants_of(all.last().unwrap()), std::slice::from_ref(&maja.id));
}

#[tokio::test]
async fn a_signal_reaches_only_its_target_and_a_broadcast_everyone_else_in_the_room() {
    let rig = Rig::start().await;
    let (maja, ada, ilya, jo) = (
        rig.person("maja").await,
        rig.person("ada").await,
        rig.person("ilya").await,
        rig.person("jonas").await,
    );
    let mut peers = vec![rig.connect(&maja).await, rig.connect(&ada).await, rig.connect(&ilya).await];
    let mut outside = rig.connect(&jo).await;
    for peer in &mut peers {
        peer.join(rig.salon).await;
    }
    let screen = "{\"screenStreamId\":\"screen-1\"}";

    peers[0].ok("SendVoiceSignal", json!([signal(rig.salon, "state", screen, Some(&ada.id))])).await;
    let targeted = peers[1].next("VoiceSignal").await;
    assert_eq!(
        targeted,
        json!({"protocolVersion": 1, "channelId": rig.salon, "fromUserId": maja.id, "toUserId": ada.id, "kind": "state", "payload": screen})
    );
    for index in [0, 2] {
        assert!(peers[index].drain("VoiceSignal").await.is_empty());
    }
    assert!(outside.drain("VoiceSignal").await.is_empty());

    peers[0].state(rig.salon, "{\"screenStreamId\":null}", None).await;
    for index in [1, 2] {
        let broadcast = peers[index].next("VoiceSignal").await;
        assert_eq!(broadcast["kind"], "state");
        assert_eq!(broadcast["toUserId"], Value::Null);
    }
    assert!(peers[0].drain("VoiceSignal").await.is_empty());
    assert!(outside.drain("VoiceSignal").await.is_empty());
}

#[tokio::test]
async fn an_invalid_signal_is_refused_and_reaches_nobody() {
    let rig = Rig::start().await;
    let (maja, ada, jo) = (rig.person("maja").await, rig.person("ada").await, rig.person("jonas").await);
    let (mut sender, mut receiver, mut outside) =
        (rig.connect(&maja).await, rig.connect(&ada).await, rig.connect(&jo).await);
    sender.join(rig.salon).await;
    receiver.join(rig.salon).await;
    let too_long = format!("{{\"pad\":\"{}\"}}", "x".repeat(128 * 1024 - 10 + 1));

    let refused = "Invalid voice signal.";
    for (kind, payload) in [("hello", "{}"), ("offer", "{}"), ("STATE", "{}")] {
        sender.refused("SendVoiceSignal", json!([signal(rig.salon, kind, payload, None)]), refused).await;
    }
    let mut wrong_version = signal(rig.salon, "state", "{}", None);
    wrong_version["protocolVersion"] = json!(2);
    sender.refused("SendVoiceSignal", json!([wrong_version]), refused).await;
    sender.refused("SendVoiceSignal", json!([signal(rig.salon, "state", &too_long, None)]), refused).await;
    sender.refused("SendVoiceSignal", json!([signal(rig.night, "state", "{}", None)]), refused).await;
    outside.refused("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", None)]), refused).await;
    sender.refused("SendVoiceSignal", json!([null]), refused).await;
    sender
        .refused("SendVoiceSignal", json!([signal(Id::random(), "state", "{}", None)]), refused)
        .await;
    sender
        .refused("SendVoiceSignal", json!([{"protocolVersion": 1, "channelId": "not-a-guid", "kind": "state", "payload": "{}"}]), "Invalid ChannelId.")
        .await;
    sender
        .refused("SendVoiceSignal", json!([{"protocolVersion": 1, "kind": "state", "payload": "{}"}]), "Invalid ChannelId.")
        .await;
    for target in [Some(jo.id.as_str()), Some("nobody")] {
        sender
            .refused("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", target)]), "Voice signal target is not in this channel.")
            .await;
    }
    for payload in ["{not json", "[true]", "", "null", "\"x\""] {
        sender.refused("SendVoiceSignal", json!([signal(rig.salon, "state", payload, None)]), "Invalid voice state.").await;
    }
    sender
        .refused("SendVoiceSignal", json!([{"protocolVersion": 1, "channelId": rig.salon, "kind": "state"}]), refused)
        .await;
    for bad in [
        json!(["x"]),
        json!([5]),
        json!([[]]),
        json!([]),
        json!([signal(rig.salon, "state", "{}", None), 1]),
        json!([{"protocolVersion": "1", "channelId": rig.salon, "kind": "state", "payload": "{}"}]),
        json!([{"protocolVersion": null, "channelId": rig.salon, "kind": "state", "payload": "{}"}]),
        json!([{"protocolVersion": 1.5, "channelId": rig.salon, "kind": "state", "payload": "{}"}]),
        json!([{"protocolVersion": 1, "channelId": 5, "kind": "state", "payload": "{}"}]),
        json!([{"protocolVersion": 1, "channelId": rig.salon, "kind": 5, "payload": "{}"}]),
        json!([{"protocolVersion": 1, "channelId": rig.salon, "kind": "state", "payload": {"muted": true}}]),
        json!([{"protocolVersion": 1, "channelId": rig.salon, "toUserId": 5, "kind": "state", "payload": "{}"}]),
    ] {
        sender.does_not_fit("SendVoiceSignal", bad).await;
    }

    assert!(receiver.drain("VoiceSignal").await.is_empty());
    assert!(outside.drain("VoiceSignal").await.is_empty());

    // A refusal leaves the connection and its seat as they were.
    sender.state(rig.salon, "{}", None).await;
    assert_eq!(receiver.next("VoiceSignal").await["kind"], "state");
}

#[tokio::test]
async fn a_signal_is_read_in_the_way_of_the_first_servers_reader() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut sender, mut receiver) = (rig.connect(&maja).await, rig.connect(&ada).await);
    sender.join(rig.salon).await;
    receiver.join(rig.salon).await;

    // Members are found in any letter case, and what is not there is nothing.
    sender
        .ok("SendVoiceSignal", json!([{"PROTOCOLVERSION": 1, "ChannelId": rig.salon, "KIND": "state", "payload": "{\"muted\":true}"}]))
        .await;
    assert_eq!(receiver.next("VoiceSignal").await["payload"], "{\"muted\":true}");
    // The last of two members of one name counts, and a document nests 64 deep.
    for (payload, muted) in [
        ("{\"muted\":true,\"muted\":false}", Some(false)),
        ("{\"muted\":false,\"muted\":true}", Some(true)),
        ("{\"Muted\":false}", None),
        ("{\"muted\":null}", None),
        ("{\"muted\":\"yes\"}", None),
    ] {
        sender.state(rig.salon, payload, None).await;
        // The room hears of a change of seat before the signal is relayed.
        if let Some(muted) = muted {
            receiver
                .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && muted_of(s, &maja.id) == Some(muted))
                .await;
        }
        let relayed = receiver.next("VoiceSignal").await;
        assert_eq!(relayed["payload"], payload);
    }
    let nested = |depth: usize| format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
    sender.state(rig.salon, &nested(64), None).await;
    assert_eq!(receiver.next("VoiceSignal").await["kind"], "state");
    sender
        .refused("SendVoiceSignal", json!([signal(rig.salon, "state", &nested(65), None)]), "Invalid voice state.")
        .await;
    // Sending to oneself is sending to a connection of the room like any other.
    sender.ok("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", Some(&maja.id))])).await;
    assert_eq!(sender.next("VoiceSignal").await["toUserId"], maja.id.as_str());
}

#[tokio::test]
async fn a_payload_of_exactly_the_limit_is_relayed() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut sender, mut receiver) = (rig.connect(&maja).await, rig.connect(&ada).await);
    sender.join(rig.salon).await;
    receiver.join(rig.salon).await;
    let payload = format!("{{\"pad\":\"{}\"}}", "x".repeat(128 * 1024 - 10));
    assert_eq!(payload.len(), 128 * 1024);

    sender.state(rig.salon, &payload, None).await;

    assert_eq!(receiver.next("VoiceSignal").await["payload"].as_str().unwrap().len(), 128 * 1024);
}

#[tokio::test]
async fn a_state_signal_with_muted_changes_the_occupancy_others_see() {
    let rig = Rig::start().await;
    let (jo, maja, ada) = (rig.person("jonas").await, rig.person("maja").await, rig.person("ada").await);
    let mut observer = rig.connect(&jo).await;
    let (mut sender, mut receiver) = (rig.connect(&maja).await, rig.connect(&ada).await);
    sender.join(rig.salon).await;
    receiver.join(rig.salon).await;
    let muted_by = |who: String| move |s: &Value| s["channelId"].is_string() && s["participants"].as_array().unwrap().iter().any(|p| p["userId"] == who.as_str() && p["muted"] == true);

    let muted = "{\"muted\":true,\"screenStreamId\":null}";
    sender.state(rig.salon, muted, None).await;
    observer.next_where("VoiceParticipantChanged", muted_by(maja.id.clone())).await;
    assert_eq!(receiver.next("VoiceSignal").await["payload"], muted);

    sender.state(rig.salon, "{\"muted\":false}", None).await;
    observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && !muted_by(maja.id.clone())(s))
        .await;
    assert_eq!(receiver.next("VoiceSignal").await["payload"], "{\"muted\":false}");

    // A state that says nothing about muting is relayed and changes no seat.
    observer.settle().await;
    observer.drain("VoiceParticipantChanged").await;
    sender.state(rig.salon, "{\"screenStreamId\":\"s1\"}", None).await;
    assert_eq!(receiver.next("VoiceSignal").await["payload"], "{\"screenStreamId\":\"s1\"}");
    assert!(observer.drain("VoiceParticipantChanged").await.is_empty());
}

#[tokio::test]
async fn leaving_voice_frees_the_seat() {
    let rig = Rig::start().await;
    let (jo, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let mut observer = rig.connect(&jo).await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;

    peer.leave().await;

    let emptied = observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && occupants_of(s).is_empty())
        .await;
    assert!(occupants_of(&emptied).is_empty());
    peer.refused("SendVoiceSignal", json!([signal(rig.salon, "state", "{\"muted\":false}", None)]), "Invalid voice signal.").await;

    // Leaving without a seat is not an error and announces nothing.
    observer.drain("VoiceParticipantChanged").await;
    peer.leave().await;
    assert!(observer.drain("VoiceParticipantChanged").await.is_empty());
}

#[tokio::test]
async fn a_dropped_connection_frees_its_seat_and_a_second_tab_keeps_the_account_online() {
    let rig = Rig::start().await;
    let (jo, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let mut observer = rig.connect(&jo).await;
    let mut voice = rig.connect(&maja).await;
    let mut chat = rig.connect(&maja).await;
    voice.join(rig.salon).await;
    observer.next_where("VoiceParticipantChanged", in_room(rig.salon)).await;

    voice.hub.hang_up().await;

    let emptied = observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && occupants_of(s).is_empty())
        .await;
    assert!(occupants_of(&emptied).is_empty());
    // The account is still connected through the other tab, so nobody was
    // told it went offline.
    let told = observer.hub.quiet_for(QUIET).await;
    assert!(!told.iter().any(|r| r["target"] == "PresenceChanged" && r["arguments"][0]["presence"] == "offline"), "{told:?}");
    chat.settle().await;
}

// Connect, which admits a member to a voice channel. The seat is the
// admission: a media session is only ever opened for one, so a connection that
// is refused its seat gets no media either, whatever it calls next.

#[tokio::test]
async fn connect_is_decided_in_the_channel_before_a_seat_or_media_is_given() {
    for (way, allowed) in [
        ("default", true),
        ("base-role-loses-it", false),
        ("held-role", true),
        ("channel-allow", true),
        ("channel-deny", false),
        ("member-deny", false),
        ("administrator-past-channel-deny", true),
    ] {
        let rig = Rig::start().await;
        let jo = rig.person("jonas").await;
        let base = rig.base_role();
        match way {
            "base-role-loses-it" => rig.base_role_loses(CONNECT),
            "held-role" => {
                rig.base_role_loses(CONNECT);
                rig.give_role(&jo, "Voice", 2, CONNECT | VIEW_CHANNEL);
            }
            "channel-allow" => {
                rig.base_role_loses(CONNECT);
                rig.channel_override(base, rig.salon, CONNECT, 0);
            }
            "channel-deny" => rig.channel_override(base, rig.salon, 0, CONNECT),
            "member-deny" => {
                rig.db()
                    .execute(
                        "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, ?5)",
                        params![Id::random(), rig.server_id, rig.salon, Id::parse(&jo.id).unwrap(), CONNECT],
                    )
                    .unwrap();
            }
            "administrator-past-channel-deny" => {
                rig.channel_override(base, rig.salon, 0, CONNECT);
                rig.give_role(&jo, "Admin", 2, ADMINISTRATOR);
            }
            _ => {}
        }
        let mut peer = rig.connect(&jo).await;

        if allowed {
            peer.join(rig.salon).await;
            peer.ok("OpenMediaSession", json!([rig.salon])).await;
            assert_eq!(rig.media.count("POST", "/v1/sessions"), 1, "{way}");
            continue;
        }
        peer.refused("JoinVoice", json!([rig.salon]), NOT_ALLOWED).await;
        // Skipping the join does not help: media is opened for a seat only.
        peer.refused("OpenMediaSession", json!([rig.salon]), "Not seated in this voice room.").await;
        assert!(rig.occupants(rig.salon).is_empty(), "{way}");
        assert_eq!(rig.media.count("POST", "/v1/sessions"), 0, "{way}");
    }
}

#[tokio::test]
async fn a_member_who_does_not_see_the_room_is_told_it_does_not_exist() {
    let rig = Rig::start().await;
    let jo = rig.person("jonas").await;
    rig.channel_override(rig.base_role(), rig.night, 0, VIEW_CHANNEL);
    let mut peer = rig.connect(&jo).await;

    peer.refused("JoinVoice", json!([rig.night]), "Unknown voice room.").await;
    peer.join(rig.salon).await;
    peer.refused("JoinVoice", json!([rig.night]), "Unknown voice room.").await;
    rig.occupants_become(rig.salon, &[&jo.id]).await;
}

#[tokio::test]
async fn a_member_without_connect_cannot_move_themselves_there_from_another_room() {
    let rig = Rig::start().await;
    let jo = rig.person("jonas").await;
    rig.channel_override(rig.base_role(), rig.night, 0, CONNECT);
    let mut peer = rig.connect(&jo).await;

    peer.join(rig.salon).await;
    peer.refused("JoinVoice", json!([rig.night]), NOT_ALLOWED).await;

    // The refused move leaves the seat where it was.
    rig.occupants_become(rig.salon, &[&jo.id]).await;
    assert!(rig.occupants(rig.night).is_empty());
}

#[tokio::test]
async fn a_moderator_may_move_somebody_into_a_channel_they_could_not_join() {
    let rig = Rig::start().await;
    let jo = rig.person("jonas").await;
    rig.channel_override(rig.base_role(), rig.night, 0, CONNECT);
    let mut peer = rig.connect(&jo).await;
    peer.join(rig.salon).await;

    let moved = rig
        .call(&rig.owner, Method::PATCH, &format!("/api/admin/channels/{}/members/{}", rig.salon, jo.id), Some(json!({"targetChannelId": rig.night})))
        .await;
    assert_eq!(moved.status, 204, "{}", moved.text());
    let told = peer.next("VoiceChannelMoved").await;
    assert_eq!(told, json!({"fromChannelId": rig.salon, "toChannelId": rig.night}));

    // The client confirms the move by joining the room it was moved to.
    peer.join(rig.night).await;
    rig.occupants_become(rig.night, &[&jo.id]).await;
    // Once out of the room, coming back is a join of their own again.
    peer.leave().await;
    peer.refused("JoinVoice", json!([rig.night]), NOT_ALLOWED).await;
}

#[tokio::test]
async fn a_seated_member_who_loses_connect_keeps_the_seat_until_they_leave() {
    let rig = Rig::start().await;
    let jo = rig.person("jonas").await;
    let mut peer = rig.connect(&jo).await;
    peer.join(rig.salon).await;

    rig.base_role_loses(CONNECT);

    peer.ok("OpenMediaSession", json!([rig.salon])).await;
    peer.join(rig.salon).await;
    peer.leave().await;
    peer.refused("JoinVoice", json!([rig.salon]), NOT_ALLOWED).await;
}

#[tokio::test]
async fn connect_is_about_server_channels_and_leaves_direct_calls_alone() {
    let rig = Rig::start().await;
    let (jo, nils) = (rig.person("jonas").await, rig.person("nils").await);
    let room = rig.conversation(&jo, &nils);
    rig.base_role_loses(CONNECT);
    let mut peer = rig.connect(&jo).await;

    peer.join(room).await;
    peer.refused("JoinVoice", json!([rig.salon]), NOT_ALLOWED).await;
}

// Direct calls: voice rooms whose id is a direct conversation. The
// conversations are written straight to the database, since the routes that
// open them belong to the social features.

#[tokio::test]
async fn a_direct_room_only_takes_its_two_owners_and_only_tells_them() {
    let rig = Rig::start().await;
    let (maja, ada, jo) = (rig.person("maja").await, rig.person("ada").await, rig.person("jonas").await);
    let room = rig.conversation(&maja, &ada);
    let (mut first, mut second, mut stranger) =
        (rig.connect(&maja).await, rig.connect(&ada).await, rig.connect(&jo).await);

    stranger.refused("JoinVoice", json!([room]), "Unknown voice room.").await;
    first.join(room).await;

    assert_eq!(occupants_of(&second.next_where("VoiceParticipantChanged", in_room(room)).await), std::slice::from_ref(&maja.id));
    second.join(room).await;
    first.ok("SendVoiceSignal", json!([signal(room, "state", "{}", Some(&ada.id))])).await;
    assert_eq!(second.next("VoiceSignal").await["fromUserId"], maja.id.as_str());
    assert!(stranger.drain("VoiceParticipantChanged").await.is_empty());
    stranger
        .refused("SendVoiceSignal", json!([signal(room, "state", "{}", None)]), "Invalid voice signal.")
        .await;
}

#[tokio::test]
async fn a_call_is_refused_to_both_people_when_either_blocked_the_other() {
    for (blocker, blocked) in [("maja", "ada"), ("ada", "maja")] {
        let rig = Rig::start().await;
        let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
        let room = rig.conversation(&maja, &ada);
        let people = [("maja", &maja), ("ada", &ada)];
        let find = |name: &str| people.iter().find(|(n, _)| *n == name).unwrap().1;
        rig.block(find(blocker), find(blocked));
        let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);

        first.refused("JoinVoice", json!([room]), "Unknown voice room.").await;
        second.refused("JoinVoice", json!([room]), "Unknown voice room.").await;
    }
}

#[tokio::test]
async fn a_call_to_somebody_who_takes_no_direct_messages_needs_a_friendship() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let room = rig.conversation(&maja, &ada);
    rig.takes_direct_messages(&ada, false);
    let mut caller = rig.connect(&maja).await;

    caller.refused("JoinVoice", json!([room]), "Unknown voice room.").await;

    rig.befriend(&maja, &ada);
    caller.join(room).await;
    rig.occupants_become(room, &[&maja.id]).await;
}

#[tokio::test]
async fn a_call_that_is_already_ringing_can_be_answered_whatever_the_caller_takes() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let room = rig.conversation(&maja, &ada);
    rig.takes_direct_messages(&maja, false);
    let (mut caller, mut answerer) = (rig.connect(&maja).await, rig.connect(&ada).await);

    answerer.refused("JoinVoice", json!([room]), "Unknown voice room.").await;
    caller.join(room).await;
    answerer.join(room).await;

    rig.occupants_become(room, &[&maja.id, &ada.id]).await;
}

#[tokio::test]
async fn declining_a_call_ends_it_for_both_people() {
    let rig = Rig::start().await;
    let (maja, ada, jo) = (rig.person("maja").await, rig.person("ada").await, rig.person("jonas").await);
    let room = rig.conversation(&maja, &ada);
    let (mut first, mut second, mut stranger) =
        (rig.connect(&maja).await, rig.connect(&ada).await, rig.connect(&jo).await);
    first.join(room).await;
    second.join(room).await;

    stranger.refused("DeclineDirectCall", json!([room]), "Unknown direct call.").await;
    second.ok("DeclineDirectCall", json!([room])).await;

    for peer in [&mut first, &mut second] {
        let declined = peer.next("DirectCallDeclined").await;
        assert_eq!(declined, json!({"channelId": room, "byUserId": ada.id}));
        let emptied = peer
            .next_where("VoiceParticipantChanged", |s| in_room(room)(s) && occupants_of(s).is_empty())
            .await;
        assert!(occupants_of(&emptied).is_empty());
    }
    assert!(rig.occupants(room).is_empty());
    first.refused("DeclineDirectCall", json!(["x"]), "Invalid conversationId.").await;
    first.refused("DeclineDirectCall", json!([null]), "Invalid conversationId.").await;
}

#[tokio::test]
async fn a_call_can_still_be_declined_after_one_side_blocked_the_other() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let room = rig.conversation(&maja, &ada);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);
    first.join(room).await;
    rig.block(&maja, &ada);

    second.ok("DeclineDirectCall", json!([room])).await;

    assert_eq!(first.next("DirectCallDeclined").await["channelId"], room.to_string().as_str());
    rig.occupants_become(room, &[]).await;
}

#[tokio::test]
async fn declining_a_call_nobody_sits_in_says_nothing_but_the_empty_room() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let room = rig.conversation(&maja, &ada);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);

    first.ok("DeclineDirectCall", json!([room])).await;

    let snapshot = second.next_where("VoiceParticipantChanged", in_room(room)).await;
    assert!(occupants_of(&snapshot).is_empty());
    assert!(second.drain("DirectCallDeclined").await.is_empty());
}

#[tokio::test]
async fn a_device_that_connects_during_a_call_is_told_who_is_in_it() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let room = rig.conversation(&maja, &ada);
    let mut caller = rig.connect(&maja).await;
    caller.join(room).await;

    // The connection is not let settle: what it is told while it starts is
    // what is being looked at.
    let cookie = format!("{COOKIE}={}", ada.client.cookie(COOKIE).unwrap());
    let mut device = HubClient::connect(rig.server.address(), &cookie).await;
    let restored = loop {
        let record = device.record().await;
        if record["target"] == "VoiceParticipantChanged" {
            break record["arguments"][0].clone();
        }
    };
    assert_eq!(restored["channelId"], room.to_string().as_str());
    assert_eq!(occupants_of(&restored), std::slice::from_ref(&maja.id));
}

#[tokio::test]
async fn blocking_ends_a_live_direct_call_and_tells_both_people() {
    let rig = Rig::start().await;
    let (maja, ada, jonas) = (rig.person("maja").await, rig.person("ada").await, rig.person("jonas").await);
    let room = rig.conversation(&maja, &ada);
    let (mut blocker, mut blocked, mut outside) =
        (rig.connect(&maja).await, rig.connect(&ada).await, rig.connect(&jonas).await);
    blocker.join(room).await;
    blocked.join(room).await;
    rig.occupants_become(room, &[&maja.id, &ada.id]).await;

    let answer = rig.call(&maja, Method::POST, &format!("/api/social/blocks/{}", ada.id), None).await;
    assert_eq!(answer.status, 204, "{}", answer.text());

    for peer in [&mut blocker, &mut blocked] {
        assert_eq!(peer.next("DirectCallDeclined").await, json!({"channelId": room, "byUserId": maja.id}));
    }
    let emptied = blocked
        .next_where("VoiceParticipantChanged", |s| in_room(room)(s) && occupants_of(s).is_empty())
        .await;
    assert!(occupants_of(&emptied).is_empty());
    rig.occupants_become(room, &[]).await;
    assert!(outside.drain("DirectCallDeclined").await.is_empty());
    // Blocking again, with nobody in the room, tells the room it is empty and nobody that it was declined.
    rig.call(&maja, Method::POST, &format!("/api/social/blocks/{}", ada.id), None).await;
    assert!(blocked.drain("DirectCallDeclined").await.is_empty());
}

#[tokio::test]
async fn ids_in_the_moderation_paths_are_read_as_the_first_server_reads_them() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let id = Id::random();
    let answer = owner
        .client
        .send(Request::new(Method::DELETE, &format!("/api/admin/channels/{id}/members/{id}")))
        .await;
    assert_ne!(answer.status, 405, "the route exists");
    // Ids in a path are read as the first server reads them.
    let braced = owner
        .client
        .send(Request::new(Method::DELETE, &format!("/api/admin/channels/%7B{id}%7D/members/{}", id.to_string().replace('-', ""))))
        .await;
    assert_eq!(braced.status, answer.status);
    let bad = owner.client.send(Request::new(Method::DELETE, "/api/admin/channels/x/members/y")).await;
    assert_eq!(bad.status, 404);
}

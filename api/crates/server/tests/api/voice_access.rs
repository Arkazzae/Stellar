//! A person who sits in a voice room and loses the right to is taken out of it
//! at once: the seat and the media session are gone, the room is told who is
//! left, and the person is told with `VoiceChannelKicked`. Every way of losing
//! the right has a test of its own, and so has a change that leaves it.
//!
//! The person sits in the instance's voice channel through a role of their own
//! (the base role cannot connect), somebody else makes the change over HTTP, and
//! a second person who keeps the right sits beside them.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Id;

use crate::support::{Person, Request, until};
use crate::voice::{CONNECT, Peer, Rig, VIEW_CHANNEL, in_room, signal};

/// Somebody seated through a role of their own, beside somebody who stays.
struct Scene {
    rig: Rig,
    jonas: Person,
    maja: Person,
    /// The role that gives jonas the right to sit.
    role: Id,
    peer: Peer,
    other: Peer,
    session: String,
}

async fn scene() -> Scene {
    let rig = Rig::start().await;
    let (jonas, maja) = (rig.person("jonas").await, rig.person("maja").await);
    rig.base_role_loses(CONNECT);
    let role = rig.give_role(&jonas, "Voice", 2, CONNECT | VIEW_CHANNEL);
    rig.give_role(&maja, "Other", 3, CONNECT | VIEW_CHANNEL);
    let (mut peer, mut other) = (rig.connect(&jonas).await, rig.connect(&maja).await);
    peer.join(rig.salon).await;
    other.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    until(|| rig.media.sessions().contains(&session)).await;
    other.drain("VoiceParticipantChanged").await;
    Scene {
        rig,
        jonas,
        maja,
        role,
        peer,
        other,
        session,
    }
}

impl Scene {
    /// What the owner asks of the server, which must be answered with `status`.
    async fn owner(&self, method: Method, path: &str, body: Option<Value>, status: u16) {
        let answer = self.rig.call(&self.rig.owner, method, path, body).await;
        assert_eq!(answer.status, status, "{path}: {}", answer.text());
    }

    /// The person was taken out: told, out of the seat, the media session closed,
    /// the room told who is left (unless the room is gone), the other person
    /// still there.
    async fn assert_unseated(&mut self, room_remains: bool) {
        let room = self.rig.salon;
        assert_eq!(self.peer.next("VoiceChannelKicked").await, json!({"channelId": room}));
        self.rig.media.delete_of(&self.session).await;
        until(|| !self.rig.media.sessions().contains(&self.session)).await;
        self.peer
            .refused("SendVoiceSignal", json!([signal(room, "state", "{}", None)]), "Invalid voice signal.")
            .await;
        self.peer.refused("OpenMediaSession", json!([room]), "Not seated in this voice room.").await;
        if room_remains {
            let snapshot = self
                .other
                .next_where("VoiceParticipantChanged", |s| in_room(room)(s) && s["participants"].as_array().is_some_and(|p| p.len() == 1))
                .await;
            assert_eq!(snapshot["participants"][0]["userId"], self.maja.id.as_str());
            self.other.ok("SendVoiceSignal", json!([signal(room, "state", "{}", None)])).await;
        }
        assert!(self.peer.drain("MediaSessionClosed").await.is_empty());
    }

    /// Nobody was taken out.
    async fn assert_nobody_unseated(&mut self) {
        let room = self.rig.salon;
        self.peer.ok("SendVoiceSignal", json!([signal(room, "state", "{}", None)])).await;
        assert!(self.peer.drain("VoiceChannelKicked").await.is_empty());
        assert!(self.other.drain("VoiceChannelKicked").await.is_empty());
        assert!(self.other.drain("VoiceParticipantChanged").await.is_empty());
        assert!(self.rig.media.sessions().contains(&self.session));
        assert_eq!(self.rig.media.count("DELETE", &format!("/v1/sessions/{}", self.session)), 0);
    }
}

fn base_of(rig: &Rig) -> Id {
    rig.base_role()
}

// ------------------------------------------------------------------ permissions

#[tokio::test]
async fn a_role_edited_to_lose_connect_takes_its_holder_out() {
    let mut scene = scene().await;
    scene.owner(Method::PATCH, &format!("/api/admin/roles/{}", scene.role), Some(json!({"permissions": ["viewChannel"]})), 200).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn a_role_edited_to_lose_a_right_that_the_base_role_still_gives_takes_nobody_out() {
    let mut scene = scene().await;
    scene.owner(Method::PATCH, &format!("/api/admin/roles/{}", scene.role), Some(json!({"permissions": ["connect"]})), 200).await;
    // The base role still lets them see the channel, and the role still lets them connect.
    scene.assert_nobody_unseated().await;
}

#[tokio::test]
async fn a_role_taken_from_the_member_takes_them_out() {
    let mut scene = scene().await;
    let path = format!("/api/admin/members/{}/role", scene.jonas.id);
    let base = scene.rig.base_role().to_string();
    scene.owner(Method::PATCH, &path, Some(json!({"roleId": base})), 204).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn a_role_deleted_takes_its_holders_out() {
    let mut scene = scene().await;
    scene.owner(Method::DELETE, &format!("/api/admin/roles/{}", scene.role), None, 204).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn an_override_of_the_role_on_the_channel_takes_its_holder_out() {
    let mut scene = scene().await;
    let path = format!("/api/admin/channels/{}/permissions/{}", scene.rig.salon, scene.role);
    scene.owner(Method::PUT, &path, Some(json!({"deny": ["connect"]})), 200).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn an_override_of_the_member_on_the_channel_takes_them_out() {
    let mut scene = scene().await;
    let path = format!("/api/admin/channels/{}/permissions/members/{}", scene.rig.salon, scene.jonas.id);
    scene.owner(Method::PUT, &path, Some(json!({"deny": ["connect"]})), 200).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn making_the_channel_private_takes_out_whoever_may_no_longer_see_it() {
    let mut scene = scene().await;
    // Private: the roles that let people in are denied view, and the one who stays is allowed.
    let salon = scene.rig.salon;
    let base = base_of(&scene.rig);
    let other_role: Id = scene.rig.db().query_row(
        "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"Name\" = 'Other'",
        [],
        |row| row.get(0),
    ).unwrap();
    scene.owner(Method::PUT, &format!("/api/admin/channels/{salon}/permissions/{other_role}"), Some(json!({"allow": ["viewChannel"]})), 200).await;
    scene.owner(Method::PUT, &format!("/api/admin/channels/{salon}/permissions/{}", scene.role), Some(json!({"deny": ["viewChannel"]})), 200).await;
    scene.owner(Method::PUT, &format!("/api/admin/channels/{salon}/permissions/{base}"), Some(json!({"deny": ["viewChannel"]})), 200).await;
    scene.assert_unseated(true).await;
}

/// A section with the channel in it, which a test then writes an override on.
fn section_with_the_channel(scene: &Scene) -> Id {
    let section = Id::random();
    let db = scene.rig.db();
    db.execute(
        "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, 'Calls', 0)",
        params![section, scene.rig.server_id],
    )
    .unwrap();
    db.execute("UPDATE \"Channels\" SET \"SectionId\" = ?1 WHERE \"Id\" = ?2", params![section, scene.rig.salon]).unwrap();
    section
}

#[tokio::test]
async fn an_override_on_the_section_of_the_channel_takes_the_holder_out() {
    let mut scene = scene().await;
    let section = section_with_the_channel(&scene);
    scene.owner(Method::PUT, &format!("/api/admin/sections/{section}/permissions/{}", scene.role), Some(json!({"deny": ["connect"]})), 200).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn moving_the_channel_into_a_section_that_denies_them_takes_the_holder_out() {
    let mut scene = scene().await;
    let section = Id::random();
    {
        let db = scene.rig.db();
        db.execute(
            "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, 'Staff', 0)",
            params![section, scene.rig.server_id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 1, ?3, ?4, 0, ?5)",
            params![Id::random(), scene.rig.server_id, section, scene.role, CONNECT],
        )
        .unwrap();
    }
    let ids: Vec<String> = {
        let db = scene.rig.db();
        let mut statement = db.prepare("SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\"").unwrap();
        statement
            .query_map([scene.rig.server_id], |row| row.get::<_, Id>(0))
            .unwrap()
            .map(|id| id.unwrap().to_string())
            .collect()
    };
    let body = json!({"channelIds": ids, "move": {"channelId": scene.rig.salon, "sectionId": section}});
    scene.owner(Method::PUT, "/api/admin/channels/order", Some(body), 204).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn the_cockpit_editing_a_role_takes_its_holder_out() {
    let mut scene = scene().await;
    scene.owner(Method::PATCH, &format!("/api/cockpit/roles/{}", scene.role), Some(json!({"permissions": ["viewChannel"]})), 200).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn the_cockpit_giving_another_role_takes_a_holder_of_the_old_one_out() {
    let mut scene = scene().await;
    let base = base_of(&scene.rig);
    scene.owner(Method::PATCH, &format!("/api/cockpit/users/{}/role", scene.jonas.id), Some(json!({"roleId": base})), 204).await;
    scene.assert_unseated(true).await;
}

// ------------------------------------------------------------------- removals

#[tokio::test]
async fn a_channel_deleted_takes_out_whoever_sat_in_it() {
    let mut scene = scene().await;
    scene.owner(Method::DELETE, &format!("/api/admin/channels/{}", scene.rig.salon), None, 204).await;
    scene.assert_unseated(false).await;
    // Nobody is seated in a room that is gone.
    scene.other.refused("SendVoiceSignal", json!([signal(scene.rig.salon, "state", "{}", None)]), "Invalid voice signal.").await;
}

#[tokio::test]
async fn a_section_that_holds_a_voice_channel_cannot_be_deleted_so_nobody_is_taken_out() {
    let mut scene = scene().await;
    let section = section_with_the_channel(&scene);
    scene.owner(Method::DELETE, &format!("/api/admin/sections/{section}"), None, 409).await;
    scene.assert_nobody_unseated().await;
}

#[tokio::test]
async fn a_kick_from_the_server_takes_the_member_out() {
    let mut scene = scene().await;
    scene.owner(Method::DELETE, &format!("/api/admin/members/{}", scene.jonas.id), None, 204).await;
    scene.assert_unseated(true).await;
}

#[tokio::test]
async fn a_ban_from_the_server_takes_the_member_out() {
    let mut scene = scene().await;
    scene.owner(Method::POST, &format!("/api/admin/members/{}/ban", scene.jonas.id), Some(json!({"reason": null})), 201).await;
    scene.assert_unseated(true).await;
}

/// Somebody who owns a server of their own, seated in its voice channel next to a
/// member who may not sit there.
struct Crew {
    rig: Rig,
    ada: Person,
    jonas: Person,
    server: String,
    channel: Id,
    peer: Peer,
    session: String,
}

async fn crew() -> Crew {
    let rig = Rig::start().await;
    let (ada, jonas) = (rig.person("ada").await, rig.person("jonas").await);
    let made = ada.client.send(Request::new(Method::POST, "/api/servers").json(&json!({"name": "Crew", "voiceChannelName": "meeting"}))).await;
    assert_eq!(made.status, 201, "{}", made.text());
    let server = made.json::<Value>()["id"].as_str().unwrap().to_string();
    let channel: Id = rig
        .db()
        .query_row(
            "SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = 1",
            [Id::parse(&server).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    let invite = ada
        .client
        .send(Request::new(Method::POST, "/api/admin/invites").header("X-Stellarr-Server", &server).json(&json!({})))
        .await;
    let code = invite.json::<Value>()["url"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    let joined = jonas.client.send(Request::new(Method::POST, &format!("/api/invites/{code}/join"))).await;
    assert_eq!(joined.status, 200, "{}", joined.text());
    // Only the owner may sit in it.
    let base: Id = rig
        .db()
        .query_row(
            "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0",
            [Id::parse(&server).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    let denied = ada
        .client
        .send(
            Request::new(Method::PUT, &format!("/api/admin/channels/{channel}/permissions/{base}"))
                .header("X-Stellarr-Server", &server)
                .json(&json!({"deny": ["connect"]})),
        )
        .await;
    assert_eq!(denied.status, 200, "{}", denied.text());
    let mut peer = rig.connect_to(&format!("?serverId={server}"), &ada).await;
    peer.join(channel).await;
    let session = peer.open_media(channel).await;
    until(|| rig.media.sessions().contains(&session)).await;
    Crew { rig, ada, jonas, server, channel, peer, session }
}

impl Crew {
    async fn assert_unseated(&mut self) {
        assert_eq!(self.peer.next("VoiceChannelKicked").await, json!({"channelId": self.channel}));
        self.rig.media.delete_of(&self.session).await;
        until(|| !self.rig.media.sessions().contains(&self.session)).await;
        self.peer.refused("SendVoiceSignal", json!([signal(self.channel, "state", "{}", None)]), "Invalid voice signal.").await;
    }

    async fn ada_asks(&self, method: Method, path: &str, body: Option<Value>, status: u16) {
        let mut request = Request::new(method, path).header("X-Stellarr-Server", &self.server);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let answer = self.ada.client.send(request).await;
        assert_eq!(answer.status, status, "{path}: {}", answer.text());
    }
}

#[tokio::test]
async fn handing_the_server_over_takes_the_former_owner_out_of_a_room_only_an_owner_may_sit_in() {
    let mut crew = crew().await;
    crew.ada_asks(Method::POST, "/api/admin/server/transfer", Some(json!({"userId": crew.jonas.id})), 204).await;
    crew.assert_unseated().await;
}

#[tokio::test]
async fn the_cockpit_handing_a_server_over_takes_the_former_owner_out() {
    let mut crew = crew().await;
    let path = format!("/api/cockpit/servers/{}/transfer", crew.server);
    let answer = crew.rig.call(&crew.rig.owner, Method::POST, &path, Some(json!({"userId": crew.jonas.id}))).await;
    assert_eq!(answer.status, 204, "{}", answer.text());
    crew.assert_unseated().await;
}

#[tokio::test]
async fn deleting_the_server_takes_out_whoever_sat_in_its_rooms() {
    let mut crew = crew().await;
    crew.ada_asks(Method::DELETE, "/api/admin/server", None, 204).await;
    crew.assert_unseated().await;
}

#[tokio::test]
async fn the_cockpit_deleting_a_server_takes_out_whoever_sat_in_its_rooms() {
    let mut crew = crew().await;
    let answer = crew.rig.call(&crew.rig.owner, Method::DELETE, &format!("/api/cockpit/servers/{}", crew.server), None).await;
    assert_eq!(answer.status, 204, "{}", answer.text());
    crew.assert_unseated().await;
}

// --------------------------------------------------- what needs nothing from here

#[tokio::test]
async fn an_account_disabled_in_the_cockpit_loses_its_connection_and_with_it_its_seat() {
    let mut scene = scene().await;
    scene.owner(Method::PATCH, &format!("/api/cockpit/users/{}/status", scene.jonas.id), Some(json!({"disabled": true})), 204).await;
    // The watcher of sessions closes the connection, and the seat goes with it.
    assert_eq!(scene.peer.hub.finish_close().await, Some(4401));
    scene.rig.media.delete_of(&scene.session).await;
    scene.other.next_where("VoiceParticipantChanged", |s| in_room(scene.rig.salon)(s) && s["participants"].as_array().is_some_and(|p| p.len() == 1)).await;
}

// ------------------------------------------------------------ what takes nobody

#[tokio::test]
async fn a_change_that_leaves_the_right_takes_nobody_out() {
    let mut scene = scene().await;
    let salon = scene.rig.salon;
    // Another override of the channel, an unrelated role, and a power that is no concern of a call.
    scene.owner(Method::PUT, &format!("/api/admin/channels/{salon}/permissions/{}", scene.role), Some(json!({"deny": ["sendMessages"]})), 200).await;
    scene.owner(Method::PUT, &format!("/api/admin/channels/{salon}/permissions/members/{}", scene.maja.id), Some(json!({"allow": ["connect"]})), 200).await;
    scene.owner(Method::POST, "/api/admin/roles", Some(json!({"name": "Unrelated", "color": "#fff", "permissions": ["manageChannels"]})), 201).await;
    scene.owner(Method::PATCH, &format!("/api/admin/roles/{}", scene.role), Some(json!({"permissions": ["connect", "viewChannel", "sendMessages"]})), 200).await;
    scene.assert_nobody_unseated().await;
}

#[tokio::test]
async fn a_moderators_move_is_no_loss_of_access() {
    let mut scene = scene().await;
    // The room they are moved to is one they could not have joined.
    let night = scene.rig.night;
    scene.rig.channel_override(scene.role, night, 0, CONNECT);
    scene.owner(Method::PATCH, &format!("/api/admin/channels/{}/members/{}", scene.rig.salon, scene.jonas.id), Some(json!({"targetChannelId": night})), 204).await;
    assert_eq!(scene.peer.next("VoiceChannelMoved").await["toChannelId"], night.to_string().as_str());
    assert!(scene.peer.drain("VoiceChannelKicked").await.is_empty());
    scene.peer.ok("SendVoiceSignal", json!([signal(night, "state", "{}", None)])).await;
}

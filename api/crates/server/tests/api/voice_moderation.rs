//! A moderator's hand on a voice seat, and the clips played into a room.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::Person;
use crate::voice::{MODERATE_MEMBERS, Rig, in_room, occupants_of, set_preference};

fn member_path(channel: Id, user: &Person) -> String {
    format!("/api/admin/channels/{channel}/members/{}", user.id)
}

fn move_body(target: Id) -> Option<Value> {
    Some(json!({ "targetChannelId": target }))
}

fn audit_lines(rig: &Rig) -> Vec<(String, String, Option<String>, String)> {
    let db = rig.db();
    let mut statement = db
        .prepare("SELECT \"Action\", \"Summary\", \"ChangesJson\", \"TargetType\" FROM \"ServerAuditEntries\" WHERE \"Action\" LIKE 'voice.%' ORDER BY \"CreatedAt\"")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[tokio::test]
async fn kicking_somebody_out_of_a_voice_channel_frees_the_seat_tells_them_and_the_room_and_is_audited() {
    let rig = Rig::start().await;
    let (jonas, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let mut observer = rig.connect(&jonas).await;
    let mut kicked = rig.connect(&maja).await;
    let mut other_tab = rig.connect(&maja).await;
    kicked.join(rig.salon).await;
    observer.next_where("VoiceParticipantChanged", in_room(rig.salon)).await;

    let answer = rig.call(&rig.owner, Method::DELETE, &member_path(rig.salon, &maja), None).await;

    assert_eq!(answer.status, 204);
    assert_eq!(answer.text(), "");
    assert_eq!(kicked.next("VoiceChannelKicked").await, json!({"channelId": rig.salon}));
    let emptied = observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && occupants_of(s).is_empty())
        .await;
    assert!(occupants_of(&emptied).is_empty());
    assert!(other_tab.drain("VoiceChannelKicked").await.is_empty());
    let lines = audit_lines(&rig);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0, "voice.disconnect");
    assert_eq!(lines[0].1, "Owner disconnected Maja from #voice");
    assert_eq!(lines[0].2.as_deref(), Some("[{\"field\":\"channel\",\"from\":\"voice\"}]"));
    assert_eq!(lines[0].3, "member");

    // Kicking somebody who has already gone is the end state that was asked for:
    // nothing more is written or said.
    let again = rig.call(&rig.owner, Method::DELETE, &member_path(rig.salon, &maja), None).await;
    assert_eq!(again.status, 204);
    assert_eq!(audit_lines(&rig).len(), 1);
    assert!(kicked.drain("VoiceChannelKicked").await.is_empty());
    assert!(observer.drain("VoiceParticipantChanged").await.is_empty());
}

#[tokio::test]
async fn kicking_leaves_a_seat_in_another_room_and_every_connection_of_the_account_in_this_one() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.night).await;

    let answer = rig.call(&rig.owner, Method::DELETE, &member_path(rig.salon, &maja), None).await;

    assert_eq!(answer.status, 204);
    rig.occupants_become(rig.night, &[&maja.id]).await;
    assert!(peer.drain("VoiceChannelKicked").await.is_empty());
    assert!(audit_lines(&rig).is_empty());
}

#[tokio::test]
async fn moving_somebody_keeps_their_seat_and_mute_state_tells_them_both_rooms_and_is_audited() {
    let rig = Rig::start().await;
    let (jonas, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let mut observer = rig.connect(&jonas).await;
    let mut moved = rig.connect(&maja).await;
    moved.join(rig.salon).await;
    moved.state(rig.salon, "{\"muted\":true}", None).await;

    let answer = rig
        .call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), move_body(rig.night))
        .await;

    assert_eq!(answer.status, 204, "{}", answer.text());
    assert_eq!(moved.next("VoiceChannelMoved").await, json!({"fromChannelId": rig.salon, "toChannelId": rig.night}));
    let left = observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && occupants_of(s).is_empty())
        .await;
    let joined = observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.night)(s) && !occupants_of(s).is_empty())
        .await;
    assert_eq!(joined["participants"][0]["muted"], true);
    assert!(joined["sequence"].as_i64() > left["sequence"].as_i64());
    let lines = audit_lines(&rig);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0, "voice.move");
    assert_eq!(lines[0].1, "Owner moved Maja from #voice to #night-shift");
    assert_eq!(lines[0].2.as_deref(), Some("[{\"field\":\"channel\",\"from\":\"voice\",\"to\":\"night-shift\"}]"));

    // The seat is where it was put and nobody sits in the old room.
    assert_eq!(rig.occupants(rig.salon), Vec::<String>::new());
    assert_eq!(rig.occupants(rig.night), std::slice::from_ref(&maja.id));
}

#[tokio::test]
async fn a_move_that_changes_nothing_or_finds_nobody_is_answered_as_the_first_server_answered() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let same = rig
        .call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), move_body(rig.salon))
        .await;
    assert_eq!(same.status, 204);
    assert!(peer.drain("VoiceChannelMoved").await.is_empty());
    assert!(audit_lines(&rig).is_empty());

    // Nobody of that account sits in the room: a conflict, and nothing written.
    let nobody = rig
        .call(&rig.owner, Method::PATCH, &member_path(rig.salon, &ada), move_body(rig.night))
        .await;
    assert_eq!(nobody.status, 409);
    assert!(audit_lines(&rig).is_empty());

    // A room that does not exist, a body that is not one, and a member that is missing.
    let unknown = rig
        .call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), move_body(Id::random()))
        .await;
    assert_eq!(unknown.status, 404);
    let missing = rig.call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), Some(json!({}))).await;
    assert_eq!(missing.status, 404);
    let null = rig
        .call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), Some(json!({"targetChannelId": null})))
        .await;
    assert_eq!(null.status, 400);
    let text = rig
        .call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), Some(json!({"targetChannelId": "x"})))
        .await;
    assert_eq!(text.status, 400);
    let no_body = rig.call(&rig.owner, Method::PATCH, &member_path(rig.salon, &maja), None).await;
    assert!(no_body.status == 400 || no_body.status == 415, "{}", no_body.status);
    rig.occupants_become(rig.salon, &[&maja.id]).await;
}

#[tokio::test]
async fn nobody_moderates_without_a_session_a_membership_or_the_right() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let path = member_path(rig.salon, &maja);

    let anonymous = rig.server.client().send(crate::support::Request::new(Method::DELETE, &path)).await;
    assert_eq!(anonymous.status, 401);
    // An ordinary member holds nothing of the kind.
    let forbidden = rig.call(&ada, Method::DELETE, &path, None).await;
    assert_eq!(forbidden.status, 403);
    assert_eq!(forbidden.text(), "");
    let moved = rig.call(&ada, Method::PATCH, &path, move_body(rig.night)).await;
    assert_eq!(moved.status, 403);
    // A server the caller is no member of is answered 403, a name that is no id 400.
    let elsewhere = rig
        .server
        .client()
        .send(
            crate::support::Request::new(Method::DELETE, &path)
                .header("X-Stellarr-Server", &Id::random().to_string()),
        )
        .await;
    assert_eq!(elsewhere.status, 401);
    let strange = ada
        .client
        .send(crate::support::Request::new(Method::DELETE, &path).header("X-Stellarr-Server", &Id::random().to_string()))
        .await;
    assert_eq!(strange.status, 403);
    let malformed = ada
        .client
        .send(crate::support::Request::new(Method::DELETE, &path).header("X-Stellarr-Server", "nonsense"))
        .await;
    assert_eq!(malformed.status, 400);
    rig.occupants_become(rig.salon, &[&maja.id]).await;
    assert!(audit_lines(&rig).is_empty());
}

#[tokio::test]
async fn a_room_the_moderator_does_not_see_is_not_found_and_a_rank_above_is_forbidden() {
    let rig = Rig::start().await;
    let (maja, ada, ilya) = (rig.person("maja").await, rig.person("ada").await, rig.person("ilya").await);
    rig.give_role(&ada, "Moderators", 3, MODERATE_MEMBERS);
    rig.give_role(&ilya, "Seniors", 2, MODERATE_MEMBERS);
    let mut peer = rig.connect(&maja).await;
    let mut senior = rig.connect(&ilya).await;
    peer.join(rig.salon).await;
    senior.join(rig.salon).await;

    // A moderator may act on those they outrank, in a room they see.
    let moved = rig.call(&ada, Method::PATCH, &member_path(rig.salon, &maja), move_body(rig.night)).await;
    assert_eq!(moved.status, 204, "{}", moved.text());
    // Not on somebody of their rank or above, which is forbidden.
    let outranked = rig.call(&ada, Method::DELETE, &member_path(rig.salon, &ilya), None).await;
    assert_eq!(outranked.status, 403);
    let moved_senior = rig.call(&ada, Method::PATCH, &member_path(rig.salon, &ilya), move_body(rig.night)).await;
    assert_eq!(moved_senior.status, 403);
    // And not in a room they do not see, which does not exist for them.
    rig.channel_override(rig.base_role(), rig.night, 0, 1 << 14);
    let unseen_kick = rig.call(&ada, Method::DELETE, &member_path(rig.night, &maja), None).await;
    assert_eq!(unseen_kick.status, 404);
    let unseen_move = rig.call(&ada, Method::PATCH, &member_path(rig.salon, &ilya), move_body(rig.night)).await;
    assert_eq!(unseen_move.status, 404);
    // The owner outranks everybody.
    let owner_kick = rig.call(&rig.owner, Method::DELETE, &member_path(rig.salon, &ilya), None).await;
    assert_eq!(owner_kick.status, 204);
}

#[tokio::test]
async fn a_move_cannot_get_round_a_denial_at_the_source_or_the_destination() {
    for (deny_destination, in_section) in [(false, false), (false, true), (true, false), (true, true)] {
        let rig = Rig::start().await;
        let (ada, target) = (rig.person("ada").await, rig.person("jonas").await);
        let moderators = rig.give_role(&ada, "Moderators", 3, MODERATE_MEMBERS);
        let mut peer = rig.connect(&target).await;
        peer.join(rig.salon).await;
        let denied = if deny_destination { rig.night } else { rig.salon };
        let (target_type, target_id) = if in_section {
            let section = Id::random();
            rig.db()
                .execute(
                    "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\") VALUES (?1, ?2, 'Restricted')",
                    params![section, rig.server_id],
                )
                .unwrap_or_else(|_| {
                    rig.db()
                        .execute("INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, 'Restricted', 0)", params![section, rig.server_id])
                        .unwrap()
                });
            rig.db()
                .execute("UPDATE \"Channels\" SET \"SectionId\" = ?1 WHERE \"Id\" = ?2", params![section, denied])
                .unwrap();
            (1, section)
        } else {
            (0, denied)
        };
        rig.db()
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                params![Id::random(), rig.server_id, target_type, target_id, moderators, MODERATE_MEMBERS],
            )
            .unwrap();

        let moved = rig.call(&ada, Method::PATCH, &member_path(rig.salon, &target), move_body(rig.night)).await;
        let kicked = rig.call(&ada, Method::DELETE, &member_path(rig.salon, &target), None).await;

        let label = format!("destination denied: {deny_destination}, in a section: {in_section}");
        assert_eq!(moved.status, 403, "{label}");
        assert_eq!(kicked.status, if deny_destination { 204 } else { 403 }, "{label}");
    }
}

#[tokio::test]
async fn local_moderation_grants_on_both_rooms_authorize_a_move_and_their_removal_revokes_it() {
    let rig = Rig::start().await;
    let (ada, target) = (rig.person("ada").await, rig.person("jonas").await);
    let moderators = rig.give_role(&ada, "Moderators", 3, 0);
    for room in [rig.salon, rig.night] {
        rig.channel_override(moderators, room, MODERATE_MEMBERS, 0);
    }
    let mut peer = rig.connect(&target).await;
    peer.join(rig.salon).await;

    let moved = rig.call(&ada, Method::PATCH, &member_path(rig.salon, &target), move_body(rig.night)).await;
    assert_eq!(moved.status, 204, "{}", moved.text());
    rig.db().execute("DELETE FROM \"PermissionOverrides\" WHERE \"RoleId\" = ?1", [moderators]).unwrap();
    let revoked = rig.call(&ada, Method::DELETE, &member_path(rig.night, &target), None).await;
    assert_eq!(revoked.status, 403);
    peer.next("VoiceChannelMoved").await;
    rig.occupants_become(rig.night, &[&target.id]).await;
}

// The soundboard: clips played into a room by the people sitting in it.

fn add_sound(rig: &Rig, server: Id, name: &str, start: f64, end: f64, duration: Option<f64>) -> Id {
    let (attachment, sound) = (Id::random(), Id::random());
    let db = rig.db();
    let owner = Id::parse(&rig.owner.id).unwrap();
    db.execute(
        "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"Width\", \"Height\", \"DurationSeconds\", \"IsVoiceMessage\", \"IsEncrypted\", \"CreatedAt\")
         VALUES (?1, ?2, ?1, 'sound.wav', 'audio/wav', 16, NULL, NULL, ?3, 0, 0, ?4)",
        params![attachment, owner, duration, Timestamp::now()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"SoundboardSounds\" (\"Id\", \"ServerId\", \"AttachmentId\", \"CreatedById\", \"Name\", \"Glyph\", \"TrimStartSeconds\", \"TrimEndSeconds\", \"Position\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, '🔔', ?6, ?7, 0, ?8)",
        params![sound, server, attachment, owner, name, start, end, Timestamp::now()],
    )
    .unwrap();
    sound
}

/// A sound of the instance's server that anybody may play, for another test file.
pub fn add_sound_for_tests(rig: &Rig) -> Id {
    soundboard_rules(rig, true, false);
    add_sound(rig, rig.server_id, "Ping", 0.0, 1.0, Some(1.0))
}

fn soundboard_rules(rig: &Rig, everyone_can_play: bool, rate_limited: bool) {
    rig.db()
        .execute(
            "UPDATE \"Servers\" SET \"SoundboardEveryoneCanPlay\" = ?1, \"SoundboardRateLimited\" = ?2",
            params![everyone_can_play, rate_limited],
        )
        .unwrap();
}

#[tokio::test]
async fn the_hub_schedules_a_sound_for_everyone_seated_and_enforces_the_cooldown() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    soundboard_rules(&rig, true, true);
    let sound = add_sound(&rig, rig.server_id, "Ping", 0.5, 2.5, Some(3.0));
    let (mut player, mut listener) = (rig.connect(&maja).await, rig.connect(&ada).await);
    player.join(rig.salon).await;
    listener.join(rig.salon).await;
    let clock = player.ok("GetRtcClock", json!([])).await.as_i64().unwrap();

    player.ok("PlaySoundboard", json!([rig.salon, sound])).await;

    for peer in [&mut player, &mut listener] {
        let played = peer.next("SoundboardPlayed").await;
        assert_eq!(played["channelId"], rig.salon.to_string().as_str());
        assert_eq!(played["triggeredByUserId"], maja.id.as_str());
        assert!(Id::parse(played["playbackId"].as_str().unwrap()).is_some());
        let starts = played["startsAtUnixMs"].as_i64().unwrap();
        assert!(starts >= clock + 700 && starts <= clock + 5_000, "{starts} against {clock}");
        let attachment: String = rig
            .db()
            .query_row("SELECT \"AttachmentId\" FROM \"SoundboardSounds\" WHERE \"Id\" = ?1", [sound], |row| row.get(0))
            .unwrap();
        let expected = format!(
            "{{\"id\":\"{sound}\",\"name\":\"Ping\",\"glyph\":\"🔔\",\"url\":\"/assets/{}\",\"durationSeconds\":3,\"trimStartSeconds\":0.5,\"trimEndSeconds\":2.5}}",
            attachment.to_lowercase()
        );
        assert_eq!(played["sound"], serde_json::from_str::<Value>(&expected).unwrap());
    }
    player.refused("PlaySoundboard", json!([rig.salon, sound]), "Soundboard cooldown is active.").await;
    // The cooldown is the person's on that server, whatever connection asks.
    let mut tab = rig.connect(&maja).await;
    tab.join(rig.salon).await;
    tab.refused("PlaySoundboard", json!([rig.salon, sound]), "Soundboard cooldown is active.").await;
    assert!(listener.drain("SoundboardPlayed").await.is_empty());
}

#[tokio::test]
async fn a_clip_is_written_on_the_wire_in_the_order_and_numbers_the_first_server_wrote_them() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    soundboard_rules(&rig, true, false);
    let sound = add_sound(&rig, rig.server_id, "Ping", 0.0, 2.0, None);
    let mut player = rig.connect(&maja).await;
    player.join(rig.salon).await;

    let call = json!({"type": 1, "invocationId": "99", "target": "PlaySoundboard", "arguments": [rig.salon, sound]});
    player.hub.send_text(&format!("{call}\u{1e}")).await;

    let text = loop {
        let raw = player.hub.raw_record().await;
        if raw.contains("\"SoundboardPlayed\"") {
            break raw;
        }
    };
    let order: Vec<usize> = ["playbackId", "channelId", "triggeredByUserId", "startsAtUnixMs", "\"sound\"", "durationSeconds", "trimStartSeconds", "trimEndSeconds"]
        .iter()
        .map(|name| text.find(name).unwrap_or_else(|| panic!("{name} in {text}")))
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    // Whole numbers have no point, and a clip without a measured length is as long as its trim.
    assert!(text.contains("\"durationSeconds\":2,\"trimStartSeconds\":0,\"trimEndSeconds\":2}"), "{text}");
}

#[tokio::test]
async fn only_staff_play_when_everyone_can_play_is_off() {
    let rig = Rig::start().await;
    let (member, moderator, manager) =
        (rig.person("ilya").await, rig.person("ada").await, rig.person("nils").await);
    soundboard_rules(&rig, false, false);
    rig.give_role(&moderator, "Moderators", 3, MODERATE_MEMBERS);
    rig.give_role(&manager, "Managers", 4, crate::voice::MANAGE_SERVER);
    let sound = add_sound(&rig, rig.server_id, "Ping", 0.0, 1.0, Some(1.0));
    let (mut plain, mut staff, mut chief) =
        (rig.connect(&member).await, rig.connect(&moderator).await, rig.connect(&manager).await);
    plain.join(rig.salon).await;
    staff.join(rig.salon).await;
    chief.join(rig.salon).await;

    plain.refused("PlaySoundboard", json!([rig.salon, sound]), "Soundboard playback is restricted.").await;
    staff.ok("PlaySoundboard", json!([rig.salon, sound])).await;
    chief.ok("PlaySoundboard", json!([rig.salon, sound])).await;

    // The member is in the room, so they hear what the moderator played.
    assert_eq!(plain.next("SoundboardPlayed").await["sound"]["id"], sound.to_string().as_str());
    assert_eq!(plain.next("SoundboardPlayed").await["triggeredByUserId"], manager.id.as_str());
    assert!(plain.drain("SoundboardPlayed").await.is_empty());
}

#[tokio::test]
async fn everyone_plays_when_the_server_allows_it_and_the_cooldown_is_off() {
    let rig = Rig::start().await;
    let ilya = rig.person("ilya").await;
    soundboard_rules(&rig, true, false);
    let sound = add_sound(&rig, rig.server_id, "Ping", 0.0, 1.0, Some(1.0));
    let mut peer = rig.connect(&ilya).await;
    peer.join(rig.salon).await;

    peer.ok("PlaySoundboard", json!([rig.salon, sound])).await;
    peer.ok("PlaySoundboard", json!([rig.salon, sound])).await;

    peer.next("SoundboardPlayed").await;
    peer.next("SoundboardPlayed").await;
}

#[tokio::test]
async fn playing_needs_a_seat_in_the_voice_channel_and_a_sound_of_the_server() {
    let rig = Rig::start().await;
    let (nils, jonas) = (rig.person("nils").await, rig.person("jonas").await);
    soundboard_rules(&rig, true, false);
    let sound = add_sound(&rig, rig.server_id, "Ping", 0.0, 1.0, Some(1.0));
    let (other, elsewhere) = crate::voice::add_server(&rig.db(), &jonas.id);
    let foreign_sound = {
        let (attachment, id) = (Id::random(), Id::random());
        let db = rig.db();
        db.execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"IsVoiceMessage\", \"IsEncrypted\", \"CreatedAt\") VALUES (?1, ?2, ?1, 's.wav', 'audio/wav', 1, 0, 0, ?3)",
            params![attachment, Id::parse(&jonas.id).unwrap(), Timestamp::now()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO \"SoundboardSounds\" (\"Id\", \"ServerId\", \"AttachmentId\", \"CreatedById\", \"Name\", \"Glyph\", \"TrimStartSeconds\", \"TrimEndSeconds\", \"Position\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, 'Far', 'x', 0, 1, 0, ?5)",
            params![id, other, attachment, Id::parse(&jonas.id).unwrap(), Timestamp::now()],
        )
        .unwrap();
        id
    };
    let mut peer = rig.connect(&nils).await;

    peer.refused("PlaySoundboard", json!([rig.salon, sound]), "Join the voice channel before playing a sound.").await;
    peer.join(rig.salon).await;
    peer.refused("PlaySoundboard", json!([rig.salon, Id::random()]), "Unknown soundboard sound.").await;
    peer.refused("PlaySoundboard", json!([rig.salon, foreign_sound]), "Unknown soundboard sound.").await;
    peer.refused("PlaySoundboard", json!(["x", sound]), "Invalid channelId.").await;
    peer.refused("PlaySoundboard", json!([null, sound]), "Invalid channelId.").await;
    peer.refused("PlaySoundboard", json!([rig.salon, "x"]), "Invalid soundId.").await;
    peer.refused("PlaySoundboard", json!([rig.salon, null]), "Invalid soundId.").await;
    peer.does_not_fit("PlaySoundboard", json!([rig.salon])).await;
    peer.does_not_fit("PlaySoundboard", json!([rig.salon, sound, sound])).await;
    peer.does_not_fit("PlaySoundboard", json!([5, sound])).await;
    // A room of another server is not one the account is a member of; a direct
    // call is not a voice channel.
    peer.refused("PlaySoundboard", json!([elsewhere, sound]), "Join the voice channel before playing a sound.").await;
    peer.join(rig.salon).await;
    assert!(peer.drain("SoundboardPlayed").await.is_empty());
}

#[tokio::test]
async fn an_arrival_sound_plays_to_the_room_cut_to_five_seconds_and_not_twice_in_half_a_minute() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let sound = add_sound(&rig, rig.server_id, "Hello", 1.0, 20.0, Some(20.0));
    set_preference(&rig.db(), &maja.id, "\"ShowPresence\"", true);
    rig.db()
        .execute("UPDATE \"UserPreferences\" SET \"JoinSoundId\" = ?1 WHERE \"UserId\" = ?2", params![sound, Id::parse(&maja.id).unwrap()])
        .unwrap();
    let (mut arriving, mut listener) = (rig.connect(&maja).await, rig.connect(&ada).await);
    listener.join(rig.salon).await;

    arriving.join(rig.salon).await;

    for peer in [&mut arriving, &mut listener] {
        let played = peer.next("JoinSoundPlayed").await;
        assert_eq!(played["triggeredByUserId"], maja.id.as_str());
        assert_eq!(played["channelId"], rig.salon.to_string().as_str());
        assert_eq!(played["sound"]["id"], sound.to_string().as_str());
        assert_eq!(played["sound"]["trimStartSeconds"].as_f64(), Some(1.0));
        assert_eq!(played["sound"]["trimEndSeconds"].as_f64(), Some(6.0));
        assert_eq!(played["sound"]["durationSeconds"].as_f64(), Some(20.0));
    }
    // The cooldown skips the arrival without blocking the join.
    arriving.leave().await;
    arriving.join(rig.salon).await;
    rig.occupants_become(rig.salon, &[&maja.id, &ada.id]).await;
    assert!(listener.drain("JoinSoundPlayed").await.is_empty());
}

#[tokio::test]
async fn an_arrival_sound_of_a_server_the_account_left_is_silent_and_a_room_of_one_hears_itself() {
    let rig = Rig::start().await;
    let (maja, jonas) = (rig.person("maja").await, rig.person("jonas").await);
    let (other, _) = crate::voice::add_server(&rig.db(), &maja.id);
    let (attachment, sound) = (Id::random(), Id::random());
    {
        let db = rig.db();
        let uploader = Id::parse(&jonas.id).unwrap();
        db.execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"IsVoiceMessage\", \"IsEncrypted\", \"CreatedAt\") VALUES (?1, ?2, ?1, 's.wav', 'audio/wav', 1, 0, 0, ?3)",
            params![attachment, uploader, Timestamp::now()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO \"SoundboardSounds\" (\"Id\", \"ServerId\", \"AttachmentId\", \"CreatedById\", \"Name\", \"Glyph\", \"TrimStartSeconds\", \"TrimEndSeconds\", \"Position\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, 'Far', 'x', 0, 2, 0, ?5)",
            params![sound, other, attachment, uploader, Timestamp::now()],
        )
        .unwrap();
        set_preference(&db, &maja.id, "\"ShowPresence\"", true);
        db.execute("UPDATE \"UserPreferences\" SET \"JoinSoundId\" = ?1 WHERE \"UserId\" = ?2", params![sound, Id::parse(&maja.id).unwrap()]).unwrap();
    }
    let mut peer = rig.connect(&maja).await;

    // A clip of a server the account belongs to is a greeting; joining plays it to the one in the room.
    peer.join(rig.salon).await;
    let played = peer.next("JoinSoundPlayed").await;
    assert_eq!(played["sound"]["durationSeconds"].as_f64(), Some(2.0));
    // Leaving that server takes the greeting with it.
    rig.db()
        .execute("DELETE FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2", params![other, Id::parse(&maja.id).unwrap()])
        .unwrap();
    peer.leave().await;
    peer.join(rig.salon).await;
    assert!(peer.drain("JoinSoundPlayed").await.is_empty());
}

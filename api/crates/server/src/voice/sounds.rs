//! Clips played into a voice room, by the soundboard and as an arrival.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, Serializer};
use serde_json::Value;
use stellarr_hub::HubError;

use super::hub::{internal, parse_id, send_to, visible};
use crate::database::DbError;
use crate::permissions::visibility::ChannelKind;
use crate::realtime::method::{Context, arguments};
use crate::values::{Id, Timestamp};

/// How far ahead a clip is scheduled: long enough for every client to fetch,
/// decode and cache the audio, short enough that the sound still belongs to the
/// moment it announces.
const PLAYBACK_LEAD_MS: i64 = 750;

const SOUNDBOARD_COOLDOWN: Duration = Duration::from_secs(5);

/// How soon after an announcement the same person's next one is silent. It
/// stops a rejoin loop from turning into a soundboard nobody can mute; the join
/// itself is never delayed or refused.
pub const JOIN_SOUND_COOLDOWN: Duration = Duration::from_secs(30);

/// A number of seconds written as the first server wrote it: a whole number
/// without a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seconds(pub f64);

impl Serialize for Seconds {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = if self.0.is_finite() { self.0 } else { 0.0 };
        crate::uploads::json_number(&Some(value), serializer)
    }
}

/// A clip as a room is told of it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundDto {
    pub id: Id,
    pub name: String,
    pub glyph: String,
    pub url: String,
    pub duration_seconds: Seconds,
    pub trim_start_seconds: Seconds,
    pub trim_end_seconds: Seconds,
}

impl SoundDto {
    /// The same clip cut to the window an arrival is allowed to occupy. The
    /// picker and the room are handed the identical shortened trim, so what
    /// somebody auditions in settings is exactly what a channel hears.
    pub fn for_arrival(mut self) -> SoundDto {
        let start = self.trim_start_seconds.0;
        self.trim_end_seconds = Seconds(crate::own_profile::join_sound::window_end(start, self.trim_end_seconds.0));
        self
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Played<'a> {
    playback_id: Id,
    channel_id: Id,
    triggered_by_user_id: Id,
    starts_at_unix_ms: i64,
    sound: &'a SoundDto,
}

fn scheduled<'a>(room: Id, user: Id, sound: &'a SoundDto) -> Played<'a> {
    Played {
        playback_id: Id::random(),
        channel_id: room,
        triggered_by_user_id: user,
        starts_at_unix_ms: Timestamp::now().unix_millis() + PLAYBACK_LEAD_MS,
        sound,
    }
}

/// The row of a clip, with the length of its audio, when it is `server`'s (or
/// of any server where `server` is none and `member` belongs).
fn read_sound(
    db: &Connection,
    sound: Id,
    server: Option<Id>,
    member: Option<Id>,
) -> rusqlite::Result<Option<SoundDto>> {
    db.query_row(
        "SELECT s.\"Id\", s.\"Name\", s.\"Glyph\", s.\"AttachmentId\", a.\"DurationSeconds\", s.\"TrimStartSeconds\", s.\"TrimEndSeconds\"
         FROM \"SoundboardSounds\" s JOIN \"ImageAttachments\" a ON a.\"Id\" = s.\"AttachmentId\"
         WHERE s.\"Id\" = ?1
           AND (?2 IS NULL OR s.\"ServerId\" = ?2)
           AND (?3 IS NULL OR EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = s.\"ServerId\" AND m.\"UserId\" = ?3))",
        params![sound, server, member],
        |row| {
            let attachment: Id = row.get(3)?;
            let duration: Option<f64> = row.get(4)?;
            let end: f64 = row.get(6)?;
            Ok(SoundDto {
                id: row.get(0)?,
                name: row.get(1)?,
                glyph: row.get(2)?,
                url: format!("/assets/{attachment}"),
                duration_seconds: Seconds(duration.unwrap_or(end)),
                trim_start_seconds: Seconds(row.get(5)?),
                trim_end_seconds: Seconds(end),
            })
        },
    )
    .optional()
}

enum Refusal {
    UnknownChannel,
    UnknownSound,
    Restricted,
}

/// Plays one of the server's clips for everyone seated in a voice channel. The
/// start is scheduled a fixed lead ahead and shared by all of them, so that
/// clients start against the RTC clock and not against packet arrival.
pub(super) async fn play_soundboard(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [channel, sound] = arguments::exactly(arguments)?;
    let channel = arguments::text(channel)?;
    let sound = arguments::text(sound)?;
    let room = parse_id(channel.as_deref(), "channelId")?;
    let clip = parse_id(sound.as_deref(), "soundId")?;
    let user = cx.user();
    if !cx
        .realtime()
        .presence()
        .is_connection_in_voice(cx.connection_id(), room)
    {
        return Err(visible("Join the voice channel before playing a sound."));
    }

    let (server, rate_limited, found) = cx
        .state()
        .database()
        .read(move |db| -> Result<_, DbError> {
            let server: Option<Id> = db
                .query_row(
                    "SELECT c.\"ServerId\" FROM \"Channels\" c WHERE c.\"Id\" = ?1 AND c.\"Kind\" = ?2
                       AND EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = c.\"ServerId\" AND m.\"UserId\" = ?3)",
                    params![room, ChannelKind::Voice, user],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(server) = server else {
                return Ok((None, false, Err(Refusal::UnknownChannel)));
            };
            let rules: Option<(bool, bool)> = db
                .query_row(
                    "SELECT \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\" FROM \"Servers\" WHERE \"Id\" = ?1",
                    [server],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let found = read_sound(db, clip, Some(server), None)?;
            let (Some((everyone_can_play, rate_limited)), Some(found)) = (rules, found) else {
                return Ok((Some(server), false, Err(Refusal::UnknownSound)));
            };
            if !crate::soundboard::can_play(db, user, server, everyone_can_play)? {
                return Ok((Some(server), rate_limited, Err(Refusal::Restricted)));
            }
            Ok((Some(server), rate_limited, Ok(found)))
        })
        .await
        .map_err(internal)?;
    let found = found.map_err(|refusal| match refusal {
        Refusal::UnknownChannel => visible("Unknown voice channel."),
        Refusal::UnknownSound => visible("Unknown soundboard sound."),
        Refusal::Restricted => visible("Soundboard playback is restricted."),
    })?;
    let server = server.ok_or_else(|| internal("a clip without a server"))?;

    if rate_limited
        && !cx
            .state()
            .voice()
            .playback()
            .try_acquire(user, server, SOUNDBOARD_COOLDOWN)
    {
        return Err(visible("Soundboard cooldown is active."));
    }

    let seated = cx.realtime().presence().voice_connections(room);
    send_to(&cx, seated, "SoundboardPlayed", &scheduled(room, user, &found));
    Ok(Value::Null)
}

/// Plays the joiner's own clip to the room they just walked into. It rides the
/// soundboard's envelope, so the joiner hears it too and the setting is
/// something one can check rather than take on trust. The clip is cut to
/// [`crate::own_profile::join_sound::MAX_SECONDS`] here and not in the browser, because how long an
/// arrival may last is a rule of the room and not a preference of whoever
/// listens.
pub(super) async fn announce_join_sound(cx: &Context, room: Id, user: Id) -> Result<(), DbError> {
    let found = cx
        .state()
        .database()
        .read(move |db| -> Result<Option<SoundDto>, DbError> {
            let chosen: Option<Option<Id>> = db
                .query_row(
                    "SELECT \"JoinSoundId\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
                    [user],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(Some(chosen)) = chosen else {
                return Ok(None);
            };
            // Leaving the server a clip came from takes the greeting with it:
            // nobody in the room could fetch the audio afterwards, so a silent
            // arrival is the honest outcome.
            Ok(read_sound(db, chosen, None, Some(user))?)
        })
        .await?;
    let Some(found) = found else {
        return Ok(());
    };
    if !cx
        .state()
        .voice()
        .playback()
        .try_acquire(user, Id::from_bytes([0; 16]), JOIN_SOUND_COOLDOWN)
    {
        return Ok(());
    }

    let arrival = found.for_arrival();
    let seated = cx.realtime().presence().voice_connections(room);
    send_to(cx, seated, "JoinSoundPlayed", &scheduled(room, user, &arrival));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(start: f64, end: f64) -> SoundDto {
        SoundDto {
            id: Id::random(),
            name: "n".into(),
            glyph: "g".into(),
            url: "/assets/x".into(),
            duration_seconds: Seconds(end),
            trim_start_seconds: Seconds(start),
            trim_end_seconds: Seconds(end),
        }
    }

    #[test]
    fn seconds_are_written_without_a_point_when_they_are_whole() {
        let text = serde_json::to_string(&clip(0.0, 2.5)).unwrap();
        assert!(text.contains("\"durationSeconds\":2.5,\"trimStartSeconds\":0,\"trimEndSeconds\":2.5"), "{text}");
        let whole = serde_json::to_string(&Seconds(15.0)).unwrap();
        assert_eq!(whole, "15");
        assert_eq!(serde_json::to_string(&Seconds(-0.0)).unwrap(), "0");
        assert_eq!(serde_json::to_string(&Seconds(f64::NAN)).unwrap(), "0");
    }

    #[test]
    fn an_arrival_is_cut_to_five_seconds_from_where_the_clip_starts() {
        assert_eq!(clip(0.0, 12.0).for_arrival().trim_end_seconds, Seconds(5.0));
        assert_eq!(clip(2.0, 12.0).for_arrival().trim_end_seconds, Seconds(7.0));
        assert_eq!(clip(2.0, 3.0).for_arrival().trim_end_seconds, Seconds(3.0));
    }
}

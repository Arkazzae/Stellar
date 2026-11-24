//! A server with its two system roles, its starter channels and its
//! notification defaults: what the first account founds the instance with, and
//! what a new server starts as.

use rusqlite::{Transaction, params};
use serde::Deserialize;
use stellarr_messages::{MessageKey, Messages};

use super::access::{ALL, BASE_ROLE_DEFAULTS};
use crate::database::DbError;
use crate::http::{ApiError, Problem};
use crate::values::{Id, Timestamp};

const MAX_NAME_LENGTH: usize = 80;
const MAX_DESCRIPTION_LENGTH: usize = 280;
const MAX_CHANNEL_NAME_LENGTH: usize = 60;

/// What a person asks for when they create a server, or found the instance.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub is_public: bool,
    pub text_channel_name: Option<String>,
    pub voice_channel_name: Option<String>,
    pub mentions_only: Option<bool>,
    pub voice_join_sound: Option<bool>,
}

/// A request that has been checked.
#[derive(Debug, Clone)]
pub struct Starter {
    name: String,
    description: String,
    is_public: bool,
    text_channel: String,
    voice_channel: Option<String>,
    mentions_only: bool,
    voice_join_sound: bool,
}

/// A channel name as it is stored: trimmed and in lower case, letter by
/// letter.
pub fn normalize_channel_name(name: Option<&str>) -> String {
    name.map_or_else(String::new, |name| stellarr_unicode::to_lower(name.trim()))
}

pub fn is_valid_channel_name(name: &str) -> bool {
    let length = name.encode_utf16().count();
    (1..=MAX_CHANNEL_NAME_LENGTH).contains(&length)
        && name
            .chars()
            .all(|c| stellarr_unicode::is_letter_or_digit(c) || c == '-')
}

/// Checks a request. The same checks serve the first registration and every
/// later server.
pub fn prepare(request: &ServerRequest, messages: &Messages) -> Result<Starter, ApiError> {
    let name = request.name.as_deref().unwrap_or("").trim().to_string();
    let description = request
        .description
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if !(1..=MAX_NAME_LENGTH).contains(&name.encode_utf16().count())
        || description.encode_utf16().count() > MAX_DESCRIPTION_LENGTH
    {
        return Err(Problem::invalid_field(
            "server",
            messages.get(MessageKey::ServerNameOrDescriptionInvalid, &[])?,
        ));
    }
    // An omitted channel name keeps the usual start. A voice name that is
    // given but empty is how a server is created without a voice channel.
    let default_text = messages.get(MessageKey::DefaultTextChannelName, &[])?;
    let default_voice = messages.get(MessageKey::DefaultVoiceChannelName, &[])?;
    let text_channel = normalize_channel_name(Some(
        request
            .text_channel_name
            .as_deref()
            .unwrap_or(&default_text),
    ));
    let voice_channel = normalize_channel_name(Some(
        request
            .voice_channel_name
            .as_deref()
            .unwrap_or(&default_voice),
    ));
    let wants_voice = !voice_channel.is_empty();
    if !is_valid_channel_name(&text_channel)
        || (wants_voice && !is_valid_channel_name(&voice_channel))
    {
        return Err(Problem::invalid_field(
            "channels",
            messages.get(MessageKey::StarterChannelNameInvalid, &[])?,
        ));
    }
    if wants_voice && text_channel == voice_channel {
        return Err(Problem::invalid_field(
            "channels",
            messages.get(MessageKey::StarterChannelNamesCollide, &[])?,
        ));
    }
    Ok(Starter {
        name,
        description,
        is_public: request.is_public,
        text_channel,
        voice_channel: wants_voice.then_some(voice_channel),
        mentions_only: request.mentions_only.unwrap_or(true),
        voice_join_sound: request.voice_join_sound.unwrap_or(true),
    })
}

/// What `add` made.
#[derive(Debug, Clone, Copy)]
pub struct NewServer {
    pub id: Id,
    pub owner_role: Id,
    pub first_text_channel: Id,
}

/// Writes a server with its owner role, its base role, its channels and its
/// notification defaults. The caller adds the first member, who owns it.
pub fn add(
    tx: &Transaction,
    starter: &Starter,
    messages: &Messages,
    now: Timestamp,
) -> Result<NewServer, DbError> {
    let server = Id::random();
    let owner_role = Id::random();
    let member_role = Id::random();
    tx.execute(
        "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"IconUrl\", \"CreatedAt\",
                                \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, 1, 1)",
        params![server, starter.name, starter.description, starter.is_public, now],
    )?;
    let role = |id: Id,
                name: MessageKey,
                color: &str,
                permissions: i64,
                position: i64,
                owner: bool|
     -> Result<(), DbError> {
        let name = messages
            .get(name, &[])
            .map_err(|e| DbError::Task(e.to_string()))?;
        tx.execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![id, server, name, color, permissions, position, owner],
        )?;
        Ok(())
    };
    role(
        owner_role,
        MessageKey::OwnerRoleName,
        "oklch(0.72 0.16 15)",
        ALL,
        0,
        true,
    )?;
    role(
        member_role,
        MessageKey::MemberRoleName,
        "oklch(0.70 0.02 277)",
        BASE_ROLE_DEFAULTS,
        1,
        false,
    )?;
    let first_text_channel = Id::random();
    let channel = |id: Id, name: &str, kind: i64, position: i64| -> Result<(), DbError> {
        tx.execute(
            "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\",
                                     \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
             VALUES (?1, ?2, ?3, ?4, ?5, '', 0, 0, 0, NULL)",
            params![id, server, name, kind, position],
        )?;
        Ok(())
    };
    channel(first_text_channel, &starter.text_channel, 0, 0)?;
    if let Some(voice) = &starter.voice_channel {
        channel(Id::random(), voice, 1, 1)?;
    }
    tx.execute(
        "INSERT INTO \"ServerAdminSettings\" (\"ServerId\", \"AllMessages\", \"MentionsOnly\", \"VoiceJoinSound\",
                                             \"MutedChannelIdsJson\", \"ServerMuted\")
         VALUES (?1, ?2, ?3, ?4, '[]', 0)",
        params![server, !starter.mentions_only, starter.mentions_only, starter.voice_join_sound],
    )?;
    Ok(NewServer {
        id: server,
        owner_role,
        first_text_channel,
    })
}

#[cfg(test)]
mod tests {
    use stellarr_messages::Language;

    use super::*;

    fn english() -> Messages {
        Messages::new(Language::English)
    }

    fn request(name: &str, text: Option<&str>, voice: Option<&str>) -> ServerRequest {
        ServerRequest {
            name: Some(name.into()),
            text_channel_name: text.map(Into::into),
            voice_channel_name: voice.map(Into::into),
            ..ServerRequest::default()
        }
    }

    fn refused_field(result: Result<Starter, ApiError>) -> String {
        match result.unwrap_err() {
            ApiError::Problem(problem) => serde_json::to_value(&problem).unwrap()["errors"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_omitted_channel_name_keeps_the_usual_start_and_an_empty_voice_name_means_none() {
        let usual = prepare(&request("Crew", None, None), &english()).unwrap();
        let silent = prepare(&request("Crew", Some("lobby"), Some("")), &english()).unwrap();

        assert_eq!(
            (usual.text_channel.as_str(), usual.voice_channel.as_deref()),
            ("general", Some("voice"))
        );
        assert_eq!(
            (silent.text_channel.as_str(), silent.voice_channel),
            ("lobby", None)
        );
    }

    #[test]
    fn names_are_trimmed_lowered_and_checked() {
        assert_eq!(
            prepare(&request("  Crew ", Some(" Lobby "), None), &english())
                .unwrap()
                .text_channel,
            "lobby"
        );
        assert_eq!(
            refused_field(prepare(&request("", None, None), &english())),
            "server"
        );
        assert_eq!(
            refused_field(prepare(&request(&"x".repeat(81), None, None), &english())),
            "server"
        );
        assert_eq!(
            refused_field(prepare(
                &request("Crew", Some("two words"), None),
                &english()
            )),
            "channels"
        );
        assert_eq!(
            refused_field(prepare(
                &request("Crew", Some("same"), Some("same")),
                &english()
            )),
            "channels"
        );
        assert_eq!(
            refused_field(prepare(
                &request("Crew", Some(&"c".repeat(61)), None),
                &english()
            )),
            "channels"
        );
        assert!(prepare(&request("Crew", Some("ogólny"), Some("głos-1")), &english()).is_ok());
    }

    #[test]
    fn a_description_is_at_most_280_characters() {
        let mut long = request("Crew", None, None);
        long.description = Some("d".repeat(281));
        let mut fits = request("Crew", None, None);
        fits.description = Some("d".repeat(280));

        assert_eq!(refused_field(prepare(&long, &english())), "server");
        assert!(prepare(&fits, &english()).is_ok());
    }

    /// What the first server stored for a starter channel of each name, read
    /// from its database after creating a server with it.
    #[test]
    fn channel_names_are_lowered_letter_by_letter_as_the_first_server_does() {
        for (given, stored) in [
            ("  ABC  ", "abc"),
            ("I", "i"),
            ("\u{130}", "\u{130}"),
            ("\u{130}i", "\u{130}i"),
            ("\u{131}i", "\u{131}i"),
            ("\u{df}", "\u{df}"),
            ("\u{1e9e}", "\u{df}"),
            ("\u{3a3}", "\u{3c3}"),
            (
                "\u{39f}\u{394}\u{39f}\u{3a3}",
                "\u{3bf}\u{3b4}\u{3bf}\u{3c3}",
            ),
            ("\u{13a0}", "\u{ab70}"),
            ("\u{13a1}", "\u{ab71}"),
            ("\u{13f8}", "\u{13f8}"),
            ("\u{13f0}", "\u{13f8}"),
            ("\u{1c90}", "\u{10d0}"),
            ("\u{10a0}", "\u{2d00}"),
            ("\u{10d0}", "\u{10d0}"),
            ("\u{1c5}", "\u{1c6}"),
            ("\u{1c4}", "\u{1c6}"),
            ("\u{13b0}2", "\u{ab80}2"),
        ] {
            assert_eq!(normalize_channel_name(Some(given)), stored, "{given:?}");
        }
        assert_eq!(normalize_channel_name(None), "");
    }
}

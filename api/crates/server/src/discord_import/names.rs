//! How Discord's names, ids and numbers fit Stellarr's rules.

use std::collections::HashSet;

use aws_lc_rs::digest;
use stellarr_unicode::{is_letter_or_digit, to_lower};

use crate::values::Id;

/// What Stellarr's endpoints allow; the import keeps to the same.
pub const MAX_CHANNEL_NAME: usize = 60;
pub const MAX_SECTION_NAME: usize = 60;
pub const MAX_TOPIC: usize = 1024;
pub const MAX_SLOW_MODE_SECONDS: i32 = 6 * 60 * 60;
pub const MAX_USER_LIMIT: i32 = 99;
pub const MAX_DISPLAY_NAME: usize = 64;

/// The username of a placeholder, which registration cannot produce: a colon
/// is not a name character.
pub fn placeholder_username(discord_id: &str) -> String {
    format!("discord:{discord_id}")
}

/// The Stellarr id of an imported message: derived from the server and
/// Discord's id, so the same message written twice is the same row, and a page
/// read again inserts nothing.
pub fn message_id(server: Id, discord_message_id: &str) -> Id {
    derived_id(format!(
        "stellarr.discord.message/{}/{discord_message_id}",
        crate::uploads::compact(server)
    ))
}

pub(super) fn attachment_id(message: Id, discord_attachment_id: &str) -> Id {
    derived_id(format!(
        "stellarr.discord.attachment/{message}/{discord_attachment_id}"
    ))
}

fn derived_id(input: String) -> Id {
    let hash = digest::digest(&digest::SHA256, input.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_ref()[..16]);
    // RFC 9562's version 8, for an id made by a rule of one's own.
    bytes[6] = (bytes[6] & 0x0F) | 0x80;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    Id::from_bytes(bytes)
}

/// A channel name Stellarr accepts: lower case, letters, digits and dashes, at
/// most 60 characters. Anything else becomes a dash, as Discord itself writes
/// spaces in a text channel's name.
pub fn channel_name(discord_name: &str) -> String {
    let mut name = String::new();
    let mut dash = false;
    for c in to_lower(discord_name.trim()).chars() {
        // A character outside the basic plane is two units that are neither a
        // letter nor a digit.
        if is_letter_or_digit(c) && u32::from(c) <= 0xFFFF {
            name.push(c);
            dash = false;
        } else if !dash && !name.is_empty() {
            name.push('-');
            dash = true;
        }
    }
    let text = name.trim_end_matches('-');
    let text = if text.chars().count() > MAX_CHANNEL_NAME {
        text.chars()
            .take(MAX_CHANNEL_NAME)
            .collect::<String>()
            .trim_end_matches('-')
            .to_string()
    } else {
        text.to_string()
    };
    if text.is_empty() {
        "channel".to_string()
    } else {
        text
    }
}

/// The name, made unique among `taken` with a number, as a second "general"
/// becomes "general-2".
pub fn unique(name: String, taken: &mut HashSet<String>) -> String {
    if taken.insert(name.clone()) {
        return name;
    }
    let mut number = 2;
    loop {
        let suffix = format!("-{number}");
        let units = name.chars().count();
        let candidate = if units + suffix.len() > MAX_CHANNEL_NAME {
            let cut: String = name.chars().take(MAX_CHANNEL_NAME - suffix.len()).collect();
            format!("{cut}{suffix}")
        } else {
            format!("{name}{suffix}")
        };
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        number += 1;
    }
}

pub fn section_name(discord_name: &str) -> String {
    let name = clamp(discord_name.trim(), MAX_SECTION_NAME);
    if name.is_empty() {
        "Category".to_string()
    } else {
        name
    }
}

pub fn topic(topic: Option<&str>) -> String {
    clamp(topic.map_or("", str::trim), MAX_TOPIC)
}

pub fn slow_mode(seconds: i32) -> i32 {
    seconds.clamp(0, MAX_SLOW_MODE_SECONDS)
}

/// A voice channel's limit. A stage allows thousands, which in Stellarr is no
/// limit at all rather than the 99 it allows at most.
pub fn user_limit(limit: i32) -> i32 {
    if (1..=MAX_USER_LIMIT).contains(&limit) {
        limit
    } else {
        0
    }
}

/// The name a placeholder shows: the nickname on the server, else the name
/// shown everywhere on Discord, else the username, else the id itself.
pub fn display_name(id: &str, candidates: &[Option<&str>]) -> String {
    candidates
        .iter()
        .map(|name| clamp(name.map_or("", str::trim), MAX_DISPLAY_NAME))
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// The earlier of two snowflakes, the first being the one held so far.
pub fn earlier(current: Option<String>, id: &str) -> String {
    match current {
        Some(current) if stellarr_discord::compare_ids(id, &current).is_ge() => current,
        _ => id.to_string(),
    }
}

/// At most `length` UTF-16 units, never cutting a surrogate pair in two.
pub fn clamp(text: &str, length: usize) -> String {
    stellarr_unicode::utf16::prefix(text, length).to_string()
}

/// A length as the first server counted it, in UTF-16 units.
pub fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_made_of_letters_digits_and_single_dashes() {
        assert_eq!(channel_name("  General Chat!! "), "general-chat");
        assert_eq!(channel_name("żółć 2"), "żółć-2");
        assert_eq!(channel_name("😀😀"), "channel");
        assert_eq!(channel_name("---"), "channel");
        assert_eq!(channel_name(&"a".repeat(80)).len(), 60);
        assert_eq!(
            channel_name(&format!("{} b", "a".repeat(59))),
            "a".repeat(59)
        );
    }

    #[test]
    fn a_taken_name_gets_a_number_that_still_fits() {
        let mut taken: HashSet<String> = HashSet::new();
        let long = "a".repeat(60);
        assert_eq!(unique(long.clone(), &mut taken), long);
        let second = unique(long.clone(), &mut taken);
        assert_eq!(second.chars().count(), 60);
        assert!(second.ends_with("-2"));
        assert_eq!(unique("general".into(), &mut taken), "general");
        assert_eq!(unique("general".into(), &mut taken), "general-2");
        assert_eq!(unique("general".into(), &mut taken), "general-3");
    }

    #[test]
    fn a_text_is_cut_by_units_and_never_inside_a_pair() {
        assert_eq!(clamp("abcd", 3), "abc");
        assert_eq!(clamp("ab😀", 3), "ab");
        assert_eq!(clamp("ab😀", 4), "ab😀");
        assert_eq!(section_name("   "), "Category");
        assert_eq!(display_name("7", &[None, Some("  "), Some(" Ola ")]), "Ola");
        assert_eq!(display_name("7", &[None]), "7");
    }

    #[test]
    fn the_oldest_snowflake_wins_as_a_number() {
        assert_eq!(earlier(None, "9"), "9");
        assert_eq!(earlier(Some("9".into()), "10"), "9");
        assert_eq!(earlier(Some("10".into()), "9"), "9");
    }

    #[test]
    fn a_message_id_is_made_from_the_server_and_discords_id() {
        let server = Id::parse("3d82da44-8aa5-4bdf-9ee1-2902df99880c").unwrap();
        let id = message_id(server, "123");
        assert_eq!(id, message_id(server, "123"));
        assert_ne!(id, message_id(server, "124"));
        let text = id.to_string();
        assert_eq!(&text[14..15], "8");
        assert!(matches!(&text[19..20], "8" | "9" | "a" | "b"));
    }
}

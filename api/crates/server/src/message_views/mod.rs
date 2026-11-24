//! What is done with messages that already exist, and the other ways of
//! reading them: reactions, pins, threads, and searching a channel.
//!
//! `PUT` and `DELETE /api/messages/{messageId}/reactions/{emoji}`, `GET
//! /api/channels/{channelId}/pins`, `PUT` and `DELETE
//! /api/messages/{messageId}/pin`, `GET /api/channels/{channelId}/threads`
//! and `.../threads/{rootId}`, and `GET /api/channels/{channelId}/messages/search`
//! and `.../messages/index`.
//!
//! A message is written by [`crate::messages::dto`], as everywhere.

mod cursor;
mod index;
mod ordinal_case;
mod pins;
mod reactions;
mod search;
mod text;
mod threads;

use crate::http::Endpoints;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .put("/api/messages/{messageId}/reactions/{emoji}", reactions::add)
        .authorized()
        .delete("/api/messages/{messageId}/reactions/{emoji}", reactions::remove)
        .authorized()
        .get("/api/channels/{channelId}/pins", pins::list)
        .authorized()
        .put("/api/messages/{messageId}/pin", pins::pin)
        .authorized()
        .delete("/api/messages/{messageId}/pin", pins::unpin)
        .authorized()
        .get("/api/channels/{channelId}/threads", threads::list)
        .authorized()
        .get("/api/channels/{channelId}/threads/{rootId}", threads::read)
        .authorized()
        .get("/api/channels/{channelId}/messages/search", index::search)
        .authorized()
        .get("/api/channels/{channelId}/messages/index", index::page)
        .authorized()
}

/// Feeds the same inputs to this code and to the first server's, for the
/// comparison in `tests/message-views-shared-database.sh`: each line of the
/// file named by `MESSAGE_VIEWS_CASES` is a kind, a space and a Base64 text,
/// and each answer is one line of `MESSAGE_VIEWS_ANSWERS`.
#[cfg(test)]
mod differential {
    use std::fmt::Write as _;

    use stellarr_unicode::{is_letter_or_number, to_lower};

    use super::ordinal_case::fold;

    use crate::http::parse_int;
    use super::cursor::IndexCursor;
    use super::search::build_query;
    use super::text;
    use crate::encoding::from_base64;
    use crate::permissions::membership::parse_guid;

    fn utf16_base64(text: &str) -> String {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        crate::encoding::base64(&bytes)
    }

    fn answer(kind: &str, argument: &str) -> String {
        let text = from_base64(argument)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        match kind {
            "Q" => build_query(&text).map_or("-".to_string(), |built| utf16_base64(&built)),
            "R" => utf16_base64(&crate::messages::dto::one_line(&text, 160)),
            "T" => utf16_base64(text::trim(&text)),
            "L" => utf16_base64(&to_lower(&text)),
            "I" => parse_int(&text).map_or("bad".to_string(), |number| number.to_string()),
            "G" => parse_guid(&text).map_or("bad".to_string(), |id| id.to_string()),
            "D" => match IndexCursor::decode(argument) {
                Some(cursor) => format!(
                    "ok {} {} {} {} {}",
                    cursor.channel,
                    cursor.snapshot.ticks() + 621_355_968_000_000_000,
                    cursor.offset,
                    match cursor.kind {
                        super::cursor::Kind::All => 0,
                        super::cursor::Kind::Media => 1,
                        super::cursor::Kind::Links => 2,
                    },
                    cursor.query_hash
                ),
                None => "bad".to_string(),
            },
            "H" => super::cursor::query_hash(Some(&text)).to_string(),
            _ => "?".to_string(),
        }
    }

    #[test]
    #[ignore = "needs the first server's answers to compare with"]
    fn answers_for_the_cases_in_the_file() {
        let (Ok(cases), Ok(out)) = (
            std::env::var("MESSAGE_VIEWS_CASES"),
            std::env::var("MESSAGE_VIEWS_ANSWERS"),
        ) else {
            return;
        };
        let mut written = String::new();
        for line in std::fs::read_to_string(cases).unwrap().lines() {
            let (kind, argument) = line.split_once(' ').unwrap_or((line, ""));
            if kind == "CF" {
                // The characters the first server's case-blind comparison
                // takes for the same one, grouped.
                let mut groups: std::collections::BTreeMap<char, Vec<u32>> = Default::default();
                for unit in (0u32..0x1_0000).filter(|u| !(0xD800..0xE000).contains(u)) {
                    let c = char::from_u32(unit).unwrap();
                    groups.entry(fold(c)).or_default().push(unit);
                }
                for members in groups.values().filter(|members| members.len() > 1) {
                    let list: Vec<String> = members.iter().map(u32::to_string).collect();
                    let _ = writeln!(written, "{}", list.join(","));
                }
                written.push_str("END\n");
            } else if kind == "LN" {
                let letters: Vec<String> = (0u32..0x1_0000)
                    .filter(|&u| char::from_u32(u).is_some_and(is_letter_or_number))
                    .map(|u| u.to_string())
                    .collect();
                let _ = writeln!(written, "{}", letters.join(","));
            } else {
                let _ = writeln!(written, "{}", answer(kind, argument));
            }
        }
        std::fs::write(out, written).unwrap();
    }
}

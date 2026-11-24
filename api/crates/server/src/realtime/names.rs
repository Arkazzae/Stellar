//! The groups a connection belongs to, and how a person writes an id the hub
//! is asked about.
//!
//! No group stands for who may see something: a group's members would go
//! stale when roles change. There is one per account, one per server and one
//! per account on the server a connection follows, and whoever may see an
//! event is worked out when it is sent.

use crate::values::Id;

/// An id as the group names write it: 32 hexadecimal digits, no hyphens.
fn compact(id: Id) -> String {
    id.to_string().replace('-', "")
}

/// Every connection that follows the server.
pub fn server_group(server: Id) -> String {
    format!("server:{}", compact(server))
}

/// The connections of one account that follow one server: what an event
/// meant for some of a server's members goes to. Unlike the account's own
/// group it leaves out the account's tabs on other servers, which would take a
/// message for a channel they do not know as something to sound.
pub fn follower_group(server: Id, user: Id) -> String {
    format!("server:{}:user:{}", compact(server), compact(user))
}

/// Every connection of one account.
pub fn user_group(user: Id) -> String {
    format!("user:{}", compact(user))
}

/// Reads an id as the first server reads one in a query, which is more
/// leniently than a UUID is written: around whitespace, with or without
/// hyphens, in braces or parentheses, as the list of hexadecimal numbers a
/// programmer writes (`{0x3d82da44,0x8aa5,...}`), and with each hyphenated
/// group read as a number that may begin with `+` or `0x`. A text that is none
/// of these is not an id.
pub fn parse_guid(text: &str) -> Option<Id> {
    let text = text.trim_matches(char::is_whitespace);
    let first = text.chars().next()?;
    let hyphens = text.contains('-');
    match first {
        '(' if text.len() == 38 && text.ends_with(')') => hyphenated(&text[1..37]),
        '(' => None,
        '{' if hyphens => (text.len() == 38 && text.ends_with('}'))
            .then(|| hyphenated(&text[1..37]))
            .flatten(),
        '{' => listed(text),
        _ if hyphens => hyphenated(text),
        _ => plain(text),
    }
}

/// 32 hexadecimal digits.
fn plain(text: &str) -> Option<Id> {
    if text.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, pair) in text.as_bytes().chunks(2).enumerate() {
        bytes[index] = hex(pair[0])? << 4 | hex(pair[1])?;
    }
    Some(Id::from_bytes(bytes))
}

/// The groups of 8, 4, 4, 4 and 12 digits with a hyphen between.
fn hyphenated(text: &str) -> Option<Id> {
    let bytes = text.as_bytes();
    if text.len() != 36 || [8, 13, 18, 23].iter().any(|at| bytes[*at] != b'-') {
        return None;
    }
    let group = |from: usize, to: usize, bits: u32| number(text.get(from..to)?, bits);
    Some(assemble(
        group(0, 8, 32)? as u32,
        group(9, 13, 16)? as u16,
        group(14, 18, 16)? as u16,
        group(19, 23, 16)? as u16,
        group(24, 36, 48)?,
    ))
}

/// `{0x..,0x..,0x..,{0x..,0x..,0x..,0x..,0x..,0x..,0x..,0x..}}`, with
/// whitespace anywhere, each number as long as it needs to be.
fn listed(text: &str) -> Option<Id> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let inner = text.strip_prefix('{')?.strip_suffix('}')?;
    let (head, tail) = inner.split_once(",{")?;
    let tail = tail.strip_suffix('}')?;
    let mut head = head.split(',');
    let mut numbers = Vec::with_capacity(11);
    // The second and third are read as numbers of 32 bits and cut to 16: a
    // longer one is not refused.
    for _ in 0..3 {
        numbers.push(prefixed(head.next()?, 32)?);
    }
    if head.next().is_some() {
        return None;
    }
    let bytes: Vec<&str> = tail.split(',').collect();
    if bytes.len() != 8 {
        return None;
    }
    let mut last = [0u8; 8];
    for (slot, text) in last.iter_mut().zip(bytes) {
        *slot = prefixed(text, 8)? as u8;
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&(numbers[0] as u32).to_be_bytes());
    out[4..6].copy_from_slice(&(numbers[1] as u16).to_be_bytes());
    out[6..8].copy_from_slice(&(numbers[2] as u16).to_be_bytes());
    out[8..].copy_from_slice(&last);
    Some(Id::from_bytes(out))
}

fn assemble(a: u32, b: u16, c: u16, d: u16, e: u64) -> Id {
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a.to_be_bytes());
    out[4..6].copy_from_slice(&b.to_be_bytes());
    out[6..8].copy_from_slice(&c.to_be_bytes());
    out[8..10].copy_from_slice(&d.to_be_bytes());
    out[10..].copy_from_slice(&e.to_be_bytes()[2..]);
    Id::from_bytes(out)
}

/// A number written after `0x`, as it stands in a list.
fn prefixed(text: &str, bits: u32) -> Option<u64> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))?;
    if digits.is_empty() {
        return None;
    }
    number(digits, bits)
}

/// A hexadecimal number the way the first server reads one: an optional `+`,
/// an optional `0x`, and then digits, as many as fit in `bits`.
fn number(text: &str, bits: u32) -> Option<u64> {
    let mut digits = text.strip_prefix('+').unwrap_or(text);
    if digits.len() > 1 && (digits.starts_with("0x") || digits.starts_with("0X")) {
        digits = &digits[2..];
    }
    let significant = digits.trim_start_matches('0');
    if significant.len() > (bits / 4) as usize {
        // Zeros may stand in front, but a number may not outgrow its place.
        digits.bytes().try_for_each(|byte| hex(byte).map(|_| ()))?;
        return None;
    }
    let mut value = 0u64;
    for byte in digits.bytes() {
        value = value << 4 | u64::from(hex(byte)?);
    }
    Some(value)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The text after `name=` in a query string, read as a browser sent it:
/// percent-decoded, with `+` for a space, and found without regard to the
/// case of the name. The first one wins.
pub fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(key).eq_ignore_ascii_case(name)).then(|| decode(value))
    })
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' if at + 2 < bytes.len() => match (hex(bytes[at + 1]), hex(bytes[at + 2])) {
                (Some(high), Some(low)) => {
                    out.push(high << 4 | low);
                    at += 2;
                }
                _ => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3d82da44-8aa5-4bdf-9ee1-2902df99880c";

    #[test]
    fn the_groups_are_named_with_ids_without_hyphens() {
        let server = Id::parse(ID).unwrap();
        let user = Id::parse("30000000-0000-0000-0000-000000000001").unwrap();

        assert_eq!(
            server_group(server),
            "server:3d82da448aa54bdf9ee12902df99880c"
        );
        assert_eq!(user_group(user), "user:30000000000000000000000000000001");
        assert_eq!(
            follower_group(server, user),
            "server:3d82da448aa54bdf9ee12902df99880c:user:30000000000000000000000000000001"
        );
    }

    #[test]
    fn an_id_is_read_in_the_forms_the_first_server_reads() {
        let want = Id::parse(ID).unwrap();
        for text in [
            ID,
            "3D82DA44-8AA5-4BDF-9EE1-2902DF99880C",
            "3d82da448aa54bdf9ee12902df99880c",
            "{3d82da44-8aa5-4bdf-9ee1-2902df99880c}",
            "(3d82da44-8aa5-4bdf-9ee1-2902df99880c)",
            "  3d82da44-8aa5-4bdf-9ee1-2902df99880c ",
            "{0x3d82da44,0x8aa5,0x4bdf,{0x9e,0xe1,0x29,0x02,0xdf,0x99,0x88,0x0c}}",
            "{ 0x3d82da44 , 0X8aa5 , 0x4bdf , { 0x9e , 0xe1 , 0x29 , 0x02 , 0xdf , 0x99 , 0x88 , 0x0c } }",
            "+d82da44-8aa5-4bdf-9ee1-2902df99880c"
                .replace('+', "3")
                .as_str(),
        ] {
            assert_eq!(parse_guid(text), Some(want), "{text:?}");
        }
    }

    #[test]
    fn a_group_that_begins_with_a_sign_or_a_prefix_is_read_as_the_number_it_is() {
        // Odd, and what the first server does: the group is a number.
        assert_eq!(
            parse_guid("+d82da44-8aa5-4bdf-9ee1-2902df99880c"),
            Id::parse("0d82da44-8aa5-4bdf-9ee1-2902df99880c")
        );
        assert_eq!(
            parse_guid("3d82da44-0xaa-4bdf-9ee1-2902df99880c"),
            Id::parse("3d82da44-00aa-4bdf-9ee1-2902df99880c")
        );
        assert_eq!(
            parse_guid("{0x1,0x2,0x3,{0x4,0x5,0x6,0x7,0x8,0x9,0xa,0xb}}"),
            Id::parse("00000001-0002-0003-0405-060708090a0b")
        );
        // The second and third numbers of a list are cut to what fits.
        assert_eq!(
            parse_guid("{0x1,0xc29b9,0x3,{0x4,0x5,0x6,0x7,0x8,0x9,0xa,0xb}}"),
            Id::parse("00000001-29b9-0003-0405-060708090a0b")
        );
    }

    #[test]
    fn what_is_not_an_id_is_not_read_as_one() {
        for text in [
            "",
            "not-a-guid",
            "3d82da44-8aa5-4bdf-9ee1-2902df99880",
            "3d82da44-8aa5-4bdf-9ee1-2902df99880cc",
            "3d82da448aa5-4bdf-9ee1-2902df99880c",
            "{3d82da448aa54bdf9ee12902df99880c}",
            "{3d82da44-8aa5-4bdf-9ee1-2902df99880c)",
            "3d82da44-8aa5-4bdf-9ee1-2902df99880g",
            "3d82da44-8aa5-4bdf-9ee1-2902df99880c\u{0}",
            "{0x3d82da44,0x8aa5,0x4bdf,{0x9e,0xe1,0x29,0x02,0xdf,0x99,0x88}}",
            "{0x1,0x2,0x3,{0x4,0x5,0x6,0x7,0x8,0x9,0xa,0x1ff}}",
            "{0x1,0x2,0x3,{0x4,0x5,0x6,0x7,0x8,0x9,0xa,0xb}",
        ] {
            assert_eq!(parse_guid(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_query_value_is_found_by_any_case_of_its_name_and_decoded() {
        assert_eq!(
            query_value("serverId=abc&x=1", "serverId").as_deref(),
            Some("abc")
        );
        assert_eq!(
            query_value("x=1&SERVERID=abc", "serverId").as_deref(),
            Some("abc")
        );
        assert_eq!(
            query_value("serverId=a%2Db+c", "serverId").as_deref(),
            Some("a-b c")
        );
        assert_eq!(
            query_value("serverId=first&serverId=second", "serverId").as_deref(),
            Some("first")
        );
        assert_eq!(query_value("serverId", "serverId").as_deref(), Some(""));
        assert_eq!(query_value("other=1", "serverId"), None);
        assert_eq!(
            query_value("serverId=%zz%4", "serverId").as_deref(),
            Some("%zz%4")
        );
    }

    /// Compares the reader with the first server's on texts it was run on: a
    /// file of JSON strings, one to a line, and a file of what the first
    /// server made of each (the id, or `-`).
    ///
    /// ```sh
    /// GUID_CORPUS=corpus.jsonl GUID_EXPECTED=expected.txt cargo test -p stellarr-api \
    ///     --lib an_id_is_read_as_the_first_server_reads_it -- --ignored
    /// ```
    #[test]
    #[ignore = "needs a corpus the first server has read"]
    fn an_id_is_read_as_the_first_server_reads_it() {
        let corpus = std::fs::read_to_string(std::env::var("GUID_CORPUS").unwrap()).unwrap();
        let expected = std::fs::read_to_string(std::env::var("GUID_EXPECTED").unwrap()).unwrap();
        let mut different = Vec::new();
        for (line, want) in corpus.lines().zip(expected.lines()) {
            let text: String = serde_json::from_str(line).unwrap();
            let got = parse_guid(&text).map_or("-".to_string(), |id| id.to_string());
            if got != want {
                different.push(format!("{text:?}: first server {want}, here {got}"));
            }
        }
        assert!(
            different.is_empty(),
            "{} differ, such as {:#?}",
            different.len(),
            &different[..different.len().min(15)]
        );
    }
}

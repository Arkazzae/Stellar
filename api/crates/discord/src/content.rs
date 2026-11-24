//! Turns the text of a Discord message into the text of a Stellarr message.
//!
//! Discord writes references as tokens (`<@id>`, `<@!id>`, `<@&id>`, `<#id>`,
//! `<:name:id>`, `<a:name:id>`, `<t:unix:style>`); Stellarr has `@username`
//! and `:shortcode:` and nothing else. A user becomes what the resolver says;
//! a role, a channel and an id nobody knows become plain text, never the raw
//! token; a time becomes an absolute UTC time, since the reader's own time
//! zone is gone once the text is copied.
//!
//! Code is read the way Stellarr reads it, fences first and then spans that
//! close at a run of backticks of the same length, and nothing inside is
//! touched. Discord's own escapes are kept, and they mean the same here.
//!
//! Plain text that Stellarr would read as markup Discord did not have is
//! escaped: an `@word` (including `@everyone` and `@here`) that was only text
//! in Discord, and a `:word:` that would show one of the server's emoji. The
//! finished text is then read by [`extract_normalized_usernames`], the reader
//! that decides whom to notify; if it finds a name the resolver did not give,
//! the message is written again with every mention as text, and as a last
//! resort with a zero-width space after every at sign.

use std::collections::BTreeSet;

use stellarr_mentions::{
    BacktickRuns, extract_units, fenced_ranges, is_address_character, run_length, starts_name,
};
use stellarr_unicode::utf16::{
    is_ascii_punctuation, is_low_surrogate, is_surrogate, scalar_at, text, to_upper_units,
    unit_is_letter_or_digit, units,
};
use stellarr_unicode::{is_letter_or_digit, is_letter_or_number};

const UNKNOWN_USER: &str = "@unknown-user";
const UNKNOWN_ROLE: &str = "@deleted-role";
const UNKNOWN_CHANNEL: &str = "#deleted-channel";

/// The range of Unix seconds a date can hold, from year 1 to year 9999.
const MIN_UNIX_SECONDS: i64 = -62_135_596_800;
const MAX_UNIX_SECONDS: i64 = 253_402_300_799;

const ZERO_WIDTH_SPACE: u16 = 0x200B;
const BACKSLASH: u16 = b'\\' as u16;
const BACKTICK: u16 = b'`' as u16;
const AT: u16 = b'@' as u16;
const COLON: u16 = b':' as u16;

/// What a Discord user mention becomes in a Stellarr message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionText {
    /// The username to mention, or the label to show.
    pub text: String,
    /// Whether `text` is a username that the message mentions.
    pub mentions: bool,
}

impl MentionText {
    /// A mention of the Stellarr account with this username: it is written
    /// `@username`, and that account is notified when the message is sent.
    pub fn account(username: impl Into<String>) -> MentionText {
        MentionText {
            text: username.into(),
            mentions: true,
        }
    }

    /// Text shown as given, such as `@Ada (left)`; it mentions nobody.
    pub fn label(text: impl Into<String>) -> MentionText {
        MentionText {
            text: text.into(),
            mentions: false,
        }
    }
}

/// What the Discord ids in a message stand for, as the import decided.
///
/// The rewriting writes exactly what this returns, so this is where a mention
/// is made or not. [`ReferenceResolver::user`] may answer
/// [`MentionText::account`] only with the Stellarr account the import made for
/// that Discord user, or one its owner linked to that Discord user on
/// purpose. Anybody else, such as a user the import left out or an account
/// that only happens to share the name, gets a [`MentionText::label`] or
/// `None`. Any other answer would notify a real person about a message they
/// were never part of. Roles, channels and unknown ids are always written as
/// text.
pub trait ReferenceResolver {
    /// What a mention of this Discord user becomes, or `None` when the import
    /// knows nothing of them.
    fn user(&self, user_id: &str) -> Option<MentionText>;
    /// How to show a mention of this role, such as `@Moderators`.
    fn role(&self, role_id: &str) -> Option<String>;
    /// How to show a link to this channel or thread, such as `#general`.
    fn channel(&self, channel_id: &str) -> Option<String>;
    /// The shortcode, without colons, of the Stellarr emoji this custom emoji
    /// became.
    fn emoji(&self, emoji_id: &str) -> Option<String>;
}

/// Rewrites Discord message text as Stellarr message text.
///
/// The result mentions exactly the accounts the resolver chose to mention and
/// nobody else; no input makes this panic or read any part of it more than a
/// bounded number of times.
pub fn rewrite(content: &str, resolver: &dyn ReferenceResolver) -> String {
    if content.is_empty() {
        return String::new();
    }

    let pieces = read(&units(content), resolver);
    let intended: BTreeSet<Vec<u16>> = pieces
        .iter()
        .filter(|piece| piece.kind == PieceKind::Mention)
        .map(|piece| to_upper_units(&piece.text[1..]))
        .collect();

    let mut written = Vec::new();
    for mode in [Mode::Mentions, Mode::NoMentions, Mode::EveryAtEscaped] {
        written = write(&pieces, mode);
        let found = extract_units(&written);
        if found.is_subset(&intended) && (mode == Mode::Mentions || found.is_empty()) {
            return text(&written);
        }
    }

    // The escapes rest on reading the text as Stellarr does. Should the two
    // readings ever differ, no at sign is left before a name character at
    // all, code included: the zero-width space is not one, and shows as
    // nothing.
    let spaced = insert_after_every_at(&written);
    if extract_units(&spaced).is_empty() {
        return text(&spaced);
    }
    // Not reachable: nothing that follows the at sign here can be part of a
    // name. If it ever were, no at sign at all is the one answer that cannot
    // mention anybody.
    text(
        &written
            .iter()
            .copied()
            .filter(|&unit| unit != AT)
            .collect::<Vec<_>>(),
    )
}

/// Text the import makes up, such as a thread's name, written so that
/// Stellarr shows it as it is: no formatting, no link, no shortcode and
/// nobody mentioned.
pub fn escape(input: &str) -> String {
    let piece = Piece {
        kind: PieceKind::Label,
        text: units(input),
    };
    let escaped = write(&[piece], Mode::EveryAtEscaped);
    if extract_units(&escaped).is_empty() {
        text(&escaped)
    } else {
        text(&insert_after_every_at(&escaped))
    }
}

fn insert_after_every_at(source: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(source.len() + 8);
    for &unit in source {
        out.push(unit);
        if unit == AT {
            out.push(ZERO_WIDTH_SPACE);
        }
    }
    out
}

/* ---------------------------------------------------------------- reading */

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PieceKind {
    /// Written as it is: code, an escape, an emoji's shortcode.
    Raw,
    /// The author's own text: Discord's formatting stays, stray mentions and
    /// shortcodes are escaped.
    Prose,
    /// Text the import made up: every marker in it is escaped.
    Label,
    /// `@username`, which is meant to mention that account.
    Mention,
}

struct Piece {
    kind: PieceKind,
    text: Vec<u16>,
}

fn piece(kind: PieceKind, text: &[u16]) -> Piece {
    Piece {
        kind,
        text: text.to_vec(),
    }
}

fn label(text: &str) -> Piece {
    Piece {
        kind: PieceKind::Label,
        text: units(text),
    }
}

fn read(content: &[u16], resolver: &dyn ReferenceResolver) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut from = 0;
    for (start, end) in fenced_ranges(content) {
        read_inline(content, from, start, resolver, &mut pieces);
        pieces.push(piece(PieceKind::Raw, &content[start..end]));
        from = end;
    }
    read_inline(content, from, content.len(), resolver, &mut pieces);
    pieces
}

/// Reads one stretch between fences: escapes, code spans, Discord's tokens
/// and prose.
fn read_inline(
    content: &[u16],
    start: usize,
    end: usize,
    resolver: &dyn ReferenceResolver,
    pieces: &mut Vec<Piece>,
) {
    let mut runs = BacktickRuns::new(&content[start..end]);
    let mut prose = start;
    let mut index = start;
    while index < end {
        let current = content[index];
        if current == BACKSLASH && index + 1 < end && is_ascii_punctuation(content[index + 1]) {
            flush(content, prose, index, pieces);
            pieces.push(piece(PieceKind::Raw, &content[index..index + 2]));
            index += 2;
            prose = index;
            continue;
        }

        if current == BACKTICK {
            flush(content, prose, index, pieces);
            let run = run_length(&content[..end], index, BACKTICK);
            let span_end = match runs.next(run, index + run - start) {
                Some(close) => start + close + run,
                None => index + run,
            };
            pieces.push(piece(PieceKind::Raw, &content[index..span_end]));
            index = span_end;
            prose = index;
            continue;
        }

        if current == b'<' as u16
            && let Some((token, length)) = parse_token(content, index)
            && index + length <= end
        {
            flush(content, prose, index, pieces);
            pieces.push(resolve(&token, &content[index..index + length], resolver));
            index += length;
            prose = index;
            continue;
        }

        index += 1;
    }
    flush(content, prose, end, pieces);
}

fn flush(content: &[u16], from: usize, to: usize, pieces: &mut Vec<Piece>) {
    if to > from {
        pieces.push(piece(PieceKind::Prose, &content[from..to]));
    }
}

/// One of Discord's reference tokens, the ids being ASCII digits.
enum Token {
    User(String),
    Role(String),
    Channel(String),
    Emoji { name: Vec<u16>, id: String },
    Time { value: String, style: Option<u16> },
}

fn digits(content: &[u16], from: usize) -> usize {
    content[from.min(content.len())..]
        .iter()
        .take_while(|&&unit| (0x30..=0x39).contains(&unit))
        .count()
}

fn ascii(content: &[u16]) -> String {
    content.iter().map(|&unit| char::from(unit as u8)).collect()
}

/// `\s` of a regular expression: what ends the name inside an emoji token.
fn is_regex_space(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0D
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// Reads a token that starts with `<` at `index`, and how long it is.
fn parse_token(content: &[u16], index: usize) -> Option<(Token, usize)> {
    let at = |position: usize| content.get(position).copied();
    let closes = |position: usize| at(position) == Some(b'>' as u16);
    let i = index + 1;
    match at(i)? {
        0x40 => {
            // <@id>, <@!id>, <@&id>
            let (skip, role) = match at(i + 1) {
                Some(0x21) => (2, false),
                Some(0x26) => (2, true),
                _ => (1, false),
            };
            let start = i + skip;
            let count = digits(content, start);
            if !(1..=20).contains(&count) || !closes(start + count) {
                return None;
            }
            let id = ascii(&content[start..start + count]);
            let token = if role {
                Token::Role(id)
            } else {
                Token::User(id)
            };
            Some((token, start + count + 1 - index))
        }
        0x23 => {
            let start = i + 1;
            let count = digits(content, start);
            if !(1..=20).contains(&count) || !closes(start + count) {
                return None;
            }
            Some((
                Token::Channel(ascii(&content[start..start + count])),
                start + count + 1 - index,
            ))
        }
        first @ (0x61 | 0x3A) => {
            // <:name:id> and <a:name:id>
            let colon = if first == 0x61 { i + 1 } else { i };
            if at(colon) != Some(COLON) {
                return None;
            }
            let name_start = colon + 1;
            let name_length = content[name_start.min(content.len())..]
                .iter()
                .take_while(|&&unit| !is_regex_space(unit) && !matches!(unit, 0x3A | 0x3C | 0x3E))
                .count();
            if !(1..=64).contains(&name_length) || at(name_start + name_length) != Some(COLON) {
                return None;
            }
            let id_start = name_start + name_length + 1;
            let count = digits(content, id_start);
            if !(1..=20).contains(&count) || !closes(id_start + count) {
                return None;
            }
            Some((
                Token::Emoji {
                    name: content[name_start..name_start + name_length].to_vec(),
                    id: ascii(&content[id_start..id_start + count]),
                },
                id_start + count + 1 - index,
            ))
        }
        0x74 => {
            // <t:seconds> and <t:seconds:style>
            if at(i + 1) != Some(COLON) {
                return None;
            }
            let start = i + 2;
            let sign = usize::from(at(start) == Some(b'-' as u16));
            let count = digits(content, start + sign);
            if !(1..=20).contains(&count) {
                return None;
            }
            let value_end = start + sign + count;
            let value = ascii(&content[start..value_end]);
            if closes(value_end) {
                return Some((Token::Time { value, style: None }, value_end + 1 - index));
            }
            let style = at(value_end + 1).filter(|unit| {
                at(value_end) == Some(COLON) && *unit < 128 && (*unit as u8).is_ascii_alphabetic()
            })?;
            if !closes(value_end + 2) {
                return None;
            }
            Some((
                Token::Time {
                    value,
                    style: Some(style),
                },
                value_end + 3 - index,
            ))
        }
        _ => None,
    }
}

fn resolve(token: &Token, raw: &[u16], resolver: &dyn ReferenceResolver) -> Piece {
    match token {
        Token::User(id) => match resolver.user(id) {
            None => label(UNKNOWN_USER),
            Some(mention) if mention.mentions && is_username(&units(&mention.text)) => Piece {
                kind: PieceKind::Mention,
                text: units(&format!("@{}", mention.text)),
            },
            Some(mention) if mention.mentions => label(&format!("@{}", mention.text)),
            Some(mention) => label(&mention.text),
        },
        Token::Role(id) => label(&resolver.role(id).unwrap_or_else(|| UNKNOWN_ROLE.to_owned())),
        Token::Channel(id) => label(
            &resolver
                .channel(id)
                .unwrap_or_else(|| UNKNOWN_CHANNEL.to_owned()),
        ),
        Token::Emoji { name, id } => match resolver.emoji(id) {
            Some(shortcode) if is_shortcode(&shortcode) => Piece {
                kind: PieceKind::Raw,
                text: units(&format!(":{shortcode}:")),
            },
            _ => {
                let mut shown = vec![COLON];
                shown.extend_from_slice(name);
                shown.push(COLON);
                Piece {
                    kind: PieceKind::Label,
                    text: shown,
                }
            }
        },
        Token::Time { value, style } => match value.parse::<i64>() {
            Ok(seconds) if (MIN_UNIX_SECONDS..=MAX_UNIX_SECONDS).contains(&seconds) => {
                label(&format_time(seconds, *style))
            }
            _ => piece(PieceKind::Prose, raw),
        },
    }
}

/// A name `@name` mentions: what Stellarr accepts as a username and reads
/// after an at sign. Letters and numbers outside the first plane do not
/// count, because they are two characters of the text, neither of them one.
fn is_username(name: &[u16]) -> bool {
    (2..=32).contains(&name.len())
        && name.iter().all(|&unit| {
            (!is_surrogate(unit)
                && char::from_u32(u32::from(unit)).is_some_and(is_letter_or_number))
                || matches!(unit, 0x5F | 0x2E | 0x2D)
        })
}

/// The shortcodes a Stellarr server gives its emoji.
fn is_shortcode(shortcode: &str) -> bool {
    (2..=32).contains(&shortcode.len())
        && shortcode
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A time in UTC, showing what the style showed: a date for the date styles,
/// a time for the time styles, both otherwise. The relative style ("4 years
/// ago") is written as the moment itself, since "ago" would only be true on
/// the day of the import.
fn format_time(seconds: i64, style: Option<u16>) -> String {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (of_day / 3600, of_day % 3600 / 60, of_day % 60);
    let date = format!("{year:04}-{month:02}-{day:02}");
    match style.and_then(|unit| char::from_u32(u32::from(unit))) {
        Some('d' | 'D') => date,
        Some('t') => format!("{hour:02}:{minute:02} UTC"),
        Some('T') => format!("{hour:02}:{minute:02}:{second:02} UTC"),
        Some('S') => format!("{date} {hour:02}:{minute:02}:{second:02} UTC"),
        _ => format!("{date} {hour:02}:{minute:02} UTC"),
    }
}

/// The calendar date of a day count from 1970-01-01, in the proleptic
/// Gregorian calendar.
pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    (year + i64::from(month <= 2), month, day)
}

/* ---------------------------------------------------------------- writing */

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Mentions as the resolver asked.
    Mentions,
    /// Every mention as text, for when the first attempt read as somebody else.
    NoMentions,
    /// No exceptions at all: every at sign before a name character is escaped.
    EveryAtEscaped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharKind {
    Text,
    Raw,
    Mention,
}

/// Joins the pieces and escapes what Stellarr would read differently. The
/// escapes depend on the characters around them, which may come from another
/// piece, so the pieces are joined first and read as one text.
fn write(pieces: &[Piece], mode: Mode) -> Vec<u16> {
    let mut source: Vec<u16> = Vec::new();
    let mut kinds: Vec<CharKind> = Vec::new();
    for piece in pieces {
        let shown = if piece.kind == PieceKind::Label {
            escape_label(&piece.text)
        } else {
            piece.text.clone()
        };
        let kind = match piece.kind {
            PieceKind::Raw => CharKind::Raw,
            PieceKind::Mention => CharKind::Mention,
            _ => CharKind::Text,
        };
        kinds.resize(kinds.len() + shown.len(), kind);
        source.extend_from_slice(&shown);
    }

    let mut output = Output::new(source.len());
    let mut index = 0;
    let mut escape_next = false;
    while index < source.len() {
        let kind = kinds[index];
        let mut end = index + 1;
        while end < source.len()
            && kinds[end] == kind
            && (kind != CharKind::Mention || source[end] != AT)
        {
            end += 1;
        }

        match kind {
            CharKind::Raw => output.append_all(&source[index..end]),
            CharKind::Mention => {
                let next = scalar_at(&source, end).map(|(c, _)| c);
                let as_text = mode != Mode::Mentions
                    || output.ends_with_letter_or_digit
                    || next.is_some_and(is_letter_or_digit);
                if as_text {
                    output.append(BACKSLASH);
                } else {
                    escape_next =
                        matches!(next, Some('_' | '.' | '-')) && kinds[end] == CharKind::Text;
                }
                output.append_all(&source[index..end]);
            }
            CharKind::Text => write_text(&source, index, end, mode, &mut output, &mut escape_next),
        }
        index = end;
    }
    output.units
}

fn write_text(
    source: &[u16],
    start: usize,
    end: usize,
    mode: Mode,
    output: &mut Output,
    escape_next: &mut bool,
) {
    let mut index = start;
    while index < end {
        let current = source[index];
        if *escape_next {
            *escape_next = false;
            output.append(BACKSLASH);
            output.append(current);
            index += 1;
            continue;
        }

        if current == BACKSLASH && index + 1 < end && is_ascii_punctuation(source[index + 1]) {
            output.append_all(&source[index..index + 2]);
            index += 2;
            continue;
        }

        if current == b'h' as u16 {
            let address_end = bare_address_end(source, index, end);
            if address_end > index {
                // Stellarr reads an address as a link and nothing in it as a mention.
                output.append_all(&source[index..address_end]);
                index = address_end;
                continue;
            }
        }

        // An at sign that would start a name, or a colon that would open a
        // shortcode, is escaped.
        let at_name = current == AT
            && starts_name(source, index + 1)
            && (mode == Mode::EveryAtEscaped || !output.ends_with_letter_or_digit);
        let shortcode = !at_name && current == COLON && shortcode_ahead(source, index + 1);
        if at_name || shortcode {
            output.append(BACKSLASH);
        }

        output.append(current);
        index += 1;
    }
}

/// What follows the colon that opens a shortcode, as the web client reads
/// one: two to sixty-four letters, digits, underscores or dashes, and a colon.
fn shortcode_ahead(source: &[u16], from: usize) -> bool {
    let count = source[from.min(source.len())..]
        .iter()
        .take_while(|&&unit| {
            unit < 128 && ((unit as u8).is_ascii_alphanumeric() || matches!(unit, 0x5F | 0x2D))
        })
        .count();
    (2..=64).contains(&count) && source.get(from + count) == Some(&COLON)
}

/// The text written so far, and whether it ends in a letter or digit.
struct Output {
    units: Vec<u16>,
    ends_with_letter_or_digit: bool,
}

impl Output {
    fn new(capacity: usize) -> Output {
        Output {
            units: Vec::with_capacity(capacity + 16),
            ends_with_letter_or_digit: false,
        }
    }

    fn append(&mut self, unit: u16) {
        self.units.push(unit);
        self.ends_with_letter_or_digit = unit_is_letter_or_digit(unit);
        if is_low_surrogate(unit)
            && self.units.len() >= 2
            && let Some((c, 2)) = scalar_at(&self.units, self.units.len() - 2)
        {
            self.ends_with_letter_or_digit = is_letter_or_digit(c);
        }
    }

    fn append_all(&mut self, units: &[u16]) {
        for &unit in units {
            self.append(unit);
        }
    }
}

/// Escapes text the import made up, as the web client's `escapeMarkup` does:
/// every formatting marker, and the colon of an http(s) scheme so a name that
/// only looks like an address does not become a link. At signs and shortcodes
/// are left to [`write_text`], which sees the text around the label too.
fn escape_label(label: &[u16]) -> Vec<u16> {
    let mut escaped = Vec::with_capacity(label.len() + 8);
    for &unit in label {
        if matches!(unit, 0x5C | 0x60 | 0x2A | 0x5F | 0x7E | 0x7C | 0x5B | 0x5D) {
            escaped.push(BACKSLASH);
        }
        escaped.push(unit);
    }

    // `(https?)://` in any case becomes `$1\://`.
    let mut out = Vec::with_capacity(escaped.len() + 8);
    let mut index = 0;
    while index < escaped.len() {
        if let Some(colon) = scheme_colon(&escaped, index) {
            out.extend_from_slice(&escaped[index..colon]);
            out.push(BACKSLASH);
            out.extend_from_slice(&escaped[colon..colon + 3]);
            index = colon + 3;
        } else {
            out.push(escaped[index]);
            index += 1;
        }
    }
    out
}

/// Where the `://` of an `http://` or `https://` that starts at `index`
/// begins, whatever the case of the letters.
fn scheme_colon(text: &[u16], index: usize) -> Option<usize> {
    let letters = |from: usize, word: &str| {
        word.bytes().enumerate().all(|(offset, byte)| {
            text.get(from + offset)
                .is_some_and(|&unit| unit < 128 && (unit as u8).eq_ignore_ascii_case(&byte))
        })
    };
    if !letters(index, "http") {
        return None;
    }
    let after = if letters(index + 4, "s") {
        index + 5
    } else {
        index + 4
    };
    let slashes = [COLON, b'/' as u16, b'/' as u16];
    (text.len() >= after + 3 && text[after..after + 3] == slashes).then_some(after)
}

/// Where a bare address that starts at `index` ends, or `index` itself when
/// none does. As in Stellarr, only a lower-case scheme starts one, and it
/// runs to the next character that is never raw in an address. It also ends
/// at a `]`, where the web client ends it: what follows a bracket is read as
/// text there, so it is escaped as text here.
fn bare_address_end(text: &[u16], index: usize, end: usize) -> usize {
    let span = &text[index..end];
    let scheme = |word: &str| {
        span.len() >= word.len()
            && word
                .bytes()
                .zip(span)
                .all(|(byte, &unit)| u16::from(byte) == unit)
    };
    let scheme_length = if scheme("https://") {
        8
    } else if scheme("http://") {
        7
    } else {
        return index;
    };

    let mut cursor = index + scheme_length;
    while cursor < end && is_address_character(text[cursor]) && text[cursor] != b']' as u16 {
        cursor += 1;
    }
    if cursor > index + scheme_length {
        cursor
    } else {
        index
    }
}

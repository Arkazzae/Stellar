//! Reading a page's bytes as text.
//!
//! A page names its character set in the `Content-Type` header, in a
//! byte-order mark, in a `<meta>` tag near its start, or nowhere. They are
//! consulted in that order of strength: a byte-order mark first, because it
//! is a fact about the bytes, then the header, then the `<meta>` tag, and
//! without any of them the page is read as UTF-8.
//!
//! Decoded here: UTF-8, UTF-16 in either byte order, and the single-byte
//! sets that pages in the languages of Europe, the Middle East and the
//! Cyrillic world still use (the Windows code pages 1250 to 1258, ISO 8859-2,
//! -4, -5, -6, -7, -8, -13 and -15, KOI8-R and KOI8-U). `iso-8859-1`,
//! `latin1` and `us-ascii` mean the Windows 1252 page, which is what every
//! browser does and what pages that say them are written in. `iso-8859-9`
//! likewise means Windows 1254.
//!
//! Not decoded: the East Asian multi-byte sets (Shift_JIS, EUC-JP, GBK,
//! GB18030, Big5, EUC-KR), UTF-32 and UTF-7. A page that declares one of
//! these, or a name nobody knows, is read as UTF-8, bytes that are not valid
//! UTF-8 becoming U+FFFD, and the result says so ([`Decoded::unsupported`]).
//! The tags and addresses on such a page are ASCII and still read; its
//! title and description come out damaged. That is the better outcome than
//! no card, and the worse one than a table, which is not worth the size for
//! the few pages of that kind that get posted here.

mod tables;

use crate::opengraph::attributes;

/// How far into a page a `<meta>` declaration is looked for. A declaration
/// further in is too late to have been in effect for the first bytes of the
/// page, and the bound keeps the search small.
pub const META_PRESCAN_BYTES: usize = 4096;

/// A character set this crate can decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charset {
    Utf8,
    Utf16Le,
    Utf16Be,
    /// A single-byte set: its name and what each byte stands for.
    Single(&'static str, &'static [u16; 256]),
}

impl Charset {
    pub fn name(self) -> &'static str {
        match self {
            Charset::Utf8 => "utf-8",
            Charset::Utf16Le => "utf-16le",
            Charset::Utf16Be => "utf-16be",
            Charset::Single(name, _) => name,
        }
    }
}

/// Every single-byte set, by its name.
pub const SINGLE_BYTE_SETS: [(&str, &[u16; 256]); 19] = [
    ("windows-1250", &tables::WINDOWS_1250),
    ("windows-1251", &tables::WINDOWS_1251),
    ("windows-1252", &tables::WINDOWS_1252),
    ("windows-1253", &tables::WINDOWS_1253),
    ("windows-1254", &tables::WINDOWS_1254),
    ("windows-1255", &tables::WINDOWS_1255),
    ("windows-1256", &tables::WINDOWS_1256),
    ("windows-1257", &tables::WINDOWS_1257),
    ("windows-1258", &tables::WINDOWS_1258),
    ("iso-8859-2", &tables::ISO_8859_2),
    ("iso-8859-4", &tables::ISO_8859_4),
    ("iso-8859-5", &tables::ISO_8859_5),
    ("iso-8859-6", &tables::ISO_8859_6),
    ("iso-8859-7", &tables::ISO_8859_7),
    ("iso-8859-8", &tables::ISO_8859_8),
    ("iso-8859-13", &tables::ISO_8859_13),
    ("iso-8859-15", &tables::ISO_8859_15),
    ("koi8-r", &tables::KOI8_R),
    ("koi8-u", &tables::KOI8_U),
];

/// The other names a set goes by, as pages and servers write them.
const ALIASES: &[(&str, &str)] = &[
    ("iso-8859-1", "windows-1252"),
    ("iso8859-1", "windows-1252"),
    ("iso88591", "windows-1252"),
    ("iso_8859-1", "windows-1252"),
    ("latin1", "windows-1252"),
    ("latin-1", "windows-1252"),
    ("l1", "windows-1252"),
    ("us-ascii", "windows-1252"),
    ("ascii", "windows-1252"),
    ("ansi_x3.4-1968", "windows-1252"),
    ("cp1252", "windows-1252"),
    ("x-cp1252", "windows-1252"),
    ("cp1250", "windows-1250"),
    ("x-cp1250", "windows-1250"),
    ("cp1251", "windows-1251"),
    ("x-cp1251", "windows-1251"),
    ("cp1253", "windows-1253"),
    ("cp1254", "windows-1254"),
    ("iso-8859-9", "windows-1254"),
    ("iso8859-9", "windows-1254"),
    ("latin5", "windows-1254"),
    ("cp1255", "windows-1255"),
    ("cp1256", "windows-1256"),
    ("cp1257", "windows-1257"),
    ("cp1258", "windows-1258"),
    ("iso8859-2", "iso-8859-2"),
    ("iso88592", "iso-8859-2"),
    ("iso_8859-2", "iso-8859-2"),
    ("latin2", "iso-8859-2"),
    ("l2", "iso-8859-2"),
    ("iso8859-4", "iso-8859-4"),
    ("latin4", "iso-8859-4"),
    ("iso8859-5", "iso-8859-5"),
    ("iso88595", "iso-8859-5"),
    ("iso_8859-5", "iso-8859-5"),
    ("cyrillic", "iso-8859-5"),
    ("iso8859-6", "iso-8859-6"),
    ("arabic", "iso-8859-6"),
    ("iso8859-7", "iso-8859-7"),
    ("iso88597", "iso-8859-7"),
    ("iso_8859-7", "iso-8859-7"),
    ("greek", "iso-8859-7"),
    ("greek8", "iso-8859-7"),
    ("iso8859-8", "iso-8859-8"),
    ("hebrew", "iso-8859-8"),
    ("iso8859-13", "iso-8859-13"),
    ("latin7", "iso-8859-13"),
    ("iso8859-15", "iso-8859-15"),
    ("iso885915", "iso-8859-15"),
    ("latin9", "iso-8859-15"),
    ("l9", "iso-8859-15"),
    ("koi8", "koi8-r"),
    ("koi8_r", "koi8-r"),
    ("koi8-ru", "koi8-u"),
];

/// The set a name or label stands for, or `None` when this crate does not
/// decode it. Case and surrounding spaces do not matter.
pub fn lookup(label: &str) -> Option<Charset> {
    let label = label
        .trim_matches(|c: char| c.is_ascii_whitespace())
        .to_ascii_lowercase();
    match label.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => return Some(Charset::Utf8),
        "utf-16" | "utf-16le" | "ucs-2" | "unicode" => return Some(Charset::Utf16Le),
        "utf-16be" | "unicodefffe" => return Some(Charset::Utf16Be),
        _ => {}
    }
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == label)
        .map_or(label.as_str(), |(_, name)| name);
    SINGLE_BYTE_SETS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, table)| Charset::Single(known, table))
}

/// A page as text, and how it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub charset: Charset,
    /// A declared name this crate cannot decode, when there was one and the
    /// page was read as UTF-8 for lack of anything better.
    pub unsupported: Option<String>,
}

/// Reads `bytes` as text. `declared` is the `charset` of the `Content-Type`
/// header, quotes already removed or not.
pub fn decode_page(bytes: &[u8], declared: Option<&str>) -> Decoded {
    if let Some((charset, skip)) = byte_order_mark(bytes) {
        return Decoded {
            text: decode(&bytes[skip..], charset),
            charset,
            unsupported: None,
        };
    }

    let mut unsupported = None;
    let mut chosen = None;
    let header = declared
        .map(|d| d.trim_matches(|c| c == '"' || c == '\'').trim())
        .filter(|d| !d.is_empty());
    if let Some(label) = header {
        match lookup(label) {
            Some(charset) => chosen = Some(charset),
            None => unsupported = Some(label.to_owned()),
        }
    }
    if chosen.is_none()
        && let Some(label) = sniff_meta_charset(bytes)
    {
        match lookup(&label) {
            // A tag that is itself read as ASCII cannot have been in UTF-16.
            Some(Charset::Utf16Le | Charset::Utf16Be) => chosen = Some(Charset::Utf8),
            Some(charset) => chosen = Some(charset),
            None => {
                unsupported.get_or_insert(label);
            }
        }
    }

    let charset = chosen.unwrap_or(Charset::Utf8);
    // A declaration that was met and used leaves nothing to report.
    let unsupported = if chosen.is_some() { None } else { unsupported };
    Decoded {
        text: decode(bytes, charset),
        charset,
        unsupported,
    }
}

fn byte_order_mark(bytes: &[u8]) -> Option<(Charset, usize)> {
    match bytes {
        [0xEF, 0xBB, 0xBF, ..] => Some((Charset::Utf8, 3)),
        [0xFE, 0xFF, ..] => Some((Charset::Utf16Be, 2)),
        [0xFF, 0xFE, ..] => Some((Charset::Utf16Le, 2)),
        _ => None,
    }
}

/// The text of `bytes` in `charset`. Never fails: what is not a character
/// of the set is U+FFFD.
pub fn decode(bytes: &[u8], charset: Charset) -> String {
    match charset {
        Charset::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        Charset::Utf16Le => decode_utf16(bytes, u16::from_le_bytes),
        Charset::Utf16Be => decode_utf16(bytes, u16::from_be_bytes),
        Charset::Single(_, table) => bytes
            .iter()
            .map(|b| char::from_u32(u32::from(table[usize::from(*b)])).unwrap_or('\u{FFFD}'))
            .collect(),
    }
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|pair| unit(*pair));
    let mut text: String = char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect();
    if bytes.len() % 2 == 1 {
        text.push('\u{FFFD}');
    }
    text
}

/// The character set a `<meta>` tag in the first [`META_PRESCAN_BYTES`]
/// bytes names, either as `charset=` or in a `Content-Type` `http-equiv`.
pub fn sniff_meta_charset(bytes: &[u8]) -> Option<String> {
    let start = &bytes[..bytes.len().min(META_PRESCAN_BYTES)];
    // High bytes become U+FFFD, which no tag or attribute here is made of.
    let text = String::from_utf8_lossy(start);
    let mut at = 0;
    while let Some(lt) = text[at..].find('<') {
        let tag = at + lt;
        let rest = &text[tag..];
        if rest.starts_with("<!--") {
            at = tag + rest.find("-->")? + 3;
            continue;
        }
        at = tag + 1;
        if rest.len() > 6
            && rest.as_bytes()[..5].eq_ignore_ascii_case(b"<meta")
            && (rest.as_bytes()[5].is_ascii_whitespace() || rest.as_bytes()[5] == b'/')
        {
            let end = rest.find('>').unwrap_or(rest.len());
            if let Some(label) = charset_of_meta(&rest[..end]) {
                return Some(label);
            }
        }
    }
    None
}

fn charset_of_meta(tag: &str) -> Option<String> {
    let attributes = attributes(tag);
    if let Some((_, value)) = attributes.iter().find(|(name, _)| name == "charset") {
        let label = value.trim();
        return (!label.is_empty()).then(|| label.to_owned());
    }
    let is_content_type = attributes.iter().any(|(name, value)| {
        name == "http-equiv" && value.trim().eq_ignore_ascii_case("content-type")
    });
    if !is_content_type {
        return None;
    }
    let (_, content) = attributes.iter().find(|(name, _)| name == "content")?;
    let lower = content.to_ascii_lowercase();
    let after = lower.find("charset")? + "charset".len();
    let value = content[after..]
        .trim_start()
        .strip_prefix('=')?
        .trim_start();
    let value = value.trim_start_matches(['"', '\'']);
    let end = value
        .find(|c: char| c.is_ascii_whitespace() || matches!(c, ';' | '"' | '\''))
        .unwrap_or(value.len());
    (end > 0).then(|| value[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_to_sets() {
        for (label, name) in [
            ("UTF-8", "utf-8"),
            (" utf8 ", "utf-8"),
            ("iso-8859-1", "windows-1252"),
            ("Latin1", "windows-1252"),
            ("US-ASCII", "windows-1252"),
            ("windows-1250", "windows-1250"),
            ("CP1251", "windows-1251"),
            ("iso-8859-2", "iso-8859-2"),
            ("latin2", "iso-8859-2"),
            ("ISO-8859-15", "iso-8859-15"),
            ("iso-8859-9", "windows-1254"),
            ("koi8-r", "koi8-r"),
            ("utf-16", "utf-16le"),
            ("UTF-16BE", "utf-16be"),
        ] {
            assert_eq!(lookup(label).unwrap().name(), name, "{label}");
        }
        for label in [
            "shift_jis",
            "euc-kr",
            "gb2312",
            "big5",
            "utf-32",
            "utf-7",
            "klingon",
            "",
        ] {
            assert!(lookup(label).is_none(), "{label}");
        }
    }

    #[test]
    fn every_set_agrees_with_ascii_below_0x80() {
        for (name, table) in SINGLE_BYTE_SETS {
            for (byte, unit) in table.iter().enumerate().take(0x80) {
                assert_eq!(usize::from(*unit), byte, "{name} {byte:#x}");
            }
        }
    }

    #[test]
    fn decodes_what_the_header_declares() {
        let page = decode_page(b"<title>Caf\xe9 \xdcn\xefcode</title>", Some("iso-8859-1"));
        assert_eq!(page.text, "<title>Café Ünïcode</title>");
        assert_eq!(page.charset.name(), "windows-1252");
        assert_eq!(page.unsupported, None);

        let polish = decode_page(b"\xa3\xf3d\x9f \xb9\xea", Some("\"windows-1250\""));
        assert_eq!(polish.text, "Łódź ąę");
        let polish = decode_page(b"\xa3\xf3d\xbc \xb1\xea", Some("ISO-8859-2"));
        assert_eq!(polish.text, "Łódź ąę");
        let russian = decode_page(b"\xcf\xf0\xe8\xe2\xe5\xf2", Some("windows-1251"));
        assert_eq!(russian.text, "Привет");
        let koi = decode_page(b"\xf0\xd2\xc9\xd7\xc5\xd4", Some("koi8-r"));
        assert_eq!(koi.text, "Привет");
    }

    #[test]
    fn a_byte_order_mark_wins_over_the_header() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("Zażółć".as_bytes());
        let page = decode_page(&bytes, Some("windows-1250"));
        assert_eq!(page.text, "Zażółć");
        assert_eq!(page.charset, Charset::Utf8);

        let mut le = vec![0xFF, 0xFE];
        le.extend("Zażółć 😀".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_page(&le, Some("utf-8")).text, "Zażółć 😀");
        let mut be = vec![0xFE, 0xFF];
        be.extend("Zażółć".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode_page(&be, None).text, "Zażółć");
    }

    #[test]
    fn utf16_without_a_mark_is_read_as_declared_and_damage_is_replaced() {
        let bytes: Vec<u8> = "ab".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode_page(&bytes, Some("utf-16be")).text, "ab");
        assert_eq!(
            decode_utf16(&[0x00, 0xD8, 0x61, 0x00, 0x7a], u16::from_le_bytes),
            "\u{FFFD}a\u{FFFD}"
        );
    }

    #[test]
    fn a_meta_declaration_is_used_when_the_header_says_nothing() {
        let page = decode_page(
            b"<head><meta charset=\"windows-1250\"><title>\xa3\xf3d\x9f</title>",
            None,
        );
        assert_eq!(page.charset.name(), "windows-1250");
        assert!(page.text.contains("Łódź"));

        let old_style = b"<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=ISO-8859-2\"><title>\xa3\xf3d\xbc";
        assert!(decode_page(old_style, None).text.contains("Łódź"));
    }

    #[test]
    fn the_header_wins_over_meta_and_a_commented_meta_is_not_read() {
        let page = decode_page(
            b"<meta charset=windows-1251><title>\xc3\xa9</title>",
            Some("utf-8"),
        );
        assert_eq!(page.text, "<meta charset=windows-1251><title>é</title>");
        let commented = decode_page(
            b"<!-- <meta charset=windows-1251> --><title>\xc3\xa9</title>",
            None,
        );
        assert_eq!(commented.charset, Charset::Utf8);
        // A declaration past the prescan is too late.
        let mut late = vec![b' '; META_PRESCAN_BYTES];
        late.extend_from_slice(b"<meta charset=windows-1251>");
        assert_eq!(decode_page(&late, None).charset, Charset::Utf8);
        // A tag that says UTF-16 cannot be believed.
        assert_eq!(
            decode_page(b"<meta charset=utf-16><title>x", None).charset,
            Charset::Utf8
        );
    }

    #[test]
    fn an_unknown_or_unsupported_name_reads_as_utf8_and_says_so() {
        let page = decode_page("<title>Zażółć</title>".as_bytes(), Some("klingon"));
        assert_eq!(page.text, "<title>Zażółć</title>");
        assert_eq!(page.unsupported.as_deref(), Some("klingon"));

        let damaged = decode_page(b"<title>\x83\x41</title>", Some("shift_jis"));
        assert_eq!(damaged.text, "<title>\u{FFFD}A</title>");
        assert_eq!(damaged.unsupported.as_deref(), Some("shift_jis"));

        // Another source can still answer.
        let rescued = decode_page(b"<meta charset=windows-1251><title>\xcf", Some("klingon"));
        assert_eq!(rescued.charset.name(), "windows-1251");
        assert_eq!(rescued.unsupported, None);
    }

    #[test]
    fn invalid_utf8_becomes_the_replacement_character() {
        assert_eq!(decode_page(b"a\xffb\xc3", None).text, "a\u{FFFD}b\u{FFFD}");
    }

    #[test]
    fn holes_in_a_table_are_the_replacement_character() {
        // 0xA5 of ISO 8859-6 is not assigned.
        assert_eq!(decode(&[0xA5], lookup("iso-8859-6").unwrap()), "\u{FFFD}");
    }

    #[test]
    fn hostile_meta_prescan_does_not_loop() {
        let junk = "<meta ".repeat(2000);
        assert_eq!(sniff_meta_charset(junk.as_bytes()), None);
        assert_eq!(
            sniff_meta_charset(b"<!-- never closed <meta charset=x>"),
            None
        );
        assert_eq!(sniff_meta_charset(b"<meta charset=>"), None);
        assert_eq!(sniff_meta_charset(b"<meta charset"), None);
    }
}

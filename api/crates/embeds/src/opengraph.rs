//! Reading the few `<meta>` tags that make a link card.
//!
//! A tag scanner and not an HTML parser, which caps what a hostile page can
//! ask of this code: no tree is built, nothing is executed or followed, and
//! every value that comes out is plain text. The scan is a single pass, so a
//! page made of unfinished tags costs time in proportion to its size and no
//! more, and what is kept is at most one value for each of seven names.

use crate::link::Link;
use crate::text::{clean_text, decode_entities};

/// The longest title shown, in UTF-16 code units, ellipsis not counted.
pub const MAX_TITLE: usize = 200;

/// The longest description shown, in UTF-16 code units, ellipsis not counted.
pub const MAX_DESCRIPTION: usize = 400;

/// The longest `<title>` text that counts, in UTF-16 code units. A longer
/// element is not read as a title: a page that does not close it close to
/// its opening has no title worth showing, and the bound is what keeps the
/// search for the closing tag small.
pub const MAX_TITLE_ELEMENT: usize = 400;

/// What a page says about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageCard {
    pub title: Option<String>,
    pub description: Option<String>,
    /// An absolute `http` or `https` address, in the form [`Link`] writes.
    pub image_url: Option<String>,
}

impl PageCard {
    /// Whether the page said nothing a card can show.
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image_url.is_none()
    }
}

/// The names that are kept, in the order of the slots.
const WANTED: [&str; 7] = [
    "og:title",
    "twitter:title",
    "og:description",
    "twitter:description",
    "description",
    "og:image",
    "twitter:image",
];

/// Reads the card out of the head of a page. `page` is where the page was
/// finally served from, which relative addresses resolve against.
pub fn parse_open_graph(html: &str, page: &Link) -> PageCard {
    // Everything after the head is body copy, which anyone can put tags in.
    let head = &html[..head_end(html)];

    // The first tag that names a wanted key wins, whatever it says: an empty
    // og:title is the page's answer and the title element is not asked
    // after it.
    let mut meta: [Option<&str>; 7] = [None; 7];
    for_each_meta(head, |key, content| {
        if let Some(slot) = WANTED.iter().position(|wanted| *wanted == key)
            && meta[slot].is_none()
        {
            meta[slot] = Some(content);
        }
    });

    let first = |slots: &[usize]| slots.iter().find_map(|slot| meta[*slot]);
    let title = first(&[0, 1]).or_else(|| title_element(head));
    let description = first(&[2, 3, 4]);
    let image = first(&[5, 6]);

    PageCard {
        title: clean_text(title, MAX_TITLE),
        description: clean_text(description, MAX_DESCRIPTION),
        image_url: absolute_web_address(image, page),
    }
}

/// Where the head ends: at the first `</head`, whitespace and a `>` after it
/// being required, or the end of the page.
fn head_end(html: &str) -> usize {
    let mut from = 0;
    while let Some(start) = find_ignoring_case(html, "</head", from) {
        let after = start + "</head".len();
        let rest = &html[after..];
        let trimmed = rest.trim_start_matches(char::is_whitespace);
        if trimmed.starts_with('>') {
            return start;
        }
        from = start + 1;
    }
    html.len()
}

/// The first place at or after `from` where `needle`, which is lower-case
/// ASCII, stands in `haystack` in any case.
fn find_ignoring_case(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    let first = needle[0];
    let mut at = from;
    while at + needle.len() <= hay.len() {
        let offset = hay[at..=hay.len() - needle.len()]
            .iter()
            .position(|b| b.to_ascii_lowercase() == first)?;
        at += offset;
        if hay[at..at + needle.len()].eq_ignore_ascii_case(needle) {
            return Some(at);
        }
        at += 1;
    }
    None
}

/// Calls `found` with the key and content of every `<meta>` tag that has
/// both, the key being its `property`, `name` or `itemprop`, in lower case
/// and in the one spelling this reads for what pages use interchangeably.
///
/// A tag runs to the first `>`, quoted or not, so a value with a bracket in
/// it is cut off and the tag says nothing.
fn for_each_meta<'a>(head: &'a str, mut found: impl FnMut(&str, &'a str)) {
    const OPEN: &str = "<meta";
    let mut position = 0;
    while let Some(start) = find_ignoring_case(head, OPEN, position) {
        let after_name = start + OPEN.len();
        position = after_name;
        if !head[after_name..].starts_with(char::is_whitespace) {
            continue;
        }
        let Some(length) = head[after_name..].find('>') else {
            // No bracket left anywhere after this point, so no tag either.
            break;
        };
        let end = after_name + length;
        position = end + 1;

        let mut key: Option<String> = None;
        let mut content: Option<&str> = None;
        for (name, value) in attributes(&head[start..end]) {
            match name.as_str() {
                "property" | "name" | "itemprop" if key.is_none() => {
                    key = Some(value.trim().to_lowercase());
                }
                "content" if content.is_none() => content = Some(value),
                _ => {}
            }
        }
        if let (Some(key), Some(content)) = (key, content) {
            found(canonical(&key), content);
        }
    }
}

/// The one name for a thing that pages spell several ways.
fn canonical(key: &str) -> &str {
    match key {
        "og:image:url" | "og:image:secure_url" => "og:image",
        "twitter:image:src" => "twitter:image",
        other => other,
    }
}

/// The `name=value` pairs of a tag, names in lower case. A name is letters,
/// colons, underscores and hyphens; a value is quoted with either kind of
/// quote or bare. What is neither, such as an attribute whose quote is never
/// closed, is skipped over.
pub(crate) fn attributes(tag: &str) -> Vec<(String, &str)> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        if !is_name_byte(bytes[position]) {
            position += 1;
            continue;
        }
        let mut name_end = position;
        while name_end < bytes.len() && is_name_byte(bytes[name_end]) {
            name_end += 1;
        }
        let cursor = skip_whitespace(tag, name_end);
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            position = name_end;
            continue;
        }
        let Some((value, value_end)) = read_value(tag, skip_whitespace(tag, cursor + 1)) else {
            position = name_end;
            continue;
        };
        out.push((tag[position..name_end].to_ascii_lowercase(), value));
        position = value_end;
    }
    out
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b':' | b'_' | b'-')
}

fn skip_whitespace(text: &str, position: usize) -> usize {
    let rest = &text[position..];
    position + (rest.len() - rest.trim_start_matches(char::is_whitespace).len())
}

fn read_value(tag: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = tag.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    let quote = bytes[start];
    if quote == b'"' || quote == b'\'' {
        if let Some(length) = tag[start + 1..].find(quote as char) {
            return Some((&tag[start + 1..start + 1 + length], start + 1 + length + 1));
        }
        // A double quote that is never closed starts no value; a single one
        // is only an ordinary character of a bare value.
        if quote == b'"' {
            return None;
        }
    }
    let rest = &tag[start..];
    let length = rest
        .find(|c: char| c.is_whitespace() || c == '"')
        .unwrap_or(rest.len());
    Some((&rest[..length], start + length))
}

/// The text of the first `<title>` element, when it is closed within
/// [`MAX_TITLE_ELEMENT`] characters of its opening tag.
fn title_element(head: &str) -> Option<&str> {
    const OPEN: &str = "<title";
    const CLOSE: &str = "</title>";
    let mut position = 0;
    let mut bracket: Option<usize> = None;
    while let Some(start) = find_ignoring_case(head, OPEN, position) {
        position = start + OPEN.len();
        // Another opening inside a tag already tried looks at the same text.
        if bracket.is_some_and(|bracket| bracket >= position) {
            continue;
        }
        bracket = Some(position + head[position..].find('>')?);
        let text_start = bracket? + 1;
        let window = window_of(&head[text_start..], MAX_TITLE_ELEMENT + CLOSE.len());
        if let Some(length) = find_ignoring_case(window, CLOSE, 0) {
            return Some(&head[text_start..text_start + length]);
        }
    }
    None
}

/// The first `units` UTF-16 code units of `text`, as far as whole characters
/// go.
fn window_of(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (at, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..at];
        }
    }
    text
}

/// Resolves an image address against the page. Anything that is not `http`
/// or `https` once resolved, such as `data:` or `javascript:`, is dropped.
fn absolute_web_address(value: Option<&str>, page: &Link) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    let decoded = decode_entities(value);
    page.join(decoded.trim()).map(|link| link.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Link {
        Link::parse("https://example.org/article").unwrap()
    }

    fn card(html: &str) -> PageCard {
        parse_open_graph(html, &page())
    }

    #[test]
    fn prefers_open_graph_over_the_title_tag() {
        let card = card(
            r#"<html><head>
              <title>Fallback</title>
              <meta property="og:title" content="Prawdziwy tytuł">
              <meta property="og:description" content="Opis   ze  spacjami">
              <meta property="og:image" content="/media/cover.png">
            </head><body>…</body></html>"#,
        );
        assert_eq!(card.title.as_deref(), Some("Prawdziwy tytuł"));
        assert_eq!(card.description.as_deref(), Some("Opis ze spacjami"));
        assert_eq!(
            card.image_url.as_deref(),
            Some("https://example.org/media/cover.png")
        );
    }

    #[test]
    fn falls_back_to_the_title_tag_and_meta_description() {
        let card = card(
            r#"<html><head><title>Tytuł &amp; więcej</title><meta name="description" content="Opis"></head></html>"#,
        );
        assert_eq!(card.title.as_deref(), Some("Tytuł & więcej"));
        assert_eq!(card.description.as_deref(), Some("Opis"));
        assert_eq!(card.image_url, None);
    }

    #[test]
    fn ignores_metadata_that_is_only_in_the_body() {
        let card = card(
            r#"<html><head><title>Prawdziwy</title></head>
            <body><meta property="og:title" content="Podstawiony"></body></html>"#,
        );
        assert_eq!(card.title.as_deref(), Some("Prawdziwy"));
    }

    #[test]
    fn drops_image_addresses_that_are_not_http() {
        for image in [
            "javascript:alert(1)",
            "data:image/svg+xml,<svg onload='alert(1)'/>",
        ] {
            let html = format!(
                r#"<html><head><meta property="og:title" content="X"><meta property="og:image" content="{image}"></head></html>"#
            );
            assert_eq!(card(&html).image_url, None, "{image}");
        }
    }

    #[test]
    fn caps_runaway_values() {
        let html = format!(
            r#"<html><head><meta property="og:title" content="{}"></head></html>"#,
            "a".repeat(500)
        );
        let title = card(&html).title.unwrap();
        assert!(
            title.chars().count() <= 201,
            "title was {} characters",
            title.chars().count()
        );
    }

    #[test]
    fn reads_tags_whatever_their_quoting_and_order() {
        let card = card(
            r#"<html><head>
            <META content='Quoted with a space' PROPERTY=og:title>
            <meta name = "description"
                  content="Spread over lines">
            <meta property="og:image:secure_url" content="https://cdn.example/a.png">
            <meta property="og:image" content="https://cdn.example/b.png">
            </head></html>"#,
        );
        assert_eq!(card.title.as_deref(), Some("Quoted with a space"));
        assert_eq!(card.description.as_deref(), Some("Spread over lines"));
        assert_eq!(card.image_url.as_deref(), Some("https://cdn.example/a.png"));
    }

    #[test]
    fn takes_the_head_to_end_only_at_its_closing_tag() {
        let card = card(
            r#"<header></header>
            <meta property="og:title" content="Before">
            </head >
            <meta property="og:description" content="After">"#,
        );
        // The header element is not the head, and a closing tag may have a
        // space in it.
        assert_eq!(card.title.as_deref(), Some("Before"));
        assert_eq!(card.description, None);
    }

    #[test]
    fn an_unfinished_page_costs_time_in_proportion_to_its_size() {
        for fragment in [
            "<meta ",
            "<title ",
            "<meta content=\"",
            "<meta a=b ",
            "</head ",
            "<title>",
        ] {
            let html = fragment.repeat(500 * 1024 / fragment.len());
            let started = std::time::Instant::now();
            card(&html);
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "{fragment}: {:?}",
                started.elapsed()
            );
        }
    }

    #[test]
    fn never_cuts_a_multi_unit_character_in_half() {
        let html = format!(
            r#"<head><meta property="og:title" content="{}😀 and more"></head>"#,
            "a".repeat(199)
        );
        assert_eq!(card(&html).title.unwrap(), format!("{}…", "a".repeat(199)));
    }

    #[test]
    fn an_empty_title_from_meta_is_the_answer() {
        let card =
            card(r#"<head><meta property="og:title" content=""><title>Fallback</title></head>"#);
        assert_eq!(card.title, None);
    }

    #[test]
    fn the_first_tag_of_a_name_wins_and_other_names_are_read() {
        let card = card(
            r#"<head><meta property="og:title" content="One"><meta name="og:title" content="Two">
            <meta property="twitter:title" content="Three"><meta itemprop="description" content="Itemprop"></head>"#,
        );
        assert_eq!(card.title.as_deref(), Some("One"));
        assert_eq!(card.description.as_deref(), Some("Itemprop"));
    }

    #[test]
    fn falls_back_through_the_twitter_names() {
        let card = card(
            r#"<head><meta name="twitter:title" content="T"><meta name="twitter:description" content="D">
            <meta name="twitter:image:src" content="//cdn.example/i.jpg"></head>"#,
        );
        assert_eq!(card.title.as_deref(), Some("T"));
        assert_eq!(card.description.as_deref(), Some("D"));
        assert_eq!(card.image_url.as_deref(), Some("https://cdn.example/i.jpg"));
    }

    #[test]
    fn a_title_element_must_close_close_by() {
        let long = format!("<head><title>{}</title></head>", "a".repeat(450));
        assert_eq!(card(&long).title, None);
        let near = format!("<head><title>{}</title></head>", "a".repeat(390));
        assert!(card(&near).title.is_some());
        // The first opening that is not closed in time does not hide a later one.
        assert_eq!(
            card(&format!(
                "<head><title>{} <title>Second</title></head>",
                "x".repeat(450)
            ))
            .title
            .as_deref(),
            Some("Second")
        );
    }

    #[test]
    fn an_attribute_value_with_a_bracket_is_cut_at_it() {
        let card = card(r#"<head><meta property="og:title" content="a > b"></head>"#);
        assert_eq!(card.title, None);
    }

    #[test]
    fn an_unclosed_double_quote_starts_no_value_and_a_single_one_is_a_bare_value() {
        assert_eq!(
            card(r#"<head><meta property="og:title" content="open></head>"#).title,
            None
        );
        assert_eq!(
            card("<head><meta property=og:title content='open>")
                .title
                .as_deref(),
            Some("'open")
        );
    }

    #[test]
    fn keys_are_trimmed_and_lowered() {
        let card = card(r#"<head><meta property="  OG:Title " content="K"></head>"#);
        assert_eq!(card.title.as_deref(), Some("K"));
    }

    #[test]
    fn decodes_the_reference_in_an_image_address() {
        let card = card(r#"<head><meta property="og:image" content="/a.png?x=1&amp;y=2"></head>"#);
        assert_eq!(
            card.image_url.as_deref(),
            Some("https://example.org/a.png?x=1&y=2")
        );
    }

    #[test]
    fn a_page_of_nothing_is_an_empty_card() {
        assert!(card("").is_empty());
        assert!(card("\0\0<<<>>>").is_empty());
        assert!(card("<head></head><title>Late</title>").is_empty());
    }
}

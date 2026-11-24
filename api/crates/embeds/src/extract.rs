//! Finding the links in a message's text.
//!
//! Follows the rules the web client autolinks by: `http` and `https` only, a
//! link runs to the next whitespace or character that cannot stand in an
//! address, and punctuation at its end belongs to the sentence. If the two
//! disagreed, a transcript would show a link that is never previewed, or a
//! card for text a reader cannot click.

use std::collections::HashSet;

use crate::link::Link;

/// Punctuation that ends a sentence and so is not part of the link before it.
const SENTENCE_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', '…', '\'', '"'];

/// The first `limit` distinct links of `content`, in reading order. Two
/// spellings of one page are two links: deciding they are the same means
/// rewriting somebody else's address, which is a guess.
pub fn extract_links(content: &str, limit: usize) -> Vec<Link> {
    let mut links = Vec::new();
    if limit == 0 {
        return links;
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut at = 0;
    while let Some(start) = next_candidate(content, at) {
        let end = candidate_end(content, start);
        at = end;
        let text = trim_trailing(&content[start..end]);
        // Something that looks like a link can still fail to read, with a
        // control character in the host for one. That is text.
        if seen.insert(text)
            && let Some(link) = Link::parse(text)
        {
            links.push(link);
            if links.len() == limit {
                break;
            }
        }
    }
    links
}

/// The length of `http://` or `https://` at the start of `bytes`, in any
/// case.
fn scheme_length(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 7 || !bytes[..4].eq_ignore_ascii_case(b"http") {
        return None;
    }
    let after = if bytes[4].eq_ignore_ascii_case(&b's') {
        5
    } else {
        4
    };
    (bytes.len() >= after + 3 && &bytes[after..after + 3] == b"://").then_some(after + 3)
}

/// The next place from `at` where a scheme starts and is followed by at
/// least one character that can be in a link.
fn next_candidate(content: &str, at: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    (at..bytes.len()).find(|&position| {
        scheme_length(&bytes[position..]).is_some_and(|length| {
            content[position + length..]
                .chars()
                .next()
                .is_some_and(is_link_character)
        })
    })
}

fn candidate_end(content: &str, start: usize) -> usize {
    let after_scheme = start + scheme_length(&content.as_bytes()[start..]).unwrap_or(0);
    let run = content[after_scheme..]
        .find(|c| !is_link_character(c))
        .unwrap_or(content.len() - after_scheme);
    after_scheme + run
}

/// What may stand in a link: anything but whitespace, the angle brackets and
/// the quotes and backtick that wrap a link in running text.
fn is_link_character(c: char) -> bool {
    !(c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
}

fn trim_trailing(mut run: &str) -> &str {
    // Counted once and kept up to date as the end is cut, so a run of a
    // million closing brackets costs a million steps and not a million squared.
    let mut round = Brackets::of(run, '(', ')');
    let mut square = Brackets::of(run, '[', ']');
    while let Some(last) = run.chars().next_back() {
        let cut = match last {
            ')' => round.close_is_unopened(),
            ']' => square.close_is_unopened(),
            other => SENTENCE_PUNCTUATION.contains(&other),
        };
        if !cut {
            break;
        }
        match last {
            ')' => round.closing -= 1,
            ']' => square.closing -= 1,
            _ => {}
        }
        run = &run[..run.len() - last.len_utf8()];
    }
    run
}

/// How many brackets of one kind a run has. A closing bracket belongs to the
/// link only when the link opened it, so "(see https://x/a)" leaves the
/// parenthesis out and "https://x/a_(b)" keeps it.
struct Brackets {
    opening: usize,
    closing: usize,
}

impl Brackets {
    fn of(run: &str, opening: char, closing: char) -> Brackets {
        Brackets {
            opening: run.matches(opening).count(),
            closing: run.matches(closing).count(),
        }
    }

    fn close_is_unopened(&self) -> bool {
        self.closing > self.opening
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(content: &str, limit: usize) -> Vec<String> {
        extract_links(content, limit)
            .iter()
            .map(Link::to_string)
            .collect()
    }

    #[test]
    fn finds_links_in_order_and_drops_duplicates() {
        assert_eq!(
            texts(
                "zobacz https://example.org/a oraz https://example.org/b i znowu https://example.org/a",
                5
            ),
            ["https://example.org/a", "https://example.org/b"]
        );
    }

    #[test]
    fn trims_trailing_punctuation_the_way_the_client_does() {
        for (content, expected) in [
            ("czytaj https://example.org/a.", "https://example.org/a"),
            (
                "czytaj https://example.org/a, potem",
                "https://example.org/a",
            ),
            ("(patrz https://example.org/a)", "https://example.org/a"),
            ("https://example.org/a_(b)", "https://example.org/a_(b)"),
            ("[https://example.org/a]", "https://example.org/a"),
            ("https://example.org/a?!…", "https://example.org/a"),
            ("\"https://example.org/a\"", "https://example.org/a"),
            ("<https://example.org/a>", "https://example.org/a"),
        ] {
            assert_eq!(texts(content, 5), [expected], "{content}");
        }
    }

    #[test]
    fn ignores_schemes_that_are_not_http() {
        assert!(
            texts(
                "javascript:alert(1) file:///etc/passwd ftp://example.org",
                5
            )
            .is_empty()
        );
    }

    #[test]
    fn stops_at_the_limit() {
        assert_eq!(
            texts("https://a.example https://b.example https://c.example", 2).len(),
            2
        );
        assert!(texts("https://a.example", 0).is_empty());
    }

    #[test]
    fn reads_the_scheme_in_any_case_and_a_link_inside_a_word() {
        assert_eq!(texts("HTTPS://Example.org/A", 5), ["https://example.org/A"]);
        assert_eq!(texts("xhttp://example.org", 5), ["http://example.org/"]);
    }

    #[test]
    fn a_scheme_with_nothing_after_it_is_text() {
        assert!(texts("http:// https://  http://<", 5).is_empty());
    }

    #[test]
    fn a_link_that_does_not_read_is_skipped_and_the_next_is_found() {
        assert_eq!(
            texts("http://exa\u{7}mple.org/ https://ok.example/", 1),
            ["https://ok.example/"]
        );
    }

    #[test]
    fn many_candidates_cost_time_in_proportion_to_the_text() {
        let content = "http://".repeat(200_000) + &"https://a ".repeat(1000);
        let started = std::time::Instant::now();
        let links = extract_links(&content, 2);
        assert!(links.len() <= 2);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn non_ascii_text_around_a_link_does_not_confuse_the_cut() {
        assert_eq!(
            texts("łódź→https://example.org/ż…ó", 5),
            ["https://example.org/ż…ó"]
        );
        assert_eq!(
            texts("https://example.org/ą…", 5),
            ["https://example.org/ą"]
        );
    }
}

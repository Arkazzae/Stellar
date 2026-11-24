//! From what somebody typed to what the full-text index is asked.
//!
//! The text is untrusted and goes into a `MATCH` expression, which has a
//! syntax of its own (quotes, stars, colons, parentheses, `AND`, `OR`, `NOT`,
//! `NEAR`, a minus). Nothing a person types is ever passed on as syntax: the
//! words are taken out of it, and the expression is built from them alone.

use std::collections::HashSet;

use stellarr_unicode::is_letter_or_number;

use super::ordinal_case::fold;

/// The most words a search is made of; the rest are not looked for.
const MAX_TERMS: usize = 16;
const LOWER_STROKE_L: char = '\u{142}';
const UPPER_STROKE_L: char = '\u{141}';

/// Turns what somebody typed into an expression that cannot contain an
/// operator of the syntax: every word is required and matches as a prefix, so
/// "rap" finds "raport.pdf", and the punctuation of an address does no harm.
/// None when there is no word in it.
pub fn build_query(query: &str) -> Option<String> {
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for word in words(query) {
        // Two spellings that differ only in case are one word, and the first
        // one counts.
        let key: String = word.chars().map(fold).collect();
        if seen.insert(key) {
            terms.push(prefix_term(&word));
            if terms.len() == MAX_TERMS {
                break;
            }
        }
    }
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

/// Finds runs of letters and numbers in UTF-16, in
/// which a character beyond the first 65,536 is two halves that are neither
/// letters nor numbers, so it ends a word like any mark does.
fn words(query: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    for c in query.chars() {
        if c.len_utf16() == 1 && is_letter_or_number(c) {
            current.push(c);
        } else if !current.is_empty() {
            found.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// A word as a prefix match. The index folds accented letters but not the
/// Polish l with a stroke, which counts as a letter of its own, so a word is
/// also searched with the two l letters exchanged: that is how a Polish word
/// typed on a keyboard without Polish letters is still found.
fn prefix_term(word: &str) -> String {
    let prefix = format!("\"{word}\"*");
    match exchange_l(word) {
        Some(other) => format!("({prefix} OR \"{other}\"*)"),
        None => prefix,
    }
}

fn exchange_l(word: &str) -> Option<String> {
    if word.contains([LOWER_STROKE_L, UPPER_STROKE_L]) {
        return Some(
            word.chars()
                .map(|c| match c {
                    LOWER_STROKE_L => 'l',
                    UPPER_STROKE_L => 'L',
                    other => other,
                })
                .collect(),
        );
    }
    word.contains(['l', 'L']).then(|| {
        word.chars()
            .map(|c| match c {
                'l' => LOWER_STROKE_L,
                'L' => UPPER_STROKE_L,
                other => other,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_is_required_and_a_prefix() {
        assert_eq!(build_query("rap").as_deref(), Some("\"rap\"*"));
        assert_eq!(
            build_query("kot pies").as_deref(),
            Some("\"kot\"* AND \"pies\"*")
        );
    }

    #[test]
    fn the_polish_l_is_searched_both_ways() {
        assert_eq!(
            build_query("lampa").as_deref(),
            Some("(\"lampa\"* OR \"\u{142}ampa\"*)")
        );
        assert_eq!(
            build_query("\u{141}\u{f3}d\u{17a}").as_deref(),
            Some("(\"\u{141}\u{f3}d\u{17a}\"* OR \"L\u{f3}d\u{17a}\"*)")
        );
    }

    #[test]
    fn nothing_of_the_syntax_gets_through() {
        assert_eq!(build_query("\"*:()-"), None);
        assert_eq!(build_query("   "), None);
        assert_eq!(build_query(""), None);
        assert_eq!(
            build_query("a\" OR b NEAR(c) -d NOT e").as_deref(),
            Some("\"a\"* AND \"OR\"* AND \"b\"* AND \"NEAR\"* AND \"c\"* AND \"d\"* AND \"NOT\"* AND \"e\"*")
        );
    }

    #[test]
    fn a_word_given_twice_in_any_case_is_one_word_and_there_are_sixteen_at_most() {
        assert_eq!(build_query("Kot kot KOT").as_deref(), Some("\"Kot\"*"));
        let many: Vec<String> = (0..30).map(|n| format!("w{n}")).collect();
        let built = build_query(&many.join(" ")).unwrap();
        assert_eq!(built.matches(" AND ").count(), 15);
    }

    #[test]
    fn a_character_beyond_the_first_plane_ends_a_word() {
        assert_eq!(build_query("a\u{1D400}b").as_deref(), Some("\"a\"* AND \"b\"*"));
        assert_eq!(build_query("\u{1D400}"), None);
    }
}

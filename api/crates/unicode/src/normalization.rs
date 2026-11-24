//! Canonical composition (NFC) as the first server's platform makes it: the
//! text decomposed by the canonical mappings, its runs of combining marks put
//! in canonical order, and composed again.
//!
//! The data is in `normalization_tables.rs`, generated from the ICU library of
//! the first server's image (see the crate's documentation). A character the
//! tables do not know is left as it is.

use crate::normalization_tables::{COMBINING_CLASS, COMPOSITION, DECOMPOSITION};

/// The text in canonical composition. Nothing in it is refused: what a
/// caller must refuse (U+FFFE, which the first server's runtime takes for a
/// broken text) it refuses itself.
pub fn nfc(text: &str) -> String {
    // Nothing below U+0300 decomposes into anything that does not compose
    // back, nor combines with what came before.
    if text.chars().all(|c| u32::from(c) < 0x300) {
        return text.to_owned();
    }
    let mut decomposed = Vec::with_capacity(text.len());
    for c in text.chars() {
        decompose(c, &mut decomposed);
    }
    // Canonical order: every run of combining marks sorted by class, keeping
    // the order of marks of one class. The sort is stable.
    let mut start = 0;
    while start < decomposed.len() {
        let length = decomposed
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&(_, class)| class != 0)
            .count();
        if let Some(run) = decomposed.get_mut(start..start + length) {
            run.sort_by_key(|&(_, class)| class);
        }
        start += length.max(1);
    }
    compose(&decomposed)
}

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// Appends the full canonical decomposition of a character, each with its
/// combining class.
fn decompose(c: char, out: &mut Vec<(char, u8)>) {
    let code = u32::from(c);
    if (S_BASE..S_BASE + S_COUNT).contains(&code) {
        // Hangul syllables decompose by arithmetic into their jamo.
        let index = code - S_BASE;
        let trailing = T_BASE + index % T_COUNT;
        let jamo = [
            L_BASE + index / N_COUNT,
            V_BASE + (index % N_COUNT) / T_COUNT,
        ];
        out.extend(jamo.into_iter().filter_map(char::from_u32).map(|c| (c, 0)));
        if trailing != T_BASE {
            out.extend(char::from_u32(trailing).map(|c| (c, 0)));
        }
        return;
    }
    let found = DECOMPOSITION
        .binary_search_by_key(&code, |&(from, _, _)| from)
        .ok()
        .and_then(|at| DECOMPOSITION.get(at));
    match found {
        Some(&(_, first, second)) => {
            for part in [first, second] {
                if let Some(part) = char::from_u32(part).filter(|_| part != 0) {
                    decompose(part, out);
                }
            }
        }
        None => out.push((c, combining_class(code))),
    }
}

fn combining_class(code: u32) -> u8 {
    COMBINING_CLASS
        .binary_search_by(|&(first, last, _)| {
            if last < code {
                std::cmp::Ordering::Less
            } else if first > code {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .ok()
        .and_then(|at| COMBINING_CLASS.get(at))
        .map_or(0, |&(_, _, class)| class)
}

/// Composes a decomposed text in canonical order: each mark joins the last
/// starter before it when nothing between them blocks it (a mark of the same
/// or a higher class), and starters next to each other join too.
fn compose(decomposed: &[(char, u8)]) -> String {
    let mut out: Vec<char> = Vec::with_capacity(decomposed.len());
    let mut starter: Option<usize> = None;
    // The class of the last character kept since the starter; none when the
    // character in hand follows the starter directly.
    let mut last_class: Option<u8> = None;
    for &(c, class) in decomposed {
        let blocked = last_class.is_some_and(|last| last == 0 || last >= class);
        if !blocked {
            let joined = starter.and_then(|at| {
                let slot = out.get_mut(at)?;
                *slot = compose_pair(*slot, c)?;
                Some(())
            });
            if joined.is_some() {
                continue;
            }
        }
        if class == 0 {
            starter = Some(out.len());
            last_class = None;
        } else {
            last_class = Some(class);
        }
        out.push(c);
    }
    out.into_iter().collect()
}

/// The primary composite of two characters, if they have one.
fn compose_pair(first: char, second: char) -> Option<char> {
    let (first, second) = (u32::from(first), u32::from(second));
    if (L_BASE..L_BASE + L_COUNT).contains(&first) && (V_BASE..V_BASE + V_COUNT).contains(&second) {
        let syllable = S_BASE + ((first - L_BASE) * V_COUNT + (second - V_BASE)) * T_COUNT;
        return char::from_u32(syllable);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&first)
        && (first - S_BASE).is_multiple_of(T_COUNT)
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&second)
    {
        return char::from_u32(first + (second - T_BASE));
    }
    COMPOSITION
        .binary_search_by_key(&(first, second), |&(a, b, _)| (a, b))
        .ok()
        .and_then(|at| COMPOSITION.get(at))
        .and_then(|&(_, _, composite)| char::from_u32(composite))
}

#[cfg(test)]
mod tests {
    use super::nfc;

    #[test]
    fn text_below_the_first_combining_mark_is_not_touched() {
        assert_eq!(nfc("plain.text@example.test"), "plain.text@example.test");
    }

    #[test]
    fn a_base_and_its_mark_compose_and_a_composed_letter_stays() {
        assert_eq!(nfc("e\u{301}"), "\u{e9}");
        assert_eq!(nfc("\u{e9}"), "\u{e9}");
        // The Kelvin sign decomposes to a letter and does not come back.
        assert_eq!(nfc("\u{212a}"), "K");
        // A singleton, a Hangul syllable made of jamo, and one that already is.
        assert_eq!(nfc("\u{1100}\u{1161}\u{11a8}"), "\u{ac01}");
        assert_eq!(nfc("\u{ac01}"), "\u{ac01}");
    }

    #[test]
    fn marks_are_put_in_canonical_order_before_they_compose() {
        // Dot below (220) goes before dot above (230); each then joins the base.
        assert_eq!(nfc("a\u{307}\u{323}"), "\u{1ea1}\u{307}");
        assert_eq!(nfc("a\u{323}\u{307}"), "\u{1ea1}\u{307}");
    }
}

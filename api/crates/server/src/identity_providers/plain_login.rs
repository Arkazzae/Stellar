//! The login to suggest for an account made from what a provider said about a
//! person: the web client's rule, which the first server kept so that both
//! suggest the same name.
//!
//! The rule lower-cases the text, takes the accents off (decomposes it and
//! drops the combining marks), and keeps ASCII letters and digits with one
//! separator at most between them. No Unicode decomposition data is in the
//! workspace, so [`FOLDED`] lists what each character that matters becomes,
//! read from the first server's own runtime (`tests/providers-witness`
//! writes it): the one letter, digit or separator it leaves, or a space for
//! white space. A character that is not here leaves nothing.

/// What a character other than ASCII becomes once it is lower-cased and
/// decomposed, for the characters that leave something. Sorted.
const FOLDED: &[(char, char)] = &[
    ('\u{0085}', ' '),
    ('\u{00A0}', ' '),
    ('\u{00E0}', 'a'),
    ('\u{00E1}', 'a'),
    ('\u{00E2}', 'a'),
    ('\u{00E3}', 'a'),
    ('\u{00E4}', 'a'),
    ('\u{00E5}', 'a'),
    ('\u{00E7}', 'c'),
    ('\u{00E8}', 'e'),
    ('\u{00E9}', 'e'),
    ('\u{00EA}', 'e'),
    ('\u{00EB}', 'e'),
    ('\u{00EC}', 'i'),
    ('\u{00ED}', 'i'),
    ('\u{00EE}', 'i'),
    ('\u{00EF}', 'i'),
    ('\u{00F1}', 'n'),
    ('\u{00F2}', 'o'),
    ('\u{00F3}', 'o'),
    ('\u{00F4}', 'o'),
    ('\u{00F5}', 'o'),
    ('\u{00F6}', 'o'),
    ('\u{00F9}', 'u'),
    ('\u{00FA}', 'u'),
    ('\u{00FB}', 'u'),
    ('\u{00FC}', 'u'),
    ('\u{00FD}', 'y'),
    ('\u{00FF}', 'y'),
    ('\u{0101}', 'a'),
    ('\u{0103}', 'a'),
    ('\u{0105}', 'a'),
    ('\u{0107}', 'c'),
    ('\u{0109}', 'c'),
    ('\u{010B}', 'c'),
    ('\u{010D}', 'c'),
    ('\u{010F}', 'd'),
    ('\u{0113}', 'e'),
    ('\u{0115}', 'e'),
    ('\u{0117}', 'e'),
    ('\u{0119}', 'e'),
    ('\u{011B}', 'e'),
    ('\u{011D}', 'g'),
    ('\u{011F}', 'g'),
    ('\u{0121}', 'g'),
    ('\u{0123}', 'g'),
    ('\u{0125}', 'h'),
    ('\u{0129}', 'i'),
    ('\u{012B}', 'i'),
    ('\u{012D}', 'i'),
    ('\u{012F}', 'i'),
    ('\u{0135}', 'j'),
    ('\u{0137}', 'k'),
    ('\u{013A}', 'l'),
    ('\u{013C}', 'l'),
    ('\u{013E}', 'l'),
    ('\u{0144}', 'n'),
    ('\u{0146}', 'n'),
    ('\u{0148}', 'n'),
    ('\u{014D}', 'o'),
    ('\u{014F}', 'o'),
    ('\u{0151}', 'o'),
    ('\u{0155}', 'r'),
    ('\u{0157}', 'r'),
    ('\u{0159}', 'r'),
    ('\u{015B}', 's'),
    ('\u{015D}', 's'),
    ('\u{015F}', 's'),
    ('\u{0161}', 's'),
    ('\u{0163}', 't'),
    ('\u{0165}', 't'),
    ('\u{0169}', 'u'),
    ('\u{016B}', 'u'),
    ('\u{016D}', 'u'),
    ('\u{016F}', 'u'),
    ('\u{0171}', 'u'),
    ('\u{0173}', 'u'),
    ('\u{0175}', 'w'),
    ('\u{0177}', 'y'),
    ('\u{017A}', 'z'),
    ('\u{017C}', 'z'),
    ('\u{017E}', 'z'),
    ('\u{01A1}', 'o'),
    ('\u{01B0}', 'u'),
    ('\u{01CE}', 'a'),
    ('\u{01D0}', 'i'),
    ('\u{01D2}', 'o'),
    ('\u{01D4}', 'u'),
    ('\u{01D6}', 'u'),
    ('\u{01D8}', 'u'),
    ('\u{01DA}', 'u'),
    ('\u{01DC}', 'u'),
    ('\u{01DF}', 'a'),
    ('\u{01E1}', 'a'),
    ('\u{01E7}', 'g'),
    ('\u{01E9}', 'k'),
    ('\u{01EB}', 'o'),
    ('\u{01ED}', 'o'),
    ('\u{01F0}', 'j'),
    ('\u{01F5}', 'g'),
    ('\u{01F9}', 'n'),
    ('\u{01FB}', 'a'),
    ('\u{0201}', 'a'),
    ('\u{0203}', 'a'),
    ('\u{0205}', 'e'),
    ('\u{0207}', 'e'),
    ('\u{0209}', 'i'),
    ('\u{020B}', 'i'),
    ('\u{020D}', 'o'),
    ('\u{020F}', 'o'),
    ('\u{0211}', 'r'),
    ('\u{0213}', 'r'),
    ('\u{0215}', 'u'),
    ('\u{0217}', 'u'),
    ('\u{0219}', 's'),
    ('\u{021B}', 't'),
    ('\u{021F}', 'h'),
    ('\u{0227}', 'a'),
    ('\u{0229}', 'e'),
    ('\u{022B}', 'o'),
    ('\u{022D}', 'o'),
    ('\u{022F}', 'o'),
    ('\u{0231}', 'o'),
    ('\u{0233}', 'y'),
    ('\u{1680}', ' '),
    ('\u{1E01}', 'a'),
    ('\u{1E03}', 'b'),
    ('\u{1E05}', 'b'),
    ('\u{1E07}', 'b'),
    ('\u{1E09}', 'c'),
    ('\u{1E0B}', 'd'),
    ('\u{1E0D}', 'd'),
    ('\u{1E0F}', 'd'),
    ('\u{1E11}', 'd'),
    ('\u{1E13}', 'd'),
    ('\u{1E15}', 'e'),
    ('\u{1E17}', 'e'),
    ('\u{1E19}', 'e'),
    ('\u{1E1B}', 'e'),
    ('\u{1E1D}', 'e'),
    ('\u{1E1F}', 'f'),
    ('\u{1E21}', 'g'),
    ('\u{1E23}', 'h'),
    ('\u{1E25}', 'h'),
    ('\u{1E27}', 'h'),
    ('\u{1E29}', 'h'),
    ('\u{1E2B}', 'h'),
    ('\u{1E2D}', 'i'),
    ('\u{1E2F}', 'i'),
    ('\u{1E31}', 'k'),
    ('\u{1E33}', 'k'),
    ('\u{1E35}', 'k'),
    ('\u{1E37}', 'l'),
    ('\u{1E39}', 'l'),
    ('\u{1E3B}', 'l'),
    ('\u{1E3D}', 'l'),
    ('\u{1E3F}', 'm'),
    ('\u{1E41}', 'm'),
    ('\u{1E43}', 'm'),
    ('\u{1E45}', 'n'),
    ('\u{1E47}', 'n'),
    ('\u{1E49}', 'n'),
    ('\u{1E4B}', 'n'),
    ('\u{1E4D}', 'o'),
    ('\u{1E4F}', 'o'),
    ('\u{1E51}', 'o'),
    ('\u{1E53}', 'o'),
    ('\u{1E55}', 'p'),
    ('\u{1E57}', 'p'),
    ('\u{1E59}', 'r'),
    ('\u{1E5B}', 'r'),
    ('\u{1E5D}', 'r'),
    ('\u{1E5F}', 'r'),
    ('\u{1E61}', 's'),
    ('\u{1E63}', 's'),
    ('\u{1E65}', 's'),
    ('\u{1E67}', 's'),
    ('\u{1E69}', 's'),
    ('\u{1E6B}', 't'),
    ('\u{1E6D}', 't'),
    ('\u{1E6F}', 't'),
    ('\u{1E71}', 't'),
    ('\u{1E73}', 'u'),
    ('\u{1E75}', 'u'),
    ('\u{1E77}', 'u'),
    ('\u{1E79}', 'u'),
    ('\u{1E7B}', 'u'),
    ('\u{1E7D}', 'v'),
    ('\u{1E7F}', 'v'),
    ('\u{1E81}', 'w'),
    ('\u{1E83}', 'w'),
    ('\u{1E85}', 'w'),
    ('\u{1E87}', 'w'),
    ('\u{1E89}', 'w'),
    ('\u{1E8B}', 'x'),
    ('\u{1E8D}', 'x'),
    ('\u{1E8F}', 'y'),
    ('\u{1E91}', 'z'),
    ('\u{1E93}', 'z'),
    ('\u{1E95}', 'z'),
    ('\u{1E96}', 'h'),
    ('\u{1E97}', 't'),
    ('\u{1E98}', 'w'),
    ('\u{1E99}', 'y'),
    ('\u{1EA1}', 'a'),
    ('\u{1EA3}', 'a'),
    ('\u{1EA5}', 'a'),
    ('\u{1EA7}', 'a'),
    ('\u{1EA9}', 'a'),
    ('\u{1EAB}', 'a'),
    ('\u{1EAD}', 'a'),
    ('\u{1EAF}', 'a'),
    ('\u{1EB1}', 'a'),
    ('\u{1EB3}', 'a'),
    ('\u{1EB5}', 'a'),
    ('\u{1EB7}', 'a'),
    ('\u{1EB9}', 'e'),
    ('\u{1EBB}', 'e'),
    ('\u{1EBD}', 'e'),
    ('\u{1EBF}', 'e'),
    ('\u{1EC1}', 'e'),
    ('\u{1EC3}', 'e'),
    ('\u{1EC5}', 'e'),
    ('\u{1EC7}', 'e'),
    ('\u{1EC9}', 'i'),
    ('\u{1ECB}', 'i'),
    ('\u{1ECD}', 'o'),
    ('\u{1ECF}', 'o'),
    ('\u{1ED1}', 'o'),
    ('\u{1ED3}', 'o'),
    ('\u{1ED5}', 'o'),
    ('\u{1ED7}', 'o'),
    ('\u{1ED9}', 'o'),
    ('\u{1EDB}', 'o'),
    ('\u{1EDD}', 'o'),
    ('\u{1EDF}', 'o'),
    ('\u{1EE1}', 'o'),
    ('\u{1EE3}', 'o'),
    ('\u{1EE5}', 'u'),
    ('\u{1EE7}', 'u'),
    ('\u{1EE9}', 'u'),
    ('\u{1EEB}', 'u'),
    ('\u{1EED}', 'u'),
    ('\u{1EEF}', 'u'),
    ('\u{1EF1}', 'u'),
    ('\u{1EF3}', 'y'),
    ('\u{1EF5}', 'y'),
    ('\u{1EF7}', 'y'),
    ('\u{1EF9}', 'y'),
    ('\u{2000}', ' '),
    ('\u{2001}', ' '),
    ('\u{2002}', ' '),
    ('\u{2003}', ' '),
    ('\u{2004}', ' '),
    ('\u{2005}', ' '),
    ('\u{2006}', ' '),
    ('\u{2007}', ' '),
    ('\u{2008}', ' '),
    ('\u{2009}', ' '),
    ('\u{200A}', ' '),
    ('\u{2028}', ' '),
    ('\u{2029}', ' '),
    ('\u{202F}', ' '),
    ('\u{205F}', ' '),
    ('\u{3000}', ' '),
];

fn folded(c: char) -> Option<char> {
    FOLDED
        .binary_search_by_key(&c, |(from, _)| *from)
        .ok()
        .map(|at| FOLDED[at].1)
}

/// ASCII white space as the first server's runtime counts it.
fn is_space(c: char) -> bool {
    c == ' ' || ('\t'..='\r').contains(&c)
}

/// The suggestion for `text`: lower case, no accents, spaces as dots, nothing
/// but ASCII letters, digits and single separators between them, at most 32
/// characters.
pub fn plain_login(text: Option<&str>) -> String {
    let lowered = stellarr_unicode::to_lower(text.unwrap_or(""));
    let mut login = String::new();
    let mut separator: Option<char> = None;
    for c in lowered.chars().map(|c| if c == '\u{0142}' { 'l' } else { c }) {
        let c = if c.is_ascii() { c } else { folded(c).unwrap_or(c) };
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if let Some(between) = separator
                && !login.is_empty()
            {
                login.push(between);
            }
            separator = None;
            login.push(c);
        } else if is_space(c) || matches!(c, '.' | '_' | '-') {
            // A run of separators counts as its first one.
            if separator.is_none() {
                separator = Some(if is_space(c) { '.' } else { c });
            }
        }
    }
    if login.len() <= 32 {
        login
    } else {
        login[..32].trim_end_matches(['.', '_', '-']).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_a_login() {
        assert_eq!(plain_login(Some("Zażółć Gęślą")), "zazolc.gesla");
        assert_eq!(plain_login(Some("  Ola   _ Łęcka ")), "ola.lecka");
        assert_eq!(plain_login(Some("André-Pierre")), "andre-pierre");
        assert_eq!(plain_login(Some("name@x.test")), "namex.test");
        assert_eq!(plain_login(None), "");
        assert_eq!(plain_login(Some("名前")), "");
    }

    #[test]
    fn it_is_cut_at_thirty_two_without_a_dangling_separator() {
        let text = format!("{}.{}", "a".repeat(31), "b".repeat(10));
        assert_eq!(plain_login(Some(&text)), "a".repeat(31));
        assert_eq!(plain_login(Some(&"a".repeat(40))).len(), 32);
    }

    #[test]
    fn white_space_that_is_not_ascii_separates_too() {
        assert_eq!(plain_login(Some("a\u{00A0}b\u{2003}c\u{3000}d")), "a.b.c.d");
    }

    #[test]
    fn the_table_is_sorted_so_that_the_search_is_sound() {
        assert!(FOLDED.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    /// Compares with the first server's own rules over the inputs of a file:
    /// `tests/providers-witness` answers the same lines (`check`). Run with
    /// `PROVIDERS_WITNESS_INPUT` and `PROVIDERS_WITNESS_EXPECTED` naming the
    /// two files.
    #[test]
    #[ignore = "needs the witness's answers: see tests/providers-witness/Program.cs"]
    fn it_agrees_with_the_first_server() {
        let read = |name: &str| std::fs::read_to_string(std::env::var(name).expect(name)).unwrap();
        let (input, expected) = (read("PROVIDERS_WITNESS_INPUT"), read("PROVIDERS_WITNESS_EXPECTED"));
        let mut differences = 0;
        for (line, answer) in input.lines().zip(expected.lines()) {
            let text: String = serde_json::from_str(line).unwrap();
            let answer: serde_json::Value = serde_json::from_str(answer).unwrap();
            let ours = (
                plain_login(Some(&text)),
                stellarr_oidc::options::safe_return_url(Some(&text)),
            );
            if answer["login"] != ours.0 || answer["safe"] != ours.1 {
                differences += 1;
                if differences < 15 {
                    eprintln!("{line}: first server {answer}, here {ours:?}");
                }
            }
        }
        assert_eq!(differences, 0);
    }
}

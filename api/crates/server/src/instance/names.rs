//! Names and addresses as they are stored for comparing: in capitals, letter by
//! letter, by the simple upper-case mapping of the first server's runtime.
//!
//! The mapping is `stellarr_unicode`'s, with one letter put right: the oracle
//! (the first server in its ICU image, which is the one that is run) maps
//! U+017F, the long s, to `S`, where the crate, made from a runtime without
//! ICU, leaves it. The dotless i, U+0131, is left alone by both. Whichever the
//! crate says, a name sent to the first server and to this one must collide
//! with the same names, so the long s follows the oracle until the crate does.

/// The upper case of one character, as stored.
pub fn to_upper_char(c: char) -> char {
    if c == '\u{017f}' {
        'S'
    } else {
        stellarr_unicode::to_upper_char(c)
    }
}

/// The text in capitals, as stored. It has as many characters as the text.
pub fn to_upper(text: &str) -> String {
    text.chars().map(to_upper_char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dotless_i_stays_and_the_long_s_becomes_s() {
        assert_eq!(to_upper("bıll"), "BıLL");
        assert_eq!(to_upper("İi"), "İI");
        assert_eq!(to_upper("ſtraße"), "STRAßE");
    }
}

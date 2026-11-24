//! Names and addresses as they are stored for comparing: in capitals, letter by
//! letter, by the simple upper-case mapping of the first server's runtime, which
//! is `stellarr_unicode`'s. Both servers work on copies of one database, so a
//! name sent to either must collide with the same names; nothing here may be
//! replaced with the standard library's case functions.

pub use stellarr_unicode::{to_upper, to_upper_char};

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

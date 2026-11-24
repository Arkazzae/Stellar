//! The versions a release and a client carry.

/// `major.minor.patch` and nothing else: no pre-release suffix, no fourth
/// part, and parts of one to nine digits, so that each fits in an integer.
/// Leading zeros are fine and white space around the whole is ignored.
/// Versions compare number by number, so `0.10.0` is after `0.9.0`.
pub fn parse_stable(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split('.');
    let mut number = || -> Option<u32> {
        let part = parts.next()?;
        if !(1..=9).contains(&part.len()) || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_three_number_stable_versions_are_accepted() {
        assert_eq!(parse_stable("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_stable(" 10.20.30 "), Some((10, 20, 30)));
        assert_eq!(parse_stable("\u{a0}1.2.3\n"), Some((1, 2, 3)));
        assert_eq!(parse_stable("000.001.002"), Some((0, 1, 2)));
        assert_eq!(parse_stable("999999999.0.0"), Some((999_999_999, 0, 0)));
        for refused in [
            "1.2",
            "1.2.3.4",
            "1.2.3-beta",
            "1.2.x",
            "1..3",
            "-1.2.3",
            "+1.2.3",
            "1.2.٣",
            "１.2.3",
            "1000000000.0.0",
            "1.2.3.",
            ".1.2.3",
            "1 .2.3",
            "",
            "   ",
        ] {
            assert_eq!(parse_stable(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn versions_compare_number_by_number() {
        assert!(parse_stable("0.10.0") > parse_stable("0.9.0"));
        assert!(parse_stable("1.0.0") > parse_stable("0.99.99"));
        assert!(parse_stable("0.1.9") < parse_stable("0.2.0"));
    }
}

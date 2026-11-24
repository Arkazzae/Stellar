//! Lookup keys for names and addresses use Stellarr's pinned simple uppercase
//! mapping. Changing that mapping would change which inputs match stored keys.

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

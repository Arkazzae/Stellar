//! UTF-8 checking for a text message that arrives in pieces. A piece may end
//! in the middle of a character, so the unfinished bytes wait for the next one.

#[derive(Debug, Default)]
pub struct Utf8Check {
    pending: [u8; 4],
    pending_len: usize,
}

impl Utf8Check {
    /// Takes the next piece of the message. False means the message is not
    /// valid UTF-8, whatever follows.
    pub fn feed(&mut self, mut piece: &[u8]) -> bool {
        // Finish the character that the previous piece left open, a byte at
        // a time: it is at most three bytes, and one may complete it.
        while self.pending_len > 0 {
            let Some((&byte, rest)) = piece.split_first() else {
                return true;
            };
            piece = rest;
            self.pending[self.pending_len] = byte;
            self.pending_len += 1;
            match std::str::from_utf8(&self.pending[..self.pending_len]) {
                Ok(_) => self.pending_len = 0,
                Err(error) if error.error_len().is_none() => {}
                Err(_) => return false,
            }
        }
        match std::str::from_utf8(piece) {
            Ok(_) => true,
            Err(error) if error.error_len().is_none() => {
                let tail = &piece[error.valid_up_to()..];
                self.pending[..tail.len()].copy_from_slice(tail);
                self.pending_len = tail.len();
                true
            }
            Err(_) => false,
        }
    }

    /// True when the message may end here: no character is left open.
    pub fn finish(&self) -> bool {
        self.pending_len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(pieces: &[&[u8]]) -> bool {
        let mut check = Utf8Check::default();
        pieces.iter().all(|piece| check.feed(piece)) && check.finish()
    }

    #[test]
    fn a_character_split_at_every_point_is_still_valid() {
        let text = "a\u{142}\u{20ac}\u{1f600}z".as_bytes();
        for first in 0..=text.len() {
            for second in first..=text.len() {
                assert!(
                    check(&[&text[..first], &text[first..second], &text[second..]]),
                    "{first} {second}"
                );
            }
        }
    }

    #[test]
    fn invalid_sequences_are_refused() {
        assert!(!check(&[&[0xff]]));
        assert!(!check(&[&[0xc0, 0x80]]), "overlong");
        assert!(!check(&[&[0xed, 0xa0, 0x80]]), "surrogate");
        assert!(!check(&[&[0xf4, 0x90, 0x80, 0x80]]), "beyond U+10FFFF");
        assert!(
            !check(&[&[0xe2, 0x82], &[0x41]]),
            "a lead byte cut off by a plain one"
        );
        assert!(!check(&[&[0x80]]), "a lone continuation byte");
    }

    #[test]
    fn a_message_ending_inside_a_character_is_refused() {
        assert!(!check(&[&[0xe2, 0x82]]));
        assert!(!check(&[&[0xe2], &[0x82]]));
    }

    #[test]
    fn empty_pieces_change_nothing() {
        assert!(check(&[b"", b"abc", b"", &[0xe2], b"", &[0x82, 0xac], b""]));
    }
}

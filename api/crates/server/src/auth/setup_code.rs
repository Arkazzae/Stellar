//! The code the owner account is created with. Until an instance has its first
//! account, whoever opens it first could become its owner; with this code it
//! takes someone who can read the container's log. A new code is drawn at every
//! start of an instance that has no account yet, kept in memory only and never
//! sent by any endpoint, and forgotten once the owner account exists.

use std::sync::Mutex;

use aws_lc_rs::rand;

use super::tokens::fixed_time_equals;

/// Without 0, 1, I and O, which are easy to mistake for one another when the
/// code is copied by hand.
const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const LENGTH: usize = 12;
const GROUP_LENGTH: usize = 4;

#[derive(Default)]
pub struct SetupCode {
    code: Mutex<Option<String>>,
}

impl SetupCode {
    /// Draws a new code in place of any earlier one, and returns it as a
    /// person reads it.
    pub fn issue(&self) -> String {
        let mut bytes = [0u8; LENGTH];
        rand::fill(&mut bytes).expect("the system random number generator failed");
        // 32 characters, so the low five bits of a random byte are unbiased.
        let code: String = bytes
            .iter()
            .map(|b| char::from(ALPHABET[usize::from(b & 31)]))
            .collect();
        let grouped = grouped(&code);
        *self.code.lock().unwrap_or_else(|p| p.into_inner()) = Some(code);
        grouped
    }

    /// Whether `candidate` is the current code. Case, spaces and dashes do not
    /// count, so a code typed as it is read is accepted. The comparison takes
    /// as long whichever character differs.
    pub fn matches(&self, candidate: Option<&str>) -> bool {
        let code = self.code.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let (Some(code), Some(candidate)) = (code, candidate) else {
            return false;
        };
        let normalized: String = candidate
            .chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .map(crate::instance::names::to_upper_char)
            .collect();
        fixed_time_equals(normalized.as_bytes(), code.as_bytes())
    }

    /// Drops the code once the instance has its owner.
    pub fn forget(&self) {
        *self.code.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// The current code as a person reads it. Tests read it here; everybody
    /// else reads it in the log.
    pub fn current(&self) -> Option<String> {
        self.code
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_deref()
            .map(grouped)
    }
}

fn grouped(code: &str) -> String {
    code.as_bytes()
        .chunks(GROUP_LENGTH)
        .map(|group| String::from_utf8_lossy(group).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_twelve_characters_in_three_groups_of_the_alphabet() {
        let code = SetupCode::default().issue();

        let groups: Vec<&str> = code.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            [4, 4, 4]
        );
        assert!(code.bytes().all(|b| b == b'-' || ALPHABET.contains(&b)));
    }

    #[test]
    fn the_code_is_accepted_whatever_its_case_spaces_and_dashes() {
        let setup = SetupCode::default();
        let code = setup.issue();
        let plain = code.replace('-', "");

        for typing in [
            code.clone(),
            code.to_lowercase(),
            plain.clone(),
            format!(" {} {} {} ", &plain[..4], &plain[4..8], &plain[8..]),
        ] {
            assert!(setup.matches(Some(&typing)), "{typing}");
        }
        assert!(!setup.matches(Some(&format!("{}X", plain))));
        assert!(!setup.matches(Some("")));
        assert!(!setup.matches(None));
    }

    #[test]
    fn a_new_code_replaces_the_old_and_a_forgotten_one_matches_nothing() {
        let setup = SetupCode::default();
        let first = setup.issue();
        let second = setup.issue();

        assert!(!setup.matches(Some(&first)));
        assert!(setup.matches(Some(&second)));
        assert_eq!(setup.current(), Some(second.clone()));
        setup.forget();
        assert!(!setup.matches(Some(&second)));
        assert_eq!(setup.current(), None);
    }
}

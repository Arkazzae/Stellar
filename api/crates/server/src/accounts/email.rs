//! Whether text is an e-mail address of the plain form `local@domain`.
//!
//! The local part is a run of atoms separated by single dots, or a quoted
//! string; the domain is a run of atoms separated by single dots. An atom is
//! letters, digits (of any script) and ``!#$%&'*+-/=?^_`{|}~``. Text that is
//! an address with a display name, such as `Ala <ala@example.test>`, is not
//! the address itself and is refused, as is anything over 320 characters.

const SYMBOLS: &str = "!#$%&'*+-/=?^_`{|}~";

fn is_atom_char(c: char) -> bool {
    c.is_alphanumeric() || SYMBOLS.contains(c)
}

fn is_dot_atom(text: &str) -> bool {
    !text.is_empty() && text.split('.').all(|atom| !atom.is_empty() && atom.chars().all(is_atom_char))
}

fn is_quoted_string(text: &str) -> bool {
    let Some(inner) = text.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return false;
    };
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if chars.next().is_none() {
                    return false;
                }
            }
            '"' | '\r' | '\n' => return false,
            _ => {}
        }
    }
    true
}

pub fn is_plain_address(email: &str) -> bool {
    if email.encode_utf16().count() > 320 {
        return false;
    }
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    (is_dot_atom(local) || is_quoted_string(local)) && is_dot_atom(domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_addresses_are_accepted() {
        for email in [
            "a@example.test",
            "first.last+tag@example.test",
            "o'brien@example.test",
            "zażółć@przykład.test",
            "user@localhost",
            "\"two words\"@example.test",
            "x@y",
        ] {
            assert!(is_plain_address(email), "{email}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for email in [
            "",
            "not-an-email",
            "Ala <ala@example.test>",
            "a@",
            "@example.test",
            "a b@example.test",
            "a..b@example.test",
            ".a@example.test",
            "a.@example.test",
            "a@example..test",
            "a@example.test.",
            "a@@example.test",
            "\"unclosed@example.test",
            &format!("{}@example.test", "a".repeat(320)),
        ] {
            assert!(!is_plain_address(email), "{email}");
        }
    }
}

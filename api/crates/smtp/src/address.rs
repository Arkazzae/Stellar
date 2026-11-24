//! Mailboxes: an address with an optional display name, and how both are
//! written into a header.

use crate::encode::{check_header_text, encoded_words};
use crate::error::Error;

/// The longest address the protocol carries (RFC 5321).
const MAX_ADDRESS: usize = 254;
const MAX_LOCAL: usize = 64;
const MAX_NAME: usize = 200;

/// An address, with the name a mail client shows for it.
///
/// The domain is kept in its ASCII form: a domain with non-ASCII letters is
/// turned into punycode when the mailbox is made (the caller supplies it in
/// normalised form). A non-ASCII local part is allowed; it can only travel
/// to a server that offers SMTPUTF8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    name: Option<String>,
    address: String,
}

impl Mailbox {
    pub fn new(name: Option<&str>, address: &str) -> Result<Mailbox, Error> {
        let name = match name.map(str::trim) {
            None | Some("") => None,
            Some(name) => {
                check_header_text("a display name", name)?;
                if name.chars().count() > MAX_NAME {
                    return Err(Error::invalid(format!(
                        "a display name is longer than {MAX_NAME} characters"
                    )));
                }
                Some(name.to_owned())
            }
        };
        Ok(Mailbox {
            name,
            address: normalise_address(address)?,
        })
    }

    /// Reads `Name <user@host>`, `"Name, Co" <user@host>`, `<user@host>` or
    /// `user@host`. The name is taken as plain text: it is the operator's
    /// setting, not something to be interpreted.
    pub fn parse(text: &str) -> Result<Mailbox, Error> {
        let text = text.trim();
        if let Some(inner) = text.strip_suffix('>') {
            let open = inner.rfind('<').ok_or_else(|| {
                Error::invalid(format!("the address {text:?} has a > without a <"))
            })?;
            let (name, address) = (inner[..open].trim(), &inner[open + 1..]);
            return Mailbox::new(Some(&unquote(name)), address);
        }
        Mailbox::new(None, text)
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `user@host`, with the host in ASCII.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Whether the address needs SMTPUTF8 to be sent.
    pub(crate) fn needs_utf8(&self) -> bool {
        !self.address.is_ascii()
    }

    /// The mailbox as it stands in an address header.
    pub(crate) fn header_value(&self) -> String {
        let Some(name) = &self.name else {
            return self.address.clone();
        };
        let phrase = if name.is_ascii() {
            if is_plain_phrase(name) {
                name.clone()
            } else {
                format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
            }
        } else {
            encoded_words(name)
        };
        format!("{phrase} <{}>", self.address)
    }
}

fn unquote(name: &str) -> String {
    let Some(inner) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) else {
        return name.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

/// A phrase that needs no quotes: words of the atom characters of RFC 5322.
fn is_plain_phrase(name: &str) -> bool {
    !name.contains("=?")
        && name
            .chars()
            .all(|c| c == ' ' || c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c))
}

fn is_atom_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || "!#$%&'*+-/=?^_`{|}~".contains(c)
        || (!c.is_ascii() && !c.is_control() && !c.is_whitespace())
}

fn normalise_address(address: &str) -> Result<String, Error> {
    let address = address.trim();
    let bad = |why: &str| Error::invalid(format!("the address {address:?} {why}"));
    let (local, domain) = address.rsplit_once('@').ok_or_else(|| bad("has no @"))?;
    if local.is_empty() || local.len() > MAX_LOCAL {
        return Err(bad("has an empty or too long part before the @"));
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return Err(bad("has a misplaced dot before the @"));
    }
    if !local.chars().all(|c| c == '.' || is_atom_char(c)) {
        return Err(bad(
            "has a character before the @ that this client does not send (quoted local parts are not supported)",
        ));
    }
    let domain =
        ascii_domain(domain).ok_or_else(|| bad("has a domain that is not a valid host name"))?;
    let full = format!("{local}@{domain}");
    if full.len() > MAX_ADDRESS {
        return Err(bad("is longer than 254 bytes"));
    }
    Ok(full)
}

fn ascii_domain(domain: &str) -> Option<String> {
    if domain.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (index, label) in domain.split('.').enumerate() {
        let label = label.to_lowercase();
        let label = if label.is_ascii() {
            label
        } else {
            format!("xn--{}", punycode(&label)?)
        };
        let bytes = label.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 63
            || bytes[0] == b'-'
            || bytes[bytes.len() - 1] == b'-'
            || !bytes
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return None;
        }
        if index > 0 {
            out.push('.');
        }
        out.push_str(&label);
    }
    Some(out)
}

/// Punycode of RFC 3492, encoding only.
fn punycode(label: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;

    fn adapt(mut delta: u32, points: u32, first: bool) -> u32 {
        delta = if first { delta / DAMP } else { delta / 2 };
        delta += delta / points;
        let mut k = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k += BASE;
        }
        k + (BASE - TMIN + 1) * delta / (delta + SKEW)
    }
    fn digit(d: u32) -> char {
        char::from(if d < 26 {
            b'a' + d as u8
        } else {
            b'0' + (d - 26) as u8
        })
    }

    let input: Vec<u32> = label.chars().map(u32::from).collect();
    let mut out: String = label.chars().filter(char::is_ascii).collect();
    let basic = out.len() as u32;
    let mut handled = basic;
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    while (handled as usize) < input.len() {
        let next = input.iter().copied().filter(|&c| c >= n).min()?;
        delta = delta.checked_add((next - n).checked_mul(handled + 1)?)?;
        n = next;
        for &c in &input {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        TMIN
                    } else if k >= bias + TMAX {
                        TMAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_address_and_a_named_one_are_read() {
        let bare = Mailbox::parse("jo@example.test").unwrap();
        assert_eq!((bare.name(), bare.address()), (None, "jo@example.test"));
        let named = Mailbox::parse("Stellarr <noreply@chat.test>").unwrap();
        assert_eq!(
            (named.name(), named.address()),
            (Some("Stellarr"), "noreply@chat.test")
        );
        assert_eq!(Mailbox::parse("<a@b.test>").unwrap().name(), None);
    }

    #[test]
    fn a_name_with_commas_and_quotes_is_kept_as_text() {
        let m = Mailbox::parse("Orbit, Co \"Ltd\" <noreply@chat.test>").unwrap();
        assert_eq!(m.name(), Some("Orbit, Co \"Ltd\""));
        assert_eq!(
            m.header_value(),
            "\"Orbit, Co \\\"Ltd\\\"\" <noreply@chat.test>"
        );
        let quoted = Mailbox::parse("\"A \\\"B\\\"\" <a@b.test>").unwrap();
        assert_eq!(quoted.name(), Some("A \"B\""));
    }

    #[test]
    fn a_polish_name_becomes_an_encoded_word() {
        let m = Mailbox::parse("Zażółć <a@b.test>").unwrap();
        let value = m.header_value();
        assert!(
            value.is_ascii() && value.starts_with("=?utf-8?b?") && value.ends_with(" <a@b.test>")
        );
    }

    #[test]
    fn a_dotted_name_is_quoted() {
        assert_eq!(
            Mailbox::parse("Jan K. Nowak <a@b.test>")
                .unwrap()
                .header_value(),
            "\"Jan K. Nowak\" <a@b.test>"
        );
        assert_eq!(
            Mailbox::parse("Jan Nowak <a@b.test>")
                .unwrap()
                .header_value(),
            "Jan Nowak <a@b.test>"
        );
    }

    #[test]
    fn what_is_not_an_address_is_refused() {
        for bad in [
            "",
            "plain",
            "a@",
            "@b.test",
            "a b@c.test",
            "a@b c.test",
            "a@b..test",
            "a@-b.test",
            "a..b@c.test",
            ".a@c.test",
            "a@c.test>",
            "a<b@c.test",
            "\"q q\"@c.test",
            "a@[1.2.3.4]",
            "a,b@c.test",
            "a@b.test\r\nBcc: x",
            "a;b@c.test",
        ] {
            assert!(Mailbox::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Mailbox::parse(&format!("{}@c.test", "a".repeat(65))).is_err());
        assert!(Mailbox::parse(&format!("a@{}.test", "b".repeat(64))).is_err());
        assert!(Mailbox::new(Some("x\ny"), "a@b.test").is_err());
        assert!(Mailbox::new(Some(&"x".repeat(201)), "a@b.test").is_err());
    }

    #[test]
    fn an_international_domain_becomes_punycode() {
        // Checked against Python's idna codec.
        assert_eq!(
            Mailbox::new(None, "a@bücher.example").unwrap().address(),
            "a@xn--bcher-kva.example"
        );
        assert_eq!(
            Mailbox::new(None, "a@MÜNCHEN.de").unwrap().address(),
            "a@xn--mnchen-3ya.de"
        );
        assert_eq!(
            Mailbox::new(None, "a@żółć.pl").unwrap().address(),
            "a@xn--kda4b0koi.pl"
        );
    }

    #[test]
    fn a_non_ascii_local_part_needs_smtputf8() {
        let m = Mailbox::new(None, "zażółć@example.test").unwrap();
        assert!(m.needs_utf8());
        assert!(!Mailbox::new(None, "a@example.test").unwrap().needs_utf8());
    }
}

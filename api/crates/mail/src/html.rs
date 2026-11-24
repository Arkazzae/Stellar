//! Text into HTML, and the parts of a web address the messages show.

/// Escapes text for HTML, in a text node or a quoted attribute. Besides the
/// five characters that mean something, it writes the Latin-1 letters from
/// U+00A0 up and everything outside the Basic Multilingual Plane as numeric
/// references, which is what mail clients with a poor idea of the message's
/// character set expect.
pub(crate) fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '"' => out.push_str("&quot;"),
            c if ('\u{a0}'..='\u{ff}').contains(&c) || u32::from(c) > 0xffff => {
                out.push_str(&format!("&#{};", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out
}

/// The scheme, user information, host and port of an absolute address with
/// an authority (`https://host:port/...`).
struct Address {
    scheme: String,
    user_info: Option<String>,
    host: String,
    port: Option<u16>,
}

impl Address {
    fn parse(text: &str) -> Option<Address> {
        let text = text.trim();
        let (scheme, rest) = text.split_once("://")?;
        let mut scheme_chars = scheme.chars();
        let valid_scheme = scheme_chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme_chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if !valid_scheme {
            return None;
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let (user_info, host_port) = match authority.rfind('@') {
            Some(at) => (Some(authority[..at].to_owned()), &authority[at + 1..]),
            None => (None, authority),
        };
        let (host, port) = split_port(host_port)?;
        if host.is_empty() {
            return None;
        }
        let scheme = scheme.to_ascii_lowercase();
        // A port that is the scheme's own is not part of the address.
        let port = port.filter(|p| default_port(&scheme) != Some(*p));
        Some(Address {
            scheme,
            user_info,
            host: host.to_lowercase(),
            port,
        })
    }

    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        }
    }
}

fn split_port(host_port: &str) -> Option<(&str, Option<u16>)> {
    if host_port.starts_with('[') {
        let close = host_port.find(']')?;
        let host = &host_port[..=close];
        return match &host_port[close + 1..] {
            "" => Some((host, None)),
            rest => Some((host, Some(rest.strip_prefix(':')?.parse().ok()?))),
        };
    }
    match host_port.rsplit_once(':') {
        Some((host, "")) => Some((host, None)),
        Some((host, port)) => Some((host, Some(port.parse().ok()?))),
        None => Some((host_port, None)),
    }
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

/// `https://host:port` of a link into the instance, or nothing when the text
/// is not an absolute address.
pub(crate) fn origin_of(url: &str) -> String {
    match Address::parse(url) {
        Some(address) => {
            let user = address
                .user_info
                .as_ref()
                .map_or(String::new(), |u| format!("{u}@"));
            format!("{}://{user}{}", address.scheme, address.authority())
        }
        None => String::new(),
    }
}

/// The host and port of an address, or the text itself when it is not one.
pub(crate) fn authority_of(url: &str) -> String {
    match Address::parse(url) {
        Some(address) => address.authority(),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_characters_and_the_wide_ones_are_escaped() {
        assert_eq!(
            encode("<b>\"Jo\" & 'Co'</b>"),
            "&lt;b&gt;&quot;Jo&quot; &amp; &#39;Co&#39;&lt;/b&gt;"
        );
        // U+00F3 is in the Latin-1 range and is written as a reference; U+017C, U+0142 and U+0107 are not.
        assert_eq!(
            encode("za\u{17c}\u{f3}\u{142}\u{107}"),
            "za\u{17c}&#243;\u{142}\u{107}"
        );
        assert_eq!(encode("a\u{a0}b"), "a&#160;b");
        assert_eq!(encode("\u{1F600}"), "&#128512;");
        assert_eq!(encode("plain ąę"), "plain ąę");
    }

    #[test]
    fn the_origin_keeps_the_scheme_and_a_port_that_is_not_the_default() {
        assert_eq!(
            origin_of("https://chat.test/reset?token=a"),
            "https://chat.test"
        );
        assert_eq!(origin_of("https://Chat.Test:443/x"), "https://chat.test");
        assert_eq!(origin_of("http://127.0.0.1:5196"), "http://127.0.0.1:5196");
        assert_eq!(origin_of("http://[::1]:8080/a"), "http://[::1]:8080");
        assert_eq!(origin_of("not an address"), "");
        assert_eq!(origin_of(""), "");
        assert_eq!(authority_of("https://chat.test:8443/"), "chat.test:8443");
        assert_eq!(authority_of("chat.test"), "chat.test");
    }
}

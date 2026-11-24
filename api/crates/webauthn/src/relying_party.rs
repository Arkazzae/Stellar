//! The relying party: the site passkeys are bound to.

use crate::error::Error;

const NAME: &str = "Stellarr";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelyingParty {
    id: String,
    name: String,
    origins: Vec<String>,
}

impl RelyingParty {
    /// A relying party with the given id, display name and allowed origins.
    /// An origin is written the way a browser writes it: scheme, host and a
    /// port only when it is not the default one.
    pub fn new(id: &str, name: &str, origins: &[&str]) -> Result<RelyingParty, Error> {
        if id.is_empty() || name.is_empty() {
            return Err(Error::InvalidRelyingParty("id and name must not be empty"));
        }
        if origins.is_empty() || origins.iter().any(|origin| origin.is_empty()) {
            return Err(Error::InvalidRelyingParty("at least one origin is needed"));
        }
        Ok(RelyingParty {
            id: id.to_owned(),
            name: name.to_owned(),
            origins: origins.iter().map(|origin| (*origin).to_owned()).collect(),
        })
    }

    /// The relying party of a public address, or `None` when passkeys cannot
    /// work there: browsers accept them only for a named host reached over
    /// HTTPS (or plain HTTP on `localhost`), never for an IP address. The id
    /// is the host and the one allowed origin is the address without its path.
    pub fn from_public_url(url: &str) -> Option<RelyingParty> {
        let (scheme, rest) = url.trim().split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "https" => 443,
            "http" => 80,
            _ => return None,
        };
        let authority = rest.split(['/', '?', '#']).next()?;
        if authority.contains(['@', '[', ']']) {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                (
                    host,
                    Some(port.parse::<u16>().ok().filter(|port| *port != 0)?),
                )
            }
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        if !is_dns_name(&host) {
            return None;
        }
        if scheme == "http" && host != "localhost" {
            return None;
        }

        let origin = match port {
            Some(port) if port != default_port => format!("{scheme}://{host}:{port}"),
            _ => format!("{scheme}://{host}"),
        };
        Some(RelyingParty {
            id: host,
            name: NAME.to_owned(),
            origins: vec![origin],
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn origins(&self) -> &[String] {
        &self.origins
    }
}

/// A host name made of letters, digits and hyphens, not an address. A name
/// whose last label is all digits reads as an IPv4 address in some forms
/// ("1", "192.0.2.10"), so it is refused.
fn is_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let labels_ok = labels.iter().all(|label| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let last = labels.last().copied().unwrap_or("");
    labels_ok && !last.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin_of(url: &str) -> Option<(String, String)> {
        RelyingParty::from_public_url(url).map(|rp| (rp.id().to_owned(), rp.origins()[0].clone()))
    }

    #[test]
    fn a_https_host_is_the_id_and_the_origin() {
        assert_eq!(
            origin_of("https://chat.example.com"),
            Some(("chat.example.com".into(), "https://chat.example.com".into()))
        );
        assert_eq!(
            origin_of(" https://Chat.Example.com/app?x=1#y "),
            Some(("chat.example.com".into(), "https://chat.example.com".into()))
        );
    }

    #[test]
    fn a_non_default_port_stays_in_the_origin_and_a_default_one_goes() {
        assert_eq!(
            origin_of("https://chat.test:8443/"),
            Some(("chat.test".into(), "https://chat.test:8443".into()))
        );
        assert_eq!(
            origin_of("https://chat.test:443"),
            Some(("chat.test".into(), "https://chat.test".into()))
        );
        assert_eq!(
            origin_of("http://localhost:5173"),
            Some(("localhost".into(), "http://localhost:5173".into()))
        );
        assert_eq!(
            origin_of("http://localhost:80"),
            Some(("localhost".into(), "http://localhost".into()))
        );
    }

    #[test]
    fn addresses_plain_http_and_other_schemes_are_refused() {
        for url in [
            "http://192.0.2.10:8080",
            "https://192.0.2.10",
            "https://[::1]",
            "https://[2001:db8::1]:8443",
            "https://1",
            "https://0x7f.1",
            "http://chat.test",
            "http://127.0.0.1",
            "ftp://chat.test",
            "chat.test",
            "",
            "https://",
            "https://user@chat.test",
            "https://chat.test:0",
            "https://chat.test:99999",
            "https://chat.test:",
            "https://-bad.test",
            "https://bad..test",
            "https://bad_name.test",
            "https://b\u{e4}d.test",
        ] {
            assert_eq!(origin_of(url), None, "{url}");
        }
    }

    #[test]
    fn new_refuses_empty_parts() {
        assert!(RelyingParty::new("", "n", &["https://a"]).is_err());
        assert!(RelyingParty::new("a", "", &["https://a"]).is_err());
        assert!(RelyingParty::new("a", "n", &[]).is_err());
        assert!(RelyingParty::new("a", "n", &[""]).is_err());
    }
}

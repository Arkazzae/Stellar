//! The reverse proxies the operator runs in front of the container, listed in
//! `Operations:TrustedProxies` (`STELLARR_TRUSTED_PROXIES`): those whose word
//! the API takes for the address of the person behind a request. How the list
//! is used is in `http::forwarded`.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::SettingError;

pub const SETTING_NAME: &str = "Operations:TrustedProxies";
pub const VARIABLE_NAME: &str = "STELLARR_TRUSTED_PROXIES";

/// An address or a network of addresses; a single address is a network of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    address: IpAddr,
    prefix: u8,
}

impl Network {
    pub fn contains(&self, candidate: IpAddr) -> bool {
        match (self.address, normalize(candidate)) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => {
                masked_v4(candidate, self.prefix) == network
            }
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                masked_v6(candidate, self.prefix) == network
            }
            _ => false,
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

/// An IPv4 address that arrived as an IPv6 one (`::ffff:a.b.c.d`) is the IPv4
/// address.
pub fn normalize(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        v4 => v4,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedProxies {
    networks: Vec<Network>,
}

impl TrustedProxies {
    pub fn networks(&self) -> &[Network] {
        &self.networks
    }

    pub fn is_empty(&self) -> bool {
        self.networks.is_empty()
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        self.networks
            .iter()
            .any(|network| network.contains(address))
    }

    /// Parses addresses and CIDR networks, IPv4 and IPv6, separated by commas
    /// or any whitespace. Blank means none. An entry the API cannot use is
    /// named in the error, so that a typo stops the start instead of leaving
    /// a proxy out.
    pub fn parse(value: &str) -> Result<TrustedProxies, SettingError> {
        let mut networks = Vec::new();
        for entry in value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|entry| !entry.is_empty())
        {
            networks.push(parse_entry(entry)?);
        }
        Ok(TrustedProxies { networks })
    }
}

fn invalid(entry: &str, problem: &str) -> SettingError {
    SettingError::new(format!(
        "{SETTING_NAME} ({VARIABLE_NAME}) lists \"{entry}\", which {problem}."
    ))
}

fn parse_entry(entry: &str) -> Result<Network, SettingError> {
    let (address_text, prefix_text) = match entry.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (entry, None),
    };
    let Some(address) = parse_address(address_text) else {
        return Err(invalid(
            entry,
            "is not an IP address or a network such as 172.18.0.0/16",
        ));
    };
    let width: u8 = if address.is_ipv4() { 32 } else { 128 };
    let mut prefix = width;
    if let Some(digits) = prefix_text {
        let parsed = (1..=3)
            .contains(&digits.len())
            .then(|| digits.parse::<u16>().ok())
            .flatten()
            .filter(|_| digits.bytes().all(|b| b.is_ascii_digit()))
            .filter(|prefix| *prefix <= u16::from(width));
        let Some(parsed) = parsed else {
            return Err(invalid(
                entry,
                &format!("has a prefix length that is not a number from 1 to {width}"),
            ));
        };
        if parsed == 0 {
            return Err(invalid(
                entry,
                "covers every address, so anyone could choose the address the API records",
            ));
        }
        prefix = parsed as u8;
    }
    let network = match address {
        IpAddr::V4(v4) => IpAddr::V4(masked_v4(v4, prefix)),
        IpAddr::V6(v6) => IpAddr::V6(masked_v6(v6, prefix)),
    };
    if network != address {
        return Err(invalid(
            entry,
            &format!("has bits set past its prefix length; the network is {network}/{prefix}"),
        ));
    }
    Ok(Network { address, prefix })
}

/// Only the plain forms: four decimal numbers, or IPv6 without brackets, port
/// or zone. The shorthand some parsers accept, `10.1` for `10.0.0.1`, would
/// turn a typo into some other address.
fn parse_address(text: &str) -> Option<IpAddr> {
    if text.contains(':') {
        if !text
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        {
            return None;
        }
        return text.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
    }
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (octet, part) in octets.iter_mut().zip(&parts) {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *octet = part.parse().ok()?;
    }
    Some(IpAddr::V4(Ipv4Addr::from(octets)))
}

fn masked_v4(address: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    Ipv4Addr::from(u32::from(address) & mask)
}

fn masked_v6(address: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    Ipv6Addr::from(u128::from(address) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(value: &str) -> Vec<String> {
        TrustedProxies::parse(value)
            .unwrap()
            .networks()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn blank_lists_no_proxy() {
        for value in ["", "   ", " ,\n, \t"] {
            assert!(
                TrustedProxies::parse(value).unwrap().is_empty(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn reads_an_address_or_a_network() {
        for (value, network) in [
            ("172.18.0.1", "172.18.0.1/32"),
            ("172.18.0.0/16", "172.18.0.0/16"),
            ("10.0.0.0/8", "10.0.0.0/8"),
            ("172.018.000.001", "172.18.0.1/32"),
            ("2001:db8::7", "2001:db8::7/128"),
            ("2001:DB8:1::/48", "2001:db8:1::/48"),
            ("fd00::/8", "fd00::/8"),
        ] {
            assert_eq!(rendered(value), [network], "{value}");
        }
    }

    #[test]
    fn reads_several_entries_separated_by_commas_and_whitespace() {
        assert_eq!(
            rendered(" 172.18.0.1 ,10.0.0.0/8,\n\t2001:db8::/32  fd00::1, "),
            [
                "172.18.0.1/32",
                "10.0.0.0/8",
                "2001:db8::/32",
                "fd00::1/128"
            ]
        );
    }

    #[test]
    fn an_entry_the_api_cannot_use_is_named_rather_than_dropped() {
        for entry in [
            "proxy.example",
            "localhost",
            "10",
            "10.1",
            "10.0.0",
            "10.0.0.256",
            "10.0.0.1.5",
            "0x0a.0.0.1",
            "10.0.0.1:8080",
            "[2001:db8::1]",
            "fe80::1%eth0",
            "2001:db8::g",
            "10.0.0.0/",
            "10.0.0.0/33",
            "10.0.0.0/-8",
            "10.0.0.0/8/8",
            "2001:db8::/129",
            "10.0.0.1/24",
            "2001:db8::1/64",
            "0.0.0.0/0",
            "::/0",
        ] {
            let error = TrustedProxies::parse(&format!("172.18.0.1, {entry}, 10.0.0.0/8"))
                .unwrap_err()
                .to_string();

            assert!(
                error.starts_with(&format!(
                    "{SETTING_NAME} ({VARIABLE_NAME}) lists \"{entry}\", which "
                )),
                "{entry}: {error}"
            );
        }
    }

    #[test]
    fn the_message_says_what_is_wrong_with_the_entry() {
        for (entry, explanation) in [
            ("10.0.0.1/24", "the network is 10.0.0.0/24"),
            ("2001:db8::1/64", "the network is 2001:db8::/64"),
            ("0.0.0.0/0", "anyone could choose the address"),
            ("10.0.0.0/33", "a number from 1 to 32"),
            ("2001:db8::/129", "a number from 1 to 128"),
        ] {
            let error = TrustedProxies::parse(entry).unwrap_err().to_string();

            assert!(error.contains(explanation), "{entry}: {error}");
        }
    }

    #[test]
    fn a_network_contains_its_addresses_in_either_notation() {
        let proxies = TrustedProxies::parse("172.18.0.0/16, 2001:db8:1::/64").unwrap();

        assert!(proxies.contains("172.18.4.5".parse().unwrap()));
        assert!(proxies.contains("::ffff:172.18.4.5".parse().unwrap()));
        assert!(proxies.contains("2001:db8:1::99".parse().unwrap()));
        assert!(!proxies.contains("172.19.0.1".parse().unwrap()));
        assert!(!proxies.contains("2001:db8:2::1".parse().unwrap()));
    }
}

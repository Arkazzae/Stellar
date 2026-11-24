//! Whether an address is out on the internet or inside the house.
//!
//! What is not on a list of reserved blocks counts as public: the blocks
//! reserved for private, loopback, link-local, shared, documentation,
//! benchmarking, multicast and future use in IPv4, and their IPv6
//! counterparts. An IPv4-mapped IPv6 address is judged by the IPv4 address
//! inside it.

use std::net::{Ipv4Addr, Ipv6Addr};

const RESERVED_V4: [(&str, u8); 14] = [
    ("0.0.0.0", 8),
    ("10.0.0.0", 8),
    ("100.64.0.0", 10),
    ("127.0.0.0", 8),
    ("169.254.0.0", 16),
    ("172.16.0.0", 12),
    ("192.0.0.0", 24),
    ("192.0.2.0", 24),
    ("192.88.99.0", 24),
    ("192.168.0.0", 16),
    ("198.18.0.0", 15),
    ("198.51.100.0", 24),
    ("203.0.113.0", 24),
    ("224.0.0.0", 3),
];

// The `::/96` block covers the unspecified and loopback addresses and the old
// IPv4-compatible form; `64:ff9b::/96` is NAT64. Both embed an IPv4 address
// that would otherwise slip past the check, so both are refused. The rest are
// the discard-only block, documentation, 6to4 (it also carries an IPv4
// address that is not checked here), unique-local, link-local, site-local and
// multicast.
const RESERVED_V6: [(&str, u8); 14] = [
    ("::", 96),
    // IPv4-translated (RFC 2765); it embeds an IPv4 address.
    ("::ffff:0:0:0", 96),
    ("64:ff9b::", 96),
    // Local-use NAT64 (RFC 8215); it embeds an IPv4 address.
    ("64:ff9b:1::", 48),
    // The IETF protocol assignments block: Teredo (which embeds an IPv4
    // address), benchmarking, ORCHID and others, none of them a public host.
    ("2001::", 23),
    // Documentation, the second block (RFC 9637).
    ("3fff::", 20),
    // Segment routing SIDs (RFC 9602).
    ("5f00::", 16),
    ("100::", 16),
    ("2001:db8::", 32),
    ("2002::", 16),
    ("fc00::", 7),
    ("fe80::", 10),
    ("fec0::", 10),
    ("ff00::", 8),
];

fn v4_in(address: u32, network: &str, prefix: u8) -> bool {
    let net = u32::from(network.parse::<Ipv4Addr>().expect("table entry is valid"));
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    address & mask == net & mask
}

fn v6_in(address: u128, network: &str, prefix: u8) -> bool {
    let net = u128::from(network.parse::<Ipv6Addr>().expect("table entry is valid"));
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    address & mask == net & mask
}

/// Whether an IPv4 address is on the public internet.
pub fn is_public_v4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    !RESERVED_V4
        .iter()
        .any(|&(net, prefix)| v4_in(value, net, prefix))
}

/// Whether an IPv6 address is on the public internet.
pub fn is_public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let value = u128::from(address);
    !RESERVED_V6
        .iter()
        .any(|&(net, prefix)| v6_in(value, net, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(text: &str) -> bool {
        is_public_v4(text.parse().unwrap())
    }

    fn v6(text: &str) -> bool {
        is_public_v6(text.parse().unwrap())
    }

    #[test]
    fn reserved_v4_blocks_are_refused_at_their_edges() {
        for text in [
            "0.0.0.0",
            "10.255.255.255",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!v4(text), "{text}");
        }
    }

    #[test]
    fn neighbours_of_reserved_v4_blocks_are_public() {
        for text in [
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "223.255.255.255",
            "1.1.1.1",
        ] {
            assert!(v4(text), "{text}");
        }
    }

    #[test]
    fn reserved_v6_blocks_are_refused() {
        for text in [
            "::",
            "::1",
            "100::1",
            "2001:db8::1",
            "2002::1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "::7f00:1",
            "64:ff9b::7f00:1",
        ] {
            assert!(!v6(text), "{text}");
        }
    }

    #[test]
    fn the_newer_reserved_v6_blocks_are_refused_at_their_edges() {
        for text in [
            "::ffff:0:0:0",
            "::ffff:0:7f00:1",
            "::ffff:0:ffff:ffff",
            "64:ff9b:1::",
            "64:ff9b:1::7f00:1",
            "64:ff9b:1:ffff:ffff:ffff:ffff:ffff",
            "2001::",
            "2001::7f00:1",
            "2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff",
            "3fff::",
            "3fff::1",
            "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff",
            "5f00::",
            "5f00::1",
            "5f00:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        ] {
            assert!(!v6(text), "{text}");
        }
        for text in [
            "::ffff:1:0:0",
            "64:ff9b:2::1",
            "64:ff9c::1",
            "2000:ffff::1",
            "2001:200::1",
            "2001:4860:4860::8888",
            "3ffe::1",
            "3fff:1000::1",
            "5eff:ffff::1",
            "5f01::1",
        ] {
            assert!(v6(text), "{text}");
        }
    }

    #[test]
    fn public_v6_is_allowed() {
        assert!(v6("2606:4700:4700::1111"));
        assert!(v6("2001:4860:4860::8888"));
    }

    #[test]
    fn mapped_v6_is_judged_by_the_v4_inside() {
        assert!(!v6("::ffff:127.0.0.1"));
        assert!(!v6("::ffff:10.0.0.1"));
        assert!(v6("::ffff:8.8.8.8"));
    }
}

//! Whether an address is out on the internet or inside the house.
//!
//! There is one table of what is not public, and it is the outbound client's:
//! the same judgement that decides where a request may connect decides what an
//! endpoint may name, so the two cannot drift apart. An IPv6 address that
//! wraps an IPv4 one (mapped, translated, the NAT64 prefix) is judged by the
//! address inside it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether an IPv4 address is on the public internet.
pub fn is_public_v4(address: Ipv4Addr) -> bool {
    stellarr_outbound::is_public(IpAddr::V4(address))
}

/// Whether an IPv6 address is on the public internet.
pub fn is_public_v6(address: Ipv6Addr) -> bool {
    stellarr_outbound::is_public(IpAddr::V6(address))
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

    #[test]
    fn a_wrapped_v4_address_is_judged_by_the_address_inside() {
        assert!(v6("64:ff9b::808:808"));
        assert!(v6("::ffff:0:808:808"));
        assert!(!v6("64:ff9b::7f00:1"));
        assert!(!v6("::ffff:0:7f00:1"));
    }
}

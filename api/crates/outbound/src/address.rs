//! Which addresses a request may connect to.
//!
//! A name that somebody else chose, a link in a message or a redirect an
//! answer asks for, must not be able to make this server open connections to
//! the machines around it. The check is therefore about the address the
//! socket is opened to, never about the text of the URL.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Where a request is allowed to connect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Any address. For hosts the operator configured.
    Anywhere,
    /// Loopback only. For a service that runs next to this one.
    LoopbackOnly,
    /// Only addresses out on the internet: nothing private, loopback,
    /// link-local, shared, reserved, documentation or multicast. For hosts
    /// that somebody other than the operator chose.
    PublicOnly,
}

impl AddressPolicy {
    /// Whether a connection to `address` is allowed.
    pub fn allows(self, address: IpAddr) -> bool {
        match self {
            AddressPolicy::Anywhere => true,
            AddressPolicy::LoopbackOnly => is_loopback(address),
            AddressPolicy::PublicOnly => is_public(address),
        }
    }
}

/// An IPv4 block: the first address and the length of the prefix.
type BlockV4 = (u32, u32);

const fn v4(a: u8, b: u8, c: u8, d: u8, prefix: u32) -> BlockV4 {
    (u32::from_be_bytes([a, b, c, d]), prefix)
}

/// Private, loopback, link-local, shared, documentation, benchmarking,
/// multicast and future use. 224.0.0.0/3 also takes the broadcast address.
const RESERVED_V4: &[BlockV4] = &[
    v4(0, 0, 0, 0, 8),
    v4(10, 0, 0, 0, 8),
    v4(100, 64, 0, 0, 10),
    v4(127, 0, 0, 0, 8),
    v4(169, 254, 0, 0, 16),
    v4(172, 16, 0, 0, 12),
    v4(192, 0, 0, 0, 24),
    v4(192, 0, 2, 0, 24),
    v4(192, 88, 99, 0, 24),
    v4(192, 168, 0, 0, 16),
    v4(198, 18, 0, 0, 15),
    v4(198, 51, 100, 0, 24),
    v4(203, 0, 113, 0, 24),
    v4(224, 0, 0, 0, 3),
];

/// An IPv6 block: the first address and the length of the prefix.
type BlockV6 = (u128, u32);

const fn v6(groups: [u16; 8], prefix: u32) -> BlockV6 {
    let mut bits = 0u128;
    let mut i = 0;
    while i < 8 {
        bits = (bits << 16) | groups[i] as u128;
        i += 1;
    }
    (bits, prefix)
}

/// Blocks that are never public in IPv6. Those that carry an IPv4 address
/// inside are handled before this list is consulted.
const RESERVED_V6: &[BlockV6] = &[
    // The unspecified address, loopback and the deprecated IPv4-compatible
    // form: the low 96 bits are an IPv4 address or nothing.
    v6([0; 8], 96),
    // Discard-only.
    v6([0x0100, 0, 0, 0, 0, 0, 0, 0], 16),
    // Reserved for IETF protocol assignments. Teredo is in it, and its
    // addresses hide a server and a client address behind a mask.
    v6([0x2001, 0, 0, 0, 0, 0, 0, 0], 23),
    // Documentation.
    v6([0x2001, 0x0db8, 0, 0, 0, 0, 0, 0], 32),
    // 6to4: a gateway on the path unwraps the IPv4 address inside it, and
    // that address is not judged, so the whole block is refused.
    v6([0x2002, 0, 0, 0, 0, 0, 0, 0], 16),
    // Segment routing identifiers.
    v6([0x5f00, 0, 0, 0, 0, 0, 0, 0], 16),
    // Documentation, the larger block from 2024.
    v6([0x3fff, 0, 0, 0, 0, 0, 0, 0], 20),
    // NAT64 for local use.
    v6([0x0064, 0xff9b, 1, 0, 0, 0, 0, 0], 48),
    // Unique local.
    v6([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7),
    // Link-local, deprecated site-local and multicast.
    v6([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10),
    v6([0xfec0, 0, 0, 0, 0, 0, 0, 0], 10),
    v6([0xff00, 0, 0, 0, 0, 0, 0, 0], 8),
];

fn in_v4(address: u32, blocks: &[BlockV4]) -> bool {
    blocks.iter().any(|&(first, prefix)| {
        let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
        address & mask == first & mask
    })
}

fn in_v6(address: u128, blocks: &[BlockV6]) -> bool {
    blocks.iter().any(|&(first, prefix)| {
        let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
        address & mask == first & mask
    })
}

/// The IPv4 address an IPv6 address carries as a stand-in for it, when it
/// is one of the forms made for that: IPv4-mapped (::ffff:a.b.c.d),
/// IPv4-translated (::ffff:0:a.b.c.d) and the NAT64 well-known prefix
/// (64:ff9b::a.b.c.d). The connection is then judged by the address inside.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let bits = u128::from(address);
    let low = Ipv4Addr::from(bits as u32);
    let prefix = bits >> 32;
    // The high 96 bits: ::ffff, ::ffff:0 and 64:ff9b::.
    let mapped = 0xffff;
    let translated = 0xffff_0000;
    let nat64 = 0x0064_ff9b_0000_0000_0000_0000;
    (prefix == mapped || prefix == translated || prefix == nat64).then_some(low)
}

/// Whether an address is out on the internet. What is on none of the lists
/// of reserved blocks is public.
///
/// IPv4-mapped addresses are judged by the IPv4 address inside, as is the
/// NAT64 well-known prefix, so that `64:ff9b::808:808` (a public address
/// reached through a translator) is public and `64:ff9b::7f00:1` is not.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => !in_v4(u32::from(address), RESERVED_V4),
        IpAddr::V6(address) => match embedded_v4(address) {
            Some(inner) => is_public(IpAddr::V4(inner)),
            None => !in_v6(u128::from(address), RESERVED_V6),
        },
    }
}

/// Whether an address is the machine itself: 127.0.0.0/8, ::1 and the
/// IPv4-mapped form of the first.
pub fn is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_loopback(),
        IpAddr::V6(address) => match address.to_ipv4_mapped() {
            Some(inner) => inner.is_loopback(),
            None => address.is_loopback(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// The cases the link previewer's tests of this check named.
    #[test]
    fn keeps_every_verdict_the_previous_check_was_tested_for() {
        for text in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "2001:db8::1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "192.88.99.1",
            "240.0.0.1",
            "::",
            "100::1",
            "2002::1",
        ] {
            assert!(!is_public(ip(text)), "{text} must not be public");
        }
        for text in [
            "1.1.1.1",
            "93.184.216.34",
            "172.32.0.1",
            "192.167.1.1",
            "2606:4700:4700::1111",
            "64:ff9b::808:808",
        ] {
            assert!(is_public(ip(text)), "{text} must be public");
        }
    }

    #[test]
    fn refuses_every_reserved_ipv4_range_at_both_ends() {
        let refused = [
            "0.0.0.0",
            "0.255.255.255",
            "10.0.0.0",
            "10.255.255.255",
            "100.64.0.0",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.255",
            "169.254.0.0",
            "169.254.169.254",
            "172.16.0.0",
            "172.31.255.255",
            "192.0.0.0",
            "192.0.0.255",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.0.0",
            "192.168.255.255",
            "198.18.0.0",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.0",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
        ];
        for text in refused {
            assert!(!is_public(ip(text)), "{text} must not be public");
        }
    }

    #[test]
    fn allows_the_neighbours_of_every_reserved_ipv4_range() {
        let allowed = [
            "1.0.0.0",
            "8.8.8.8",
            "9.255.255.255",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "191.255.255.255",
            "192.0.1.1",
            "192.0.3.0",
            "192.88.98.255",
            "192.88.100.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "198.51.99.255",
            "198.51.101.0",
            "203.0.112.255",
            "203.0.114.0",
            "223.255.255.255",
        ];
        for text in allowed {
            assert!(is_public(ip(text)), "{text} must be public");
        }
    }

    #[test]
    fn refuses_every_reserved_ipv6_range() {
        let refused = [
            "::",
            "::1",
            "::8.8.8.8",
            "::7f00:1",
            "100::",
            "100::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:1ff:ffff::1",
            "2001:2::1",
            "2001:db8::1",
            "2002::1",
            "2002:808:808::1",
            "2002:7f00:1::1",
            "3fff::1",
            "3fff:fff:ffff::1",
            "5f00::1",
            "64:ff9b:1::1",
            "64:ff9b:1:ffff::1",
            "fc00::1",
            "fdff:ffff::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "feff::1",
            "ff00::1",
            "ff02::1",
        ];
        for text in refused {
            assert!(!is_public(ip(text)), "{text} must not be public");
        }
    }

    #[test]
    fn allows_ordinary_global_ipv6_addresses() {
        let allowed = [
            "2606:4700:4700::1111",
            "2a00:1450:4001:81b::200e",
            "2001:200::1",
            "2001:4860:4860::8888",
            "2003::1",
            "2400:cb00::1",
            "fb00::1",
            "fe7f::1",
            "2fff::1",
        ];
        for text in allowed {
            assert!(is_public(ip(text)), "{text} must be public");
        }
    }

    #[test]
    fn judges_a_mapped_address_by_the_ipv4_address_inside() {
        for text in [
            "::ffff:127.0.0.1",
            "::ffff:10.1.2.3",
            "::ffff:192.168.0.1",
            "::ffff:169.254.169.254",
            "::ffff:0.0.0.0",
            "::ffff:224.0.0.1",
            "::ffff:7f00:1",
        ] {
            assert!(!is_public(ip(text)), "{text} must not be public");
        }
        assert!(is_public(ip("::ffff:8.8.8.8")));
        assert!(is_public(ip("::ffff:808:808")));
    }

    #[test]
    fn judges_the_nat64_prefix_by_the_ipv4_address_inside() {
        assert!(is_public(ip("64:ff9b::808:808")));
        assert!(is_public(ip("64:ff9b::8.8.8.8")));
        assert!(!is_public(ip("64:ff9b::7f00:1")));
        assert!(!is_public(ip("64:ff9b::a00:1")));
        assert!(!is_public(ip("64:ff9b::c0a8:1")));
        assert!(!is_public(ip("64:ff9b::a9fe:a9fe")));
    }

    #[test]
    fn judges_an_ipv4_translated_address_by_the_ipv4_address_inside() {
        assert!(is_public(ip("::ffff:0:808:808")));
        assert!(!is_public(ip("::ffff:0:7f00:1")));
        assert!(!is_public(ip("::ffff:0:a00:1")));
    }

    #[test]
    fn refuses_the_forms_that_hide_an_ipv4_address_whatever_it_is() {
        // 6to4 and Teredo carry an address that is not judged here, so
        // neither is let through even when what is inside looks public.
        assert!(!is_public(ip("2002:808:808::1")));
        assert!(!is_public(ip("2001:0:808:808::1")));
        // The deprecated IPv4-compatible form.
        assert!(!is_public(ip("::808:808")));
    }

    #[test]
    fn loopback_means_this_machine_only() {
        for text in ["127.0.0.1", "127.9.9.9", "::1", "::ffff:127.0.0.1"] {
            assert!(is_loopback(ip(text)), "{text}");
        }
        for text in [
            "0.0.0.0",
            "10.0.0.1",
            "192.168.0.1",
            "8.8.8.8",
            "::",
            "fe80::1",
            "::ffff:10.0.0.1",
            "::ffff:8.8.8.8",
            "2606:4700::1",
        ] {
            assert!(!is_loopback(ip(text)), "{text}");
        }
    }

    #[test]
    fn each_policy_allows_what_it_says() {
        assert!(AddressPolicy::Anywhere.allows(ip("10.0.0.1")));
        assert!(AddressPolicy::Anywhere.allows(ip("8.8.8.8")));
        assert!(AddressPolicy::LoopbackOnly.allows(ip("127.0.0.1")));
        assert!(!AddressPolicy::LoopbackOnly.allows(ip("8.8.8.8")));
        assert!(!AddressPolicy::LoopbackOnly.allows(ip("10.0.0.1")));
        assert!(AddressPolicy::PublicOnly.allows(ip("8.8.8.8")));
        assert!(!AddressPolicy::PublicOnly.allows(ip("127.0.0.1")));
        assert!(!AddressPolicy::PublicOnly.allows(ip("::1")));
    }
}

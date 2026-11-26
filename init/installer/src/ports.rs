//! Local listeners from ss, lsof or netstat, grouped by port for one protocol.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ss,
    Lsof,
    Netstat,
}

fn ip(host: &str) -> Option<IpAddr> {
    let host = host.trim_matches(['[', ']']).split('%').next()?;
    host.parse::<IpAddr>().ok().map(|addr| addr.to_canonical())
}

// A bare published port covers every interface. An IPv6 wildcard in a
// system listing may be dual-stack; the listing does not say IPV6_V6ONLY.
fn overlaps(listener: Option<IpAddr>, host: Option<IpAddr>) -> bool {
    match (listener, host) {
        (Some(a), Some(b)) if a.is_ipv4() == b.is_ipv4() => {
            a == b || a.is_unspecified() || b.is_unspecified()
        }
        (Some(a), Some(b)) => {
            (a.is_ipv6() && a.is_unspecified()) || (b.is_ipv6() && b.is_unspecified())
        }
        _ => true,
    }
}

#[derive(Default)]
pub struct Listeners(BTreeMap<u32, BTreeSet<Option<IpAddr>>>);

impl Listeners {
    pub fn from_listing(tool: Tool, listing: &str) -> Self {
        let mut listeners = Self::default();
        for line in listing.lines() {
            let mut fields = line.split_whitespace();
            let endpoint = if tool == Tool::Lsof {
                fields.find(|&field| field == "TCP" || field == "UDP");
                fields.next().and_then(|field| field.split("->").next())
            } else {
                fields.nth(3)
            };
            // BSD netstat uses a dot before the port, including on IPv6.
            let Some((host, port)) = endpoint.and_then(|s| s.rsplit_once([':', '.'])) else {
                continue;
            };
            if let Ok(port) = port.parse() {
                let addr = if host == "*" {
                    let family = match tool {
                        Tool::Lsof => line.split_whitespace().nth(4),
                        Tool::Netstat => line.split_whitespace().next(),
                        // With numeric output ss prints 0.0.0.0 for IPv4
                        // and * for a dual-stack wildcard socket.
                        Tool::Ss => Some("IPv6"),
                    };
                    match family {
                        Some("IPv4" | "tcp4" | "udp4") => Some(Ipv4Addr::UNSPECIFIED.into()),
                        Some("IPv6" | "tcp6" | "udp6" | "tcp46" | "udp46") => {
                            Some(Ipv6Addr::UNSPECIFIED.into())
                        }
                        _ => None,
                    }
                } else {
                    ip(host)
                };
                listeners.0.entry(port).or_default().insert(addr);
            }
        }
        listeners
    }

    pub fn busy(&self, port: u32, host: &str) -> bool {
        let requested = ip(host);
        self.0.get(&port).is_some_and(|addresses| {
            addresses
                .iter()
                .any(|&listener| overlaps(listener, requested))
        })
    }

    /// Ignore only this container's published address. Another container
    /// can hold the same port on a different interface.
    pub fn remove_owned(&mut self, port: u32, host: &str) -> bool {
        let Some(addresses) = self.0.get_mut(&port) else {
            return false;
        };
        let Some(owned) = ip(host) else {
            return false;
        };
        addresses.remove(&Some(owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_local_address_of_each_tool() {
        for (tool, text) in [
            (
                Tool::Ss,
                "LISTEN 0 128 172.17.0.1:8888 0.0.0.0:*\nLISTEN 0 128 [::1]:22 [::]:*",
            ),
            (
                Tool::Lsof,
                "proc 1 ada 3u IPv4 0x1 0t0 TCP 172.17.0.1:8888->127.0.0.1:6000 (ESTABLISHED)\nproc 1 ada 3u IPv6 0x1 0t0 TCP [::1]:22",
            ),
            (
                Tool::Netstat,
                "tcp4 0 0 172.17.0.1.8888 *.* LISTEN\ntcp6 0 0 ::1.22 *.* LISTEN",
            ),
        ] {
            let listeners = Listeners::from_listing(tool, text);
            assert!(listeners.busy(8888, ""));
            assert!(listeners.busy(8888, "172.17.0.1"));
            assert!(!listeners.busy(8888, "127.0.0.1"));
            assert!(listeners.busy(22, "[0:0:0:0:0:0:0:1]"));
            assert!(!listeners.busy(22, "127.0.0.1"));
            assert!(!listeners.busy(6000, ""));
        }
    }

    #[test]
    fn wildcard_and_other_containers_are_still_busy() {
        let mut listeners = Listeners::from_listing(
            Tool::Ss,
            "LISTEN 0 128 0.0.0.0:443 0.0.0.0:*\nLISTEN 0 128 *:8443 *:*\nLISTEN 0 128 127.0.0.1:8888 0.0.0.0:*\nLISTEN 0 128 172.17.0.1:8888 0.0.0.0:*",
        );
        assert!(listeners.busy(443, "127.0.0.1"));
        assert!(listeners.busy(8443, "127.0.0.1"));
        assert!(listeners.remove_owned(8888, "127.0.0.1"));
        assert!(!listeners.busy(8888, "127.0.0.1"));
        assert!(listeners.busy(8888, ""));
    }
}

//! The ports something listens on, read from ss, lsof or netstat.

use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ss,
    Lsof,
    Netstat,
}

/// The local port of each line of a listing: the last part of the address,
/// after a colon or, in BSD's netstat, a dot. ss and netstat show the address
/// in the fourth column, lsof after the protocol.
pub fn port_numbers(tool: Tool, listing: &str) -> BTreeSet<u32> {
    let mut ports = BTreeSet::new();
    for line in listing.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let address = if tool == Tool::Lsof {
            let at = fields[..fields.len().saturating_sub(1)]
                .iter()
                .position(|f| *f == "TCP" || *f == "UDP");
            let address = at.map_or("", |i| fields[i + 1]);
            address.split("->").next().unwrap_or_default()
        } else {
            fields.get(3).copied().unwrap_or_default()
        };
        let parts: Vec<&str> = address.split([':', '.']).collect();
        if parts.len() > 1
            && let Some(last) = parts.last()
            && !last.is_empty()
            && last.bytes().all(|b| b.is_ascii_digit())
            && let Ok(port) = last.parse()
        {
            ports.insert(port);
        }
    }
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_tool() {
        let ss = "State  Recv-Q Send-Q Local Address:Port Peer Address:Port\nLISTEN 0 128 0.0.0.0:8443 0.0.0.0:*\nLISTEN 0 128 [::]:22 [::]:*\n";
        assert_eq!(port_numbers(Tool::Ss, ss), BTreeSet::from([22, 8443]));
        let lsof = "COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nproc 1 ada 3u IPv4 0x1 0t0 UDP *:3478\nproc 1 ada 3u IPv4 0x1 0t0 TCP 127.0.0.1:5000->127.0.0.1:6000 (ESTABLISHED)\n";
        assert_eq!(port_numbers(Tool::Lsof, lsof), BTreeSet::from([3478, 5000]));
        let netstat =
            "Proto Recv-Q Send-Q Local Address Foreign Address State\ntcp4 0 0 *.8888 *.* LISTEN\n";
        assert_eq!(port_numbers(Tool::Netstat, netstat), BTreeSet::from([8888]));
    }
}

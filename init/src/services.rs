//! The services of the Stellarr container, as the binary runs them.
//!
//! The entrypoint prepares the data volume, renders the configuration and
//! exports the settings before it hands over to the manager, so every service
//! finds its environment already in place and inherits it.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crate::service::{Readiness, Service, Signal};

/// Where the control socket listens.
pub const SOCKET: &str = "/run/stellarr/init.sock";

/// The longest a shutdown takes before whatever is left gets SIGKILL. It
/// covers the deadlines of all services one after the other. The container's
/// stop grace period must be longer, or Docker kills the manager first.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

const fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

/// The services in start order. Only the API needs another service ready
/// before it starts: it resets the media server on start.
pub fn container() -> Vec<Service> {
    vec![
        // Carries every call. It parses packets from the Internet and needs
        // no file and no privilege, so it does not run as root. Its control
        // API answers /v1/health without the bearer secret once the task
        // that owns the media plane is running.
        Service::new("media", &["/opt/stellarr/media/stellarr-media"])
            .user("nobody")
            .ready(Readiness::Http {
                addr: loopback(5200),
                path: "/v1/health".into(),
            })
            .stop(Signal::Term, Duration::from_secs(5))
            .critical(),
        // /health/ready answers once the database opens. The host's own
        // shutdown timeout is 30 seconds, the .NET default since 8; the
        // deadline here is longer so that the host always finishes first.
        Service::new(
            "api",
            &[
                "/usr/share/dotnet/dotnet",
                "/opt/stellarr/api/Stellarr.Api.dll",
            ],
        )
        .dir("/opt/stellarr/api")
        .needs(&["media"])
        .ready(Readiness::Http {
            addr: loopback(5000),
            path: "/health/ready".into(),
        })
        .stop(Signal::Term, Duration::from_secs(35))
        .critical(),
        // A binding request is the cheapest thing coturn answers, and it
        // needs no credentials. TLS renewal restarts it rather than reloading.
        Service::new(
            "turn",
            &["/usr/bin/turnserver", "-c", "/etc/turnserver.conf"],
        )
        .ready(Readiness::Stun(loopback(3478)))
        .stop(Signal::Term, Duration::from_secs(5))
        .critical(),
        // SIGTERM is nginx's fast shutdown. SIGQUIT, the graceful one, waits
        // for every open WebSocket and so would always run into the
        // deadline. SIGHUP reloads the configuration and the certificate.
        Service::new("nginx", &["/usr/sbin/nginx", "-g", "daemon off;"])
            .ready(Readiness::Tcp(loopback(80)))
            .stop(Signal::Term, Duration::from_secs(10))
            .reload(Signal::Hup)
            .critical(),
        // Renews a self-signed certificate twice a day at most and then
        // reloads nginx and restarts turn. The instance works without it
        // for weeks, so it is not critical, and it needs neither of the
        // two to be ready: a renewal must not wait for a broken turn.
        Service::new(
            "tls-maintenance",
            &["/usr/local/bin/stellarr-tls-maintenance", "watch"],
        )
        .ready(Readiness::Running(Duration::from_secs(1)))
        .stop(Signal::Term, Duration::from_secs(5)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Table;

    #[test]
    fn the_container_table_is_valid() {
        let table = Table::new(container()).expect("the table is valid");
        let names: Vec<&str> = table
            .order()
            .iter()
            .map(|&i| table.services()[i].name.as_str())
            .collect();
        assert_eq!(names, ["media", "api", "turn", "nginx", "tls-maintenance"]);
    }

    #[test]
    fn the_shutdown_covers_every_stop_deadline() {
        let sum: Duration = container().iter().map(|s| s.stop_timeout).sum();
        assert!(sum <= SHUTDOWN_TIMEOUT, "{sum:?} > {SHUTDOWN_TIMEOUT:?}");
    }

    #[test]
    fn the_supervisord_names_are_kept() {
        // tls-maintenance.sh restarts "turn" by name.
        let table = Table::new(container()).unwrap();
        assert!(table.index_of("turn").is_some());
    }
}

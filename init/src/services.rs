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
///
/// Each check is asked again every ten seconds once the service is ready, so
/// together they cover what the container's health check asked before this
/// manager: the API with its database, nginx serving, the media server's
/// control API, and a STUN answer from coturn.
pub fn container() -> Vec<Service> {
    vec![
        // Carries every call. It parses packets from the Internet and needs
        // no file and no privilege, so it does not run as root. Its control
        // API answers /v1/health without the bearer secret, from the task
        // that runs the media plane, so a stuck plane fails the check.
        Service::new("media", &["/opt/stellarr/media/stellarr-media"])
            .user("nobody")
            .ready(Readiness::http(loopback(5200), "/v1/health"))
            .stop(Signal::Term, Duration::from_secs(5))
            .critical(),
        // /health/ready answers only once the API reaches its database. The
        // API gives requests in flight 30 seconds to finish after SIGTERM;
        // the deadline here is longer so that the API always finishes first.
        Service::new("api", &["/opt/stellarr/api/stellarr-api"])
            .dir("/opt/stellarr/api")
            .needs(&["media"])
            .ready(Readiness::http(loopback(5000), "/health/ready"))
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
        // A page nginx serves from the image, asked for over plain HTTP as a
        // proxy in front would ask, since the check speaks no TLS: without
        // the header the answer is a redirect to HTTPS. A worker that
        // accepts nothing fails it, where a TCP connection alone would be
        // taken by the kernel. SIGTERM is nginx's fast shutdown; SIGQUIT,
        // the graceful one, waits for every open WebSocket and so would
        // always run into the deadline. SIGHUP reloads the configuration and
        // the certificate. -e stderr sends what nginx logs before it has read
        // its configuration, which keeps no access log and sends errors to
        // stderr too, to the container's log instead of /var/log/nginx.
        Service::new(
            "nginx",
            &["/usr/sbin/nginx", "-e", "stderr", "-g", "daemon off;"],
        )
        .ready(Readiness::Http {
            addr: loopback(80),
            path: "/index.html".into(),
            headers: vec![("X-Forwarded-Proto".into(), "https".into())],
        })
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
    fn the_names_scripts_and_docs_use_are_kept() {
        // tls-maintenance.sh reloads "nginx" and restarts "turn" by name,
        // and the operator docs name every service.
        let table = Table::new(container()).unwrap();
        for name in ["media", "api", "turn", "nginx", "tls-maintenance"] {
            assert!(table.index_of(name).is_some(), "{name}");
        }
    }
}

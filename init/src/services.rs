//! The services of the Stellarr container, as the binary runs them.
//!
//! The entrypoint prepares the data volume, renders the configuration and
//! exports the settings before it hands over to the manager, which passes
//! each service the environment it needs.

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
/// control API, and a STUN answer when coturn is enabled.
pub fn container() -> Vec<Service> {
    container_with_turn(std::env::var("STELLARR_TURN_ENABLED").as_deref() == Ok("true"))
}

const MEDIA_ENV: &[&str] = &[
    "STELLARR_MEDIA_BIND",
    "STELLARR_MEDIA_PUBLIC_IP",
    "STELLARR_MEDIA_PUBLIC_PORT",
    "STELLARR_MEDIA_ADVERTISE",
    "STELLARR_MEDIA_CONTROL_BIND",
    "STELLARR_MEDIA_CONTROL_SECRET",
    "STELLARR_MEDIA_LOG",
];

const TLS_ENV: &[&str] = &[
    "STELLARR_DATA_ROOT",
    "STELLARR_DOMAIN",
    "STELLARR_TLS_RENEW_BEFORE_SECONDS",
    "STELLARR_TLS_RENEW_CHECK_SECONDS",
    "STELLARR_TURN_ENABLED",
];

// Named API settings mirror api/crates/server/src/settings/variables.rs.
const API_ENV: &[&str] = &[
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
    "STELLARR_API_LISTEN",
    "STELLARR_BACKUP_ENABLED",
    "STELLARR_BACKUP_INITIAL_DELAY_MINUTES",
    "STELLARR_BACKUP_INTERVAL_HOURS",
    "STELLARR_BACKUP_RETRY_MINUTES",
    "STELLARR_CLOUDFLARE_R2_ACCESS_KEY",
    "STELLARR_CLOUDFLARE_R2_BUCKET",
    "STELLARR_CLOUDFLARE_R2_ENABLED",
    "STELLARR_CLOUDFLARE_R2_ENDPOINT",
    "STELLARR_CLOUDFLARE_R2_MODE",
    "STELLARR_CLOUDFLARE_R2_PREFIX",
    "STELLARR_CLOUDFLARE_R2_REGION",
    "STELLARR_CLOUDFLARE_R2_SECRET_KEY",
    "STELLARR_CLOUDFLARE_R2_WORKER_TOKEN",
    "STELLARR_DATA_ROOT",
    "STELLARR_DESKTOP_MINIMUM_VERSION",
    "STELLARR_DESKTOP_RECOMMENDED_VERSION",
    "STELLARR_DOMAIN",
    "STELLARR_ENVIRONMENT",
    "STELLARR_GIPHY_API_KEY",
    "STELLARR_INSTANCE_ID",
    "STELLARR_LANGUAGE",
    "STELLARR_MAIL_ENABLED",
    "STELLARR_MAIL_FROM",
    "STELLARR_MAIL_FROM_NAME",
    "STELLARR_MAX_CONCURRENT_UPLOADS",
    "STELLARR_MEDIA_PORT",
    "STELLARR_MIN_FREE_BYTES",
    "STELLARR_OIDC_CLIENT_ID",
    "STELLARR_OIDC_CLIENT_SECRET",
    "STELLARR_OIDC_CLIENT_SECRET_FILE",
    "STELLARR_OIDC_DISPLAY_NAME",
    "STELLARR_OIDC_ENABLED",
    "STELLARR_OIDC_ISSUER",
    "STELLARR_OIDC_PROVIDER_MFA_AMR",
    "STELLARR_OIDC_TRUST_PROVIDER_MFA",
    "STELLARR_PUSH_ENABLED",
    "STELLARR_PUSH_PRIVATE_KEY",
    "STELLARR_PUSH_PUBLIC_KEY",
    "STELLARR_PUSH_SUBJECT",
    "STELLARR_RESEND_API_KEY",
    "STELLARR_SMTP_PASSWORD",
    "STELLARR_TIME_ZONE",
    "STELLARR_TRUSTED_PROXIES",
    "STELLARR_TURNS_ENABLED",
    "STELLARR_TURNS_PORT",
    "STELLARR_TURN_ENABLED",
    "STELLARR_TURN_PORT",
    "STELLARR_TURN_TCP_ENABLED",
    "STELLARR_UPLOAD_TOTAL_BYTES",
    "STELLARR_UPLOAD_USER_BYTES",
];

// Preserve raw API setting keys, including indexed lists and log targets.
fn is_raw_api_setting(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if name == "dataroot" {
        return true;
    }
    let section = name.split("__").next().unwrap_or(&name);
    let section = section.split(':').next().unwrap_or(section);
    matches!(
        section,
        "backups"
            | "build"
            | "clients"
            | "cloudflare"
            | "desktop"
            | "giphy"
            | "http"
            | "logging"
            | "mail"
            | "maintenance"
            | "oidc"
            | "operations"
            | "push"
            | "ratelimiting"
            | "rtc"
            | "stellarr"
            | "storage"
            | "testing"
            | "uploads"
    ) && section.len() != name.len()
}

fn service_environment(service: &str, name: &str) -> bool {
    matches!(name, "PATH" | "TZ")
        || match service {
            "api" => API_ENV.contains(&name) || is_raw_api_setting(name),
            "media" => MEDIA_ENV.contains(&name),
            "tls-maintenance" => TLS_ENV.contains(&name),
            _ => false,
        }
}

fn container_with_turn(turn_enabled: bool) -> Vec<Service> {
    let mut services = vec![
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
            .user("stellarr")
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
        .user("turnserver")
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
        // reloads nginx and restarts the optional TURN service. Renewal can
        // run before nginx or TURN is ready, so it can replace a bad certificate.
        Service::new(
            "tls-maintenance",
            &["/usr/local/bin/stellarr-tls-maintenance", "watch"],
        )
        .ready(Readiness::Running(Duration::from_secs(1)))
        .stop(Signal::Term, Duration::from_secs(5)),
    ];
    if !turn_enabled {
        services.retain(|service| service.name != "turn");
    }
    for (name, value) in std::env::vars_os() {
        let (Some(name), Some(value)) = (name.to_str(), value.to_str()) else {
            continue;
        };
        for service in &mut services {
            if service_environment(&service.name, name) {
                service.env.push((name.to_string(), value.to_string()));
            }
        }
    }
    services
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Table;

    #[test]
    fn the_container_table_is_valid() {
        let table = Table::new(container_with_turn(true)).expect("the table is valid");
        let names: Vec<&str> = table
            .order()
            .iter()
            .map(|&i| table.services()[i].name.as_str())
            .collect();
        assert_eq!(names, ["media", "api", "turn", "nginx", "tls-maintenance"]);
    }

    #[test]
    fn direct_mode_does_not_run_or_check_coturn() {
        let table = Table::new(container_with_turn(false)).unwrap();
        assert!(table.index_of("turn").is_none());
        for name in ["media", "api", "nginx", "tls-maintenance"] {
            assert!(table.index_of(name).is_some(), "{name}");
        }
    }

    #[test]
    fn only_services_that_need_a_secret_in_their_environment_receive_it() {
        for (name, recipients) in [
            ("STELLARR_SMTP_PASSWORD", &["api"][..]),
            ("Rtc__TurnSharedSecret", &["api"][..]),
            ("STELLARR_MEDIA_CONTROL_SECRET", &["media"][..]),
            ("LD_PRELOAD", &[][..]),
        ] {
            for service in container_with_turn(true) {
                assert_eq!(
                    service_environment(&service.name, name),
                    recipients.contains(&service.name.as_str()),
                    "{name} for {}",
                    service.name
                );
            }
        }
    }

    #[test]
    fn the_shutdown_covers_every_stop_deadline() {
        let sum: Duration = container_with_turn(true)
            .iter()
            .map(|s| s.stop_timeout)
            .sum();
        assert!(sum <= SHUTDOWN_TIMEOUT, "{sum:?} > {SHUTDOWN_TIMEOUT:?}");
    }

    #[test]
    fn the_names_scripts_and_docs_use_are_kept() {
        // tls-maintenance.sh reloads "nginx" and restarts "turn" by name,
        // and the operator docs name every service.
        let table = Table::new(container_with_turn(true)).unwrap();
        for name in ["media", "api", "turn", "nginx", "tls-maintenance"] {
            assert!(table.index_of(name).is_some(), "{name}");
        }
    }
}

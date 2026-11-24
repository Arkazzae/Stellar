use axum::extract::ConnectInfo;
use axum::http::{header, request::Parts, uri::Authority};

use crate::http::{ClientInfo, linger::Peer};
use crate::settings::{Settings, normalize, origin::canonical};

pub(super) fn allows(settings: &Settings, parts: &Parts) -> bool {
    let mut origins = parts.headers.get_all(header::ORIGIN).iter();
    let Some(origin) = origins.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    // The browser sends one serialized origin, with no path or user info.
    if origins.next().is_some()
        || !origin.split_once("://").is_some_and(|(_, authority)| {
            authority.parse::<Authority>().is_ok() && !authority.contains('@')
        })
    {
        return false;
    }
    let Some(origin) = canonical(origin) else {
        return false;
    };
    if origin == settings.desktop.canonical_origin {
        return true;
    }

    // The bundled Nginx connects over loopback. A standalone deployment can
    // also explicitly trust its upstream proxy. Both preserve the public Host.
    let trusted =
        parts
            .extensions
            .get::<ConnectInfo<Peer>>()
            .is_some_and(|ConnectInfo(Peer(address))| {
                let address = normalize(address.ip());
                address.is_loopback() || settings.operations.trusted_proxies.contains(address)
            });
    let https = trusted
        && parts
            .extensions
            .get::<ClientInfo>()
            .is_some_and(ClientInfo::is_https);
    let scheme = if https { "https" } else { "http" };
    let Some(host) = parts
        .headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    host.parse::<Authority>().is_ok()
        && !host.contains('@')
        && canonical(&format!("{scheme}://{host}")).as_ref() == Some(&origin)
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    #[test]
    fn forwarded_scheme_counts_for_a_configured_proxy() {
        let settings = Settings::load(&[
            ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
            ("STELLARR_TRUSTED_PROXIES".into(), "192.0.2.10".into()),
        ])
        .unwrap();
        let (mut parts, ()) = Request::builder()
            .header(header::HOST, "chat.example.test:8443")
            .header(header::ORIGIN, "https://chat.example.test:8443")
            .extension(ClientInfo {
                address: "198.51.100.10".parse().unwrap(),
                scheme: "https".into(),
                host: None,
            })
            .body(())
            .unwrap()
            .into_parts();

        parts
            .extensions
            .insert(ConnectInfo(Peer("192.0.2.10:12345".parse().unwrap())));
        assert!(allows(&settings, &parts));

        parts
            .extensions
            .insert(ConnectInfo(Peer("192.0.2.20:12345".parse().unwrap())));
        assert!(!allows(&settings, &parts));
    }
}

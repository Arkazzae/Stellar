//! Short-lived credentials for the TURN relay, which verifies them with the
//! shared secret instead of keeping an account for every caller.
//!
//! The relay's time-limited credential scheme (the TURN REST API) is defined in
//! terms of HMAC-SHA1: the user name is the moment the credential ends and the
//! account, and the credential is the base64 of the MAC of that name under the
//! secret. It authenticates a message with a secret; it does not hash
//! passwords.

use std::time::Duration;

use aws_lc_rs::hmac;

use crate::encoding::base64;
use crate::settings::RtcSettings;
use crate::values::{Id, Timestamp};

/// How long a credential lasts.
pub const LIFETIME: Duration = Duration::from_secs(60 * 60);

/// The secret that is used when none is set, which only development and tests
/// can be without: a start elsewhere is refused without one.
const DEVELOPMENT_SECRET: &str = "stellarr-turn-development-secret";

/// What a browser is given to use the relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCredentials {
    pub username: String,
    pub credential: String,
    pub expires_at: Timestamp,
}

/// A credential for `user` that ends an hour after `now`.
pub fn credentials(settings: &RtcSettings, user: Id, now: Timestamp) -> TurnCredentials {
    let secret = settings
        .turn_shared_secret
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
        .unwrap_or(DEVELOPMENT_SECRET);
    sign(secret, user, now.plus(LIFETIME))
}

fn sign(secret: &str, user: Id, expires_at: Timestamp) -> TurnCredentials {
    let username = format!(
        "{}:{}",
        expires_at.unix_seconds(),
        user.to_string().replace('-', "")
    );
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret.as_bytes());
    let mac = hmac::sign(&key, username.as_bytes());
    TurnCredentials {
        credential: base64(mac.as_ref()),
        username,
        expires_at,
    }
}

/// The public media endpoint the bootstrap names, `udp://{host}:{port}`. It is
/// diagnostic only: a browser connects to the candidates in the session
/// description, never to this address.
pub fn public_endpoint(settings: &RtcSettings, host: &str) -> String {
    format!("udp://{host}:{}", settings.media.media_port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Id {
        Id::parse("30000000-0000-0000-0000-000000000001").unwrap()
    }

    // The two credentials below were made with `openssl dgst -sha1 -hmac`, which
    // is independent of this code:
    //   printf '1795513600:30000000000000000000000000000001' |
    //     openssl dgst -sha1 -hmac 'stellarr-turn-development-secret' -binary | base64
    #[test]
    fn a_credential_is_the_mac_openssl_makes_of_its_name() {
        let ends = Timestamp::from_unix_seconds(1_795_513_600).unwrap();

        let made = sign("stellarr-turn-development-secret", user(), ends);

        assert_eq!(made.username, "1795513600:30000000000000000000000000000001");
        assert_eq!(made.credential, "DJ3V6vMbgXvJjZVbkRkwhxFTRiw=");
        let other = sign("a different secret, with ünïcode", user(), ends);
        assert_eq!(other.credential, "N3BdQXd9wtM2UWsdzrVqzVrPgog=");
    }

    #[test]
    fn a_credential_ends_an_hour_from_now_and_the_secret_falls_back_in_development() {
        let settings = crate::settings::Settings::load(&[(
            "STELLARR_ENVIRONMENT".to_string(),
            "Testing".to_string(),
        )])
        .unwrap();
        let now = Timestamp::from_unix_seconds(1_795_510_000).unwrap();

        let made = credentials(&settings.rtc, user(), now);

        assert_eq!(made.expires_at.unix_seconds(), 1_795_513_600);
        assert!(made.username.starts_with("1795513600:"));
        assert_eq!(
            made,
            sign(DEVELOPMENT_SECRET, user(), made.expires_at),
            "an instance without a secret of its own uses the development one"
        );
    }

    #[test]
    fn the_public_endpoint_names_the_host_and_the_media_port() {
        let settings = crate::settings::Settings::load(&[
            ("STELLARR_ENVIRONMENT".to_string(), "Testing".to_string()),
            ("STELLARR_MEDIA_PORT".to_string(), "49123".to_string()),
        ])
        .unwrap();

        assert_eq!(
            public_endpoint(&settings.rtc, "chat.example.com"),
            "udp://chat.example.com:49123"
        );
        assert_eq!(public_endpoint(&settings.rtc, "localhost"), "udp://localhost:49123");
    }
}

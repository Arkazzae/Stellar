//! What a browser hands in when it subscribes, and the checks on it.
//!
//! Nothing here contacts the endpoint. A host name that resolves to a private
//! address cannot be caught by looking at the text, so the application must
//! also refuse such addresses when it opens the socket; this module refuses
//! everything that can be decided from the text alone.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::address::{is_public_v4, is_public_v6};
use crate::base64url;
use crate::p256;

/// Longest endpoint accepted, in bytes.
pub const ENDPOINT_MAX_LENGTH: usize = 2_048;
/// Longest encoded `p256dh` accepted, in characters.
pub const P256DH_MAX_LENGTH: usize = 256;
/// Longest encoded `auth` accepted, in characters.
pub const AUTH_MAX_LENGTH: usize = 64;
/// Length of the authentication secret in bytes.
pub const AUTH_SECRET_LEN: usize = 16;

/// A subscription whose three parts passed validation.
///
/// `endpoint` is the normalized form (lower-case host, no default port, an
/// empty path written as `/`) and is what should be stored and used.
#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedSubscription {
    endpoint: String,
    p256dh: [u8; p256::POINT_LEN],
    auth: [u8; AUTH_SECRET_LEN],
}

impl std::fmt::Debug for ValidatedSubscription {
    // The auth secret is a credential; it stays out of logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedSubscription")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl ValidatedSubscription {
    /// Validates the three values of a Push API subscription.
    ///
    /// Returns `None` when any of them is unusable.
    pub fn validate(
        endpoint: Option<&str>,
        p256dh: Option<&str>,
        auth: Option<&str>,
    ) -> Option<Self> {
        Some(Self {
            endpoint: normalize_endpoint(endpoint?)?,
            p256dh: decode_p256dh(p256dh?)?,
            auth: decode_auth(auth?)?,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The receiver's public key, an uncompressed P-256 point.
    pub fn p256dh(&self) -> &[u8; p256::POINT_LEN] {
        &self.p256dh
    }

    /// The receiver's authentication secret.
    pub fn auth(&self) -> &[u8; AUTH_SECRET_LEN] {
        &self.auth
    }

    /// The key in the unpadded base64url form that is stored.
    pub fn p256dh_text(&self) -> String {
        base64url::encode(&self.p256dh)
    }

    /// The secret in the unpadded base64url form that is stored.
    pub fn auth_text(&self) -> String {
        base64url::encode(&self.auth)
    }

    /// The origin a VAPID token is addressed to: scheme and host, no port.
    pub fn audience(&self) -> &str {
        let rest = &self.endpoint["https://".len()..];
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        &self.endpoint[..("https://".len() + end)]
    }
}

/// Normalizes an endpoint, or refuses it.
///
/// An endpoint must be an `https` URL on the default port, without user
/// information or a fragment, with a host that is neither a local name nor a
/// reserved address. Anything that is not plain ASCII, or that holds a space,
/// a control character or a backslash, is refused rather than rewritten: a
/// push service hands out plain URLs, and rewriting is where parsers disagree.
pub fn normalize_endpoint(value: &str) -> Option<String> {
    let candidate = value.trim();
    if candidate.is_empty() || candidate.len() > ENDPOINT_MAX_LENGTH {
        return None;
    }
    if candidate
        .bytes()
        .any(|b| b <= 0x20 || b >= 0x7f || b == b'\\' || b == b'#')
    {
        return None;
    }

    let scheme_len = "https://".len();
    if candidate.len() <= scheme_len || !candidate[..scheme_len].eq_ignore_ascii_case("https://") {
        return None;
    }
    let rest = &candidate[scheme_len..];
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    if authority.contains('@') {
        return None;
    }

    let (host_text, port_text) = split_host_port(authority)?;
    if let Some(port) = port_text {
        let digits_ok =
            !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
        if !digits_ok || port.parse::<u32>().ok()? != 443 {
            return None;
        }
    }
    let host = normalize_host(host_text)?;

    let tail = if tail.is_empty() || tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_owned()
    };
    let endpoint = format!("https://{host}{tail}");
    (endpoint.len() <= ENDPOINT_MAX_LENGTH).then_some(endpoint)
}

fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if let Some(inner) = authority.strip_prefix('[') {
        let close = inner.find(']')?;
        let after = &inner[close + 1..];
        let host = &authority[..close + 2];
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(after.strip_prefix(':')?))),
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((authority, None)),
    }
}

/// Returns the host as it is written into the normalized endpoint.
fn normalize_host(text: &str) -> Option<String> {
    if let Some(inner) = text.strip_prefix('[') {
        let literal = inner.strip_suffix(']')?;
        let address: Ipv6Addr = literal.parse().ok()?;
        return is_public_v6(address).then(|| format!("[{address}]"));
    }

    let host = text.to_ascii_lowercase();
    let bare = host.strip_suffix('.').unwrap_or(&host);
    if bare.is_empty() || host.len() > 253 {
        return None;
    }
    let mut labels = bare.split('.');
    let well_formed = labels.all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    });
    if !well_formed {
        return None;
    }

    if bare == "localhost"
        || [".localhost", ".local", ".internal", ".home.arpa"]
            .iter()
            .any(|suffix| bare.ends_with(suffix))
    {
        return None;
    }

    // A last label that starts like a number makes the whole name an IPv4
    // address in every resolver's eyes, in the short and the hex forms too.
    let last = bare.rsplit('.').next().unwrap_or("");
    if last.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        let address = parse_legacy_ipv4(bare)?;
        return is_public_v4(address).then(|| address.to_string());
    }
    Some(bare.to_owned())
}

/// Parses the classic `inet_aton` spellings: one to four parts, each decimal,
/// `0x` hexadecimal or `0` octal, the last part filling the remaining bytes.
fn parse_legacy_ipv4(text: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut numbers = Vec::with_capacity(parts.len());
    for part in &parts {
        let (digits, radix) =
            if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                (hex, 16)
            } else if part.len() > 1 && part.starts_with('0') {
                (&part[1..], 8)
            } else {
                (*part, 10)
            };
        if digits.is_empty() || digits.len() > 12 {
            return None;
        }
        numbers.push(u64::from_str_radix(digits, radix).ok()?);
    }
    let (last, leading) = numbers.split_last()?;
    if leading.iter().any(|&n| n > 255) {
        return None;
    }
    let room = 8 * (4 - leading.len() as u32);
    if *last >= 1u64 << room {
        return None;
    }
    let mut value = *last;
    for (index, &n) in leading.iter().enumerate() {
        value |= n << (8 * (3 - index as u32));
    }
    Some(Ipv4Addr::from(u32::try_from(value).ok()?))
}

fn decode_p256dh(text: &str) -> Option<[u8; p256::POINT_LEN]> {
    let bytes = base64url::decode(text, P256DH_MAX_LENGTH)?;
    let point = <[u8; p256::POINT_LEN]>::try_from(bytes).ok()?;
    p256::is_valid_point(&point).then_some(point)
}

fn decode_auth(text: &str) -> Option<[u8; AUTH_SECRET_LEN]> {
    let bytes = base64url::decode(text, AUTH_MAX_LENGTH)?;
    <[u8; AUTH_SECRET_LEN]>::try_from(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::subscription_keys;

    fn endpoint(value: &str) -> Option<String> {
        normalize_endpoint(value)
    }

    #[test]
    fn plain_https_endpoints_are_kept() {
        assert_eq!(
            endpoint("https://push.example.test/send/subscription").as_deref(),
            Some("https://push.example.test/send/subscription")
        );
        assert_eq!(
            endpoint("  https://FCM.Example.TEST:443/fcm/send/abc?x=1  ").as_deref(),
            Some("https://fcm.example.test/fcm/send/abc?x=1")
        );
        assert_eq!(
            endpoint("HTTPS://push.example.test").as_deref(),
            Some("https://push.example.test/")
        );
        assert_eq!(
            endpoint("https://push.example.test?token=1").as_deref(),
            Some("https://push.example.test/?token=1")
        );
        assert_eq!(
            endpoint("https://[2606:4700:4700::1111]/x").as_deref(),
            Some("https://[2606:4700:4700::1111]/x")
        );
        assert_eq!(
            endpoint("https://8.8.8.8/x").as_deref(),
            Some("https://8.8.8.8/x")
        );
    }

    #[test]
    fn local_and_non_default_endpoints_are_refused() {
        for value in [
            "https://127.0.0.1/send",
            "https://127.0.0.1:9/send",
            "https://[::1]/send",
            "https://[::1]:9/send",
            "https://localhost/send",
            "https://localhost:9/send",
            "https://LOCALHOST./send",
            "https://app.localhost/send",
            "https://printer.local/send",
            "https://db.internal/send",
            "https://router.home.arpa/send",
            "http://push.example.test/send",
            "https://push.example.test:8443/send",
            "https://push.example.test:/send",
            "https://push.example.test:0443x/send",
            "https://user:secret@push.example.test/send",
            "https://user@push.example.test/send",
            "https://push.example.test/send#part",
            "https://push.example.test/send#",
            "https://10.0.0.1/send",
            "https://192.168.1.1/send",
            "https://169.254.1.1/send",
            "https://172.16.0.9/send",
            "https://100.64.0.1/send",
            "https://[fe80::1]/send",
            "https://[fd00::1]/send",
            "https://[::ffff:127.0.0.1]/send",
            "https://[::ffff:10.0.0.1]/send",
            "https://[fe80::1%25eth0]/send",
        ] {
            assert_eq!(endpoint(value), None, "{value}");
        }
    }

    #[test]
    fn disguised_ipv4_addresses_are_refused() {
        for value in [
            "https://2130706433/send",
            "https://0x7f000001/send",
            "https://0x7f.1/send",
            "https://127.1/send",
            "https://0177.0.0.1/send",
            "https://10.1/send",
            "https://1.2.3.4.5/send",
            "https://999.1.1.1/send",
            "https://256.1.1.1/send",
            "https://127.0.0.1./send",
            "https://1e3/send",
        ] {
            assert_eq!(endpoint(value), None, "{value}");
        }
        // The same spellings of a public address are accepted, written plainly.
        assert_eq!(
            endpoint("https://134744072/x").as_deref(),
            Some("https://8.8.8.8/x")
        );
    }

    #[test]
    fn malformed_endpoints_are_refused() {
        let long = format!("https://push.example.test/{}", "a".repeat(2_048));
        for value in [
            "",
            "   ",
            "https://",
            "https:///send",
            "https://push.example.test/ send",
            "https://push.example.test/\tsend",
            "https://push.example.test/\nsend",
            "https://push.example.test/\u{0}send",
            "https://push.example.test/sënd",
            "https://pûsh.example.test/send",
            "https://push.example.test\\@evil.test/send",
            "https://push..example.test/send",
            "https://.example.test/send",
            "https://-.example.test/send",
            "https://push.exa mple.test/send",
            "https://push.example.test%2eevil/send",
            "https://[::1/send",
            "https://[::1]x/send",
            "https://[]/send",
            "ftp://push.example.test/send",
            "//push.example.test/send",
            "push.example.test/send",
            long.as_str(),
        ] {
            assert_eq!(endpoint(value), None, "{value:?}");
        }
    }

    #[test]
    fn endpoint_at_the_length_limit_is_kept() {
        let prefix = "https://push.example.test/";
        let value = format!("{prefix}{}", "a".repeat(ENDPOINT_MAX_LENGTH - prefix.len()));
        assert_eq!(value.len(), ENDPOINT_MAX_LENGTH);
        assert_eq!(endpoint(&value).as_deref(), Some(value.as_str()));
        assert_eq!(endpoint(&format!("{value}a")), None);
    }

    #[test]
    fn a_valid_subscription_validates_and_round_trips() {
        let keys = subscription_keys();
        let checked = ValidatedSubscription::validate(
            Some("https://push.example.test/send/subscription"),
            Some(&keys.p256dh_text),
            Some(&keys.auth_text),
        )
        .expect("valid");
        assert_eq!(
            checked.endpoint(),
            "https://push.example.test/send/subscription"
        );
        assert_eq!(checked.p256dh_text(), keys.p256dh_text);
        assert_eq!(checked.auth_text(), keys.auth_text);
        assert_eq!(checked.audience(), "https://push.example.test");
        assert!(!format!("{checked:?}").contains(&keys.auth_text));
    }

    #[test]
    fn a_trailing_dot_is_dropped_from_endpoint_and_audience() {
        let keys = subscription_keys();
        let checked = ValidatedSubscription::validate(
            Some("https://a.test./x"),
            Some(&keys.p256dh_text),
            Some(&keys.auth_text),
        )
        .unwrap();
        assert_eq!(checked.endpoint(), "https://a.test/x");
        assert_eq!(checked.audience(), "https://a.test");
        assert_eq!(endpoint("https://LOCALHOST./x"), None);
        assert_eq!(endpoint("https://box.local./x"), None);
        assert_eq!(endpoint("https://a.test../x"), None);
    }

    #[test]
    fn audience_ignores_path_and_query() {
        let keys = subscription_keys();
        let make = |url| {
            ValidatedSubscription::validate(
                Some(url),
                Some(&keys.p256dh_text),
                Some(&keys.auth_text),
            )
            .unwrap()
        };
        assert_eq!(
            make("https://p.example.test?x=/y").audience(),
            "https://p.example.test"
        );
        assert_eq!(
            make("https://[2606:4700:4700::1111]/a").audience(),
            "https://[2606:4700:4700::1111]"
        );
    }

    #[test]
    fn missing_parts_are_refused() {
        let keys = subscription_keys();
        let ok = Some("https://push.example.test/x");
        assert!(
            ValidatedSubscription::validate(None, Some(&keys.p256dh_text), Some(&keys.auth_text))
                .is_none()
        );
        assert!(ValidatedSubscription::validate(ok, None, Some(&keys.auth_text)).is_none());
        assert!(ValidatedSubscription::validate(ok, Some(&keys.p256dh_text), None).is_none());
    }

    #[test]
    fn malformed_keys_are_refused() {
        let keys = subscription_keys();
        let check = |p256dh: &str, auth: &str| {
            ValidatedSubscription::validate(
                Some("https://push.example.test/send/keys"),
                Some(p256dh),
                Some(auth),
            )
        };
        assert!(check(&keys.p256dh_text, &keys.auth_text).is_some());
        // Not base64url at all.
        assert!(check("not-base64!", &keys.auth_text).is_none());
        // The right length but not a point: all zeros, and a zero point with the prefix.
        assert!(check(&base64url::encode(&[0u8; 64]), &keys.auth_text).is_none());
        assert!(check(&base64url::encode(&[0u8; 65]), &keys.auth_text).is_none());
        let mut off_curve = [0u8; 65];
        off_curve[0] = 0x04;
        assert!(check(&base64url::encode(&off_curve), &keys.auth_text).is_none());
        // A real point nudged off the curve.
        let mut nudged = keys.p256dh;
        nudged[64] ^= 1;
        assert!(check(&base64url::encode(&nudged), &keys.auth_text).is_none());
        // A compressed or truncated or over-long key.
        let mut compressed = keys.p256dh[..33].to_vec();
        compressed[0] = 0x02;
        assert!(check(&base64url::encode(&compressed), &keys.auth_text).is_none());
        assert!(check(&base64url::encode(&keys.p256dh[..64]), &keys.auth_text).is_none());
        let mut long = keys.p256dh.to_vec();
        long.push(0);
        assert!(check(&base64url::encode(&long), &keys.auth_text).is_none());
        assert!(check(&"A".repeat(400), &keys.auth_text).is_none());
        // Auth of the wrong length or alphabet, or with a stray character.
        assert!(check(&keys.p256dh_text, &base64url::encode(&[0u8; 15])).is_none());
        assert!(check(&keys.p256dh_text, &base64url::encode(&[0u8; 17])).is_none());
        assert!(check(&keys.p256dh_text, &format!("{}+", keys.auth_text)).is_none());
        assert!(check(&keys.p256dh_text, "").is_none());
        assert!(check("", &keys.auth_text).is_none());
        assert!(check(&keys.p256dh_text, &"A".repeat(100)).is_none());
    }

    // A receiver key from RFC 8291 whose text holds both '-' and '_'.
    const KEY_WITH_URL_SAFE_CHARS: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AUTH_OF_THE_SAME: &str = "BTBZMqHH6r4Tts7J_aSIgg";

    fn with_key(p256dh: &str) -> Option<ValidatedSubscription> {
        ValidatedSubscription::validate(
            Some("https://push.example.test/x"),
            Some(p256dh),
            Some(AUTH_OF_THE_SAME),
        )
    }

    #[test]
    fn a_key_in_the_standard_base64_alphabet_is_refused() {
        assert!(KEY_WITH_URL_SAFE_CHARS.contains('-') && KEY_WITH_URL_SAFE_CHARS.contains('_'));
        assert!(with_key(KEY_WITH_URL_SAFE_CHARS).is_some());
        let standard = KEY_WITH_URL_SAFE_CHARS.replace('-', "+").replace('_', "/");
        assert!(with_key(&standard).is_none());
    }

    #[test]
    fn a_key_with_exact_padding_is_accepted_and_stored_without_it() {
        // 65 bytes encode to 87 characters; one '=' completes the last group.
        let padded = format!("{KEY_WITH_URL_SAFE_CHARS}=");
        let checked = with_key(&padded).expect("padding is accepted");
        assert_eq!(checked.p256dh_text(), KEY_WITH_URL_SAFE_CHARS);
        assert!(with_key(&format!("{KEY_WITH_URL_SAFE_CHARS}==")).is_none());
    }
}

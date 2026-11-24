//! The settings of the providers people can sign in through, and the small
//! rules around them: which providers count as configured, where a person may
//! be sent back to, how an issuer is stored, and when a provider's own second
//! step counts.

use std::collections::BTreeMap;
use std::fmt;

use crate::form;

pub const DEFAULT_PROVIDER_ID: &str = "default";
pub const DEFAULT_MFA_AMR: &str = "mfa";
pub const DEFAULT_CALLBACK_PATH: &str = "/api/auth/oidc/callback";

/// Ids a named provider cannot take: the original one, and the words of the routes.
const RESERVED_IDS: [&str; 6] = [
    DEFAULT_PROVIDER_ID,
    "login",
    "callback",
    "link",
    "reauthenticate",
    "pending",
];

/// How recent the provider's own sign-in must be to count, in seconds. The
/// provider is asked for it with `max_age`, and its `auth_time` must show it.
pub const PROVIDER_MFA_MAX_AGE: i64 = 5 * 60;

/// How far the provider's clock may be ahead of ours, or behind, when its
/// `auth_time` is read. A different tolerance from the token's own times.
const AUTH_TIME_SKEW: i64 = 60;

/// The longest return address kept. The address travels inside the sealed
/// state, so it is bounded.
const MAX_RETURN_URL: usize = 2048;

const MAX_UNIX_SECONDS: i64 = 253_402_300_799;

/// One provider as configured. Every provider, the original or a named one,
/// has these fields.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub authority: String,
    pub client_id: String,
    pub client_secret: String,
    /// A file holding the secret, for deployments that mount secrets. The
    /// application reads it; this crate never touches the disk.
    pub client_secret_file: String,
    pub display_name: String,
    /// Off by default: the provider only proves who someone is, and the
    /// instance asks for its own second step after it.
    pub trust_provider_mfa: bool,
    /// The `amr` values, comma-separated, that mean more than a password.
    pub provider_mfa_amr: String,
}

impl fmt::Debug for ProviderSettings {
    // The secret never reaches a log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("enabled", &self.enabled)
            .field("authority", &self.authority)
            .field("client_id", &self.client_id)
            .field("display_name", &self.display_name)
            .finish_non_exhaustive()
    }
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            authority: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            client_secret_file: String::new(),
            display_name: String::new(),
            trust_provider_mfa: false,
            provider_mfa_amr: DEFAULT_MFA_AMR.to_string(),
        }
    }
}

/// The whole single sign-on section: the provider at the top keeps the
/// original callback and the id `default`; more are named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The provider at the top of the section. Off unless the operator turns it on.
    pub original: ProviderSettings,
    pub callback_path: String,
    pub named: BTreeMap<String, ProviderSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            original: ProviderSettings {
                enabled: false,
                ..ProviderSettings::default()
            },
            callback_path: DEFAULT_CALLBACK_PATH.to_string(),
            named: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionsError {
    /// A named provider's id is not a short lower-case name no route uses.
    InvalidProviderId(String),
}

impl fmt::Display for OptionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProviderId(id) => write!(
                f,
                "{id:?} is not a valid provider id: use 1 to 32 lower-case letters, digits or hyphens, not {}",
                RESERVED_IDS.join(", ")
            ),
        }
    }
}

impl std::error::Error for OptionsError {}

impl Settings {
    /// Every provider that can really be used: the original one first, then
    /// the named ones in the order of their ids. A named provider whose id is
    /// not valid stops everything, since its callback could not be registered
    /// with the provider as written, whether or not it is otherwise usable.
    /// `read_secret_file` returns the content of a secret file, or `None`.
    pub fn configured_providers(
        &self,
        read_secret_file: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<Provider>, OptionsError> {
        let mut providers = Vec::new();
        let original = Provider::new(
            DEFAULT_PROVIDER_ID,
            &self.callback_path,
            &self.original,
            &read_secret_file,
        );
        if original.is_configured() {
            providers.push(original);
        }
        for (id, settings) in &self.named {
            if !is_valid_id(id) {
                return Err(OptionsError::InvalidProviderId(id.clone()));
            }
            let path = format!("/api/auth/oidc/{id}/callback");
            let named = Provider::new(id, &path, settings, &read_secret_file);
            if named.is_configured() {
                providers.push(named);
            }
        }
        Ok(providers)
    }

    pub fn find(
        &self,
        id: &str,
        read_secret_file: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Provider>, OptionsError> {
        Ok(self
            .configured_providers(read_secret_file)?
            .into_iter()
            .find(|provider| provider.id == id))
    }
}

fn is_valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !id.starts_with('-')
        && !RESERVED_IDS.contains(&id)
}

/// A provider with its id, its callback and its secret resolved.
#[derive(Clone, PartialEq, Eq)]
pub struct Provider {
    pub id: String,
    pub callback_path: String,
    pub settings: ProviderSettings,
    client_secret: String,
}

impl fmt::Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("callback_path", &self.callback_path)
            .finish_non_exhaustive()
    }
}

impl Provider {
    fn new(
        id: &str,
        callback_path: &str,
        settings: &ProviderSettings,
        read_secret_file: &impl Fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            id: id.to_string(),
            callback_path: callback_path.to_string(),
            settings: settings.clone(),
            client_secret: resolve_secret(settings, read_secret_file),
        }
    }

    /// The name the provider's sign-in is registered under.
    pub fn scheme(&self) -> String {
        self.route("external-oidc", "-")
    }

    pub fn login_path(&self) -> String {
        self.path("login")
    }

    pub fn link_path(&self) -> String {
        self.path("link")
    }

    pub fn reauthenticate_path(&self) -> String {
        self.path("reauthenticate")
    }

    fn route(&self, base: &str, glue: &str) -> String {
        if self.id == DEFAULT_PROVIDER_ID {
            base.to_string()
        } else {
            format!("{base}{glue}{}", self.id)
        }
    }

    fn path(&self, action: &str) -> String {
        if self.id == DEFAULT_PROVIDER_ID {
            format!("/api/auth/oidc/{action}")
        } else {
            format!("/api/auth/oidc/{}/{action}", self.id)
        }
    }

    /// The authority without a trailing slash, as the discovery address is built from it.
    pub fn authority(&self) -> &str {
        self.settings.authority.trim().trim_end_matches('/')
    }

    pub fn client_id(&self) -> &str {
        &self.settings.client_id
    }

    /// The inline secret, else the content of the secret file, else an empty string.
    pub fn client_secret(&self) -> &str {
        &self.client_secret
    }

    /// True when the provider can really work: enabled, reached over HTTPS,
    /// and a confidential client with a secret.
    pub fn is_configured(&self) -> bool {
        self.settings.enabled
            && is_https_url(&self.settings.authority)
            && !self.settings.client_id.trim().is_empty()
            && !self.client_secret.is_empty()
            && self.callback_path.starts_with('/')
    }

    /// Whether the provider says it asked for more than a password, recently
    /// enough, and the instance trusts it to.
    pub fn passed_second_step<S: AsRef<str>>(
        &self,
        amr: &[S],
        auth_time: Option<&str>,
        now: i64,
    ) -> bool {
        if !self.settings.trust_provider_mfa || !signed_in_recently(auth_time, now) {
            return false;
        }
        let accepted: Vec<&str> = self
            .settings
            .provider_mfa_amr
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect();
        amr.iter().any(|value| accepted.contains(&value.as_ref()))
    }
}

fn resolve_secret(
    settings: &ProviderSettings,
    read_secret_file: &impl Fn(&str) -> Option<String>,
) -> String {
    let inline = settings.client_secret.trim();
    let path = settings.client_secret_file.trim();
    if !inline.is_empty() || path.is_empty() {
        return inline.to_string();
    }
    read_secret_file(path)
        .map(|content| content.trim().to_string())
        .unwrap_or_default()
}

/// An issuer as it is stored: providers differ on a trailing slash.
pub fn canonical_issuer(issuer: &str) -> &str {
    issuer.trim().trim_end_matches('/')
}

/// When the provider's `auth_time` says the person signed in there; `None`
/// when it is missing or unreadable. Digits only: no sign, no space, no fraction.
pub fn signed_in_at(auth_time: Option<&str>) -> Option<i64> {
    let text = auth_time?;
    if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>()
        .ok()
        .map(|s| s.clamp(0, MAX_UNIX_SECONDS))
}

/// Whether the provider's `auth_time` says the person signed in there within
/// [`PROVIDER_MFA_MAX_AGE`], give or take the clock skew. A missing or
/// unreadable time does not count.
pub fn signed_in_recently(auth_time: Option<&str>, now: i64) -> bool {
    signed_in_at(auth_time).is_some_and(|at| {
        at <= now + AUTH_TIME_SKEW && at >= now - PROVIDER_MFA_MAX_AGE - AUTH_TIME_SKEW
    })
}

/// When a fresh sign-in at the provider counts from, for a sign-in asked for
/// as proof: when the provider says the person signed in, not when it
/// answered, so a sign-in three minutes old leaves two of the five fresh
/// minutes. A provider whose clock is ahead counts as now. `None` when the
/// time is missing, unreadable, or already past the fresh window.
pub fn fresh_sign_in_time(auth_time: Option<&str>, now: i64) -> Option<i64> {
    let at = signed_in_at(auth_time).filter(|_| signed_in_recently(auth_time, now))?;
    let signed_in = at.min(now);
    (signed_in >= now - PROVIDER_MFA_MAX_AGE).then_some(signed_in)
}

/// Whether the text, trimmed, is an absolute `https` address with a host: no
/// spaces or control characters, no credentials, no fragment, a port that is
/// a number up to 65535, and an IPv6 host that is closed.
pub fn is_https_url(url: &str) -> bool {
    let url = url.trim();
    let Some(rest) = url
        .get(..8)
        .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty()
        && !authority.contains('@')
        && host_and_port_ok(authority)
        && !url.contains('#')
        && url
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && c != '\\')
}

fn host_and_port_ok(authority: &str) -> bool {
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let Some((host, after)) = inner.split_once(']') else {
            return false;
        };
        match after.strip_prefix(':') {
            Some(port) => (host, port),
            None if after.is_empty() => (host, ""),
            None => return false,
        }
    } else {
        authority.split_once(':').unwrap_or((authority, ""))
    };
    !host.is_empty()
        && (authority.starts_with('[') || !host.contains(['[', ']']))
        && port.len() <= 5
        && port.bytes().all(|c| c.is_ascii_digit())
        && (port.is_empty() || port.parse::<u32>().is_ok_and(|p| p <= 65535))
}

/// Where to send the person once signed in. Only a path on this origin is
/// accepted, so the sign-in link cannot be used to bounce them elsewhere.
pub fn safe_return_url(return_url: Option<&str>) -> String {
    let candidate = return_url.unwrap_or("").trim();
    let decoded = form::unescape(candidate);
    if candidate.len() <= MAX_RETURN_URL && leads_here(candidate) && leads_here(&decoded) {
        candidate.to_string()
    } else {
        "/".to_string()
    }
}

/// A rooted path no browser reads as another host. `.` and `..` segments are
/// refused too: a URL parser folds them, so `/.//host` would become `//host`
/// on the page that follows the path.
fn leads_here(path: &str) -> bool {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return false;
    }
    let route = path.split(['?', '#']).next().unwrap_or("");
    !route
        .split('/')
        .any(|segment| segment == "." || segment == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_files(_: &str) -> Option<String> {
        None
    }

    fn configured(authority: &str) -> ProviderSettings {
        ProviderSettings {
            authority: authority.into(),
            client_id: "stellarr".into(),
            client_secret: "secret".into(),
            ..ProviderSettings::default()
        }
    }

    #[test]
    fn return_url_must_stay_on_the_current_origin() {
        let cases = [
            (None, "/"),
            (Some(""), "/"),
            (Some("/"), "/"),
            (Some("/invite/abc?source=oidc"), "/invite/abc?source=oidc"),
            (Some("https://evil.example/"), "/"),
            (Some("//evil.example/"), "/"),
            (Some("/\\evil.example/"), "/"),
            (Some("/.//evil.example"), "/"),
            (Some("/%2e//evil.example"), "/"),
            (Some("/a/..//evil.example"), "/"),
            (Some("/%2F%2Fevil.example"), "/"),
            (Some("/invite/v1.2"), "/invite/v1.2"),
        ];
        for (value, expected) in cases {
            assert_eq!(safe_return_url(value), expected, "{value:?}");
        }
    }

    #[test]
    fn return_url_refuses_control_characters_and_oversize() {
        assert_eq!(safe_return_url(Some("/a\nb")), "/");
        assert_eq!(safe_return_url(Some("/a%0Ab")), "/");
        assert_eq!(
            safe_return_url(Some(&format!("/{}", "a".repeat(2048)))),
            "/"
        );
        assert_eq!(safe_return_url(Some("  /ok  ")), "/ok");
    }

    #[test]
    fn named_provider_needs_a_short_lower_case_id_that_no_route_uses() {
        for id in [
            "Work",
            "work space",
            "-work",
            "default",
            "login",
            "callback",
            "link",
            "pending",
            "reauthenticate",
            "",
            &"a".repeat(33),
        ] {
            let mut settings = Settings::default();
            settings
                .named
                .insert(id.into(), configured("https://work.example.test"));
            assert_eq!(
                settings.configured_providers(no_files),
                Err(OptionsError::InvalidProviderId(id.into())),
                "{id:?}"
            );
        }
        let mut ok = Settings::default();
        ok.named
            .insert("a-1".into(), configured("https://work.example.test"));
        ok.named
            .insert("a".repeat(32), configured("https://work.example.test"));
        assert_eq!(ok.configured_providers(no_files).unwrap().len(), 2);
    }

    #[test]
    fn providers_are_listed_original_first_with_their_own_callbacks() {
        let mut settings = Settings {
            original: ProviderSettings {
                enabled: true,
                ..configured("https://auth.example.test")
            },
            ..Settings::default()
        };
        settings
            .named
            .insert("work".into(), configured("https://work.example.test"));
        // Not over HTTPS: left out, like an unconfigured original.
        settings
            .named
            .insert("lab".into(), configured("http://lab.example.test"));

        let providers = settings.configured_providers(no_files).unwrap();

        let ids: Vec<_> = providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["default", "work"]);
        let callbacks: Vec<_> = providers.iter().map(|p| p.callback_path.as_str()).collect();
        assert_eq!(
            callbacks,
            ["/api/auth/oidc/callback", "/api/auth/oidc/work/callback"]
        );
        let schemes: Vec<_> = providers.iter().map(Provider::scheme).collect();
        assert_eq!(schemes, ["external-oidc", "external-oidc-work"]);
        assert_eq!(providers[1].login_path(), "/api/auth/oidc/work/login");
        assert_eq!(providers[0].link_path(), "/api/auth/oidc/link");
        assert_eq!(
            providers[1].reauthenticate_path(),
            "/api/auth/oidc/work/reauthenticate"
        );
        let now = 1_800_000_000;
        assert!(!providers[1].passed_second_step(&["mfa"], Some(&now.to_string()), now));
        assert!(settings.find("work", no_files).unwrap().is_some());
        assert!(settings.find("lab", no_files).unwrap().is_none());
    }

    #[test]
    fn original_provider_is_off_until_enabled() {
        let settings = Settings {
            original: ProviderSettings {
                enabled: false,
                ..configured("https://auth.example.test")
            },
            ..Settings::default()
        };
        assert!(settings.configured_providers(no_files).unwrap().is_empty());
    }

    #[test]
    fn trusted_provider_second_step_counts_only_when_recent_and_reported() {
        let provider = Provider::new(
            "default",
            "/cb",
            &ProviderSettings {
                trust_provider_mfa: true,
                provider_mfa_amr: "mfa, hwk".into(),
                ..configured("https://a.example.test")
            },
            &no_files,
        );
        let now = 1_800_000_000i64;
        let at = |ago: i64| Some((now - ago).to_string());

        assert!(provider.passed_second_step(&["pwd", "hwk"], at(60).as_deref(), now));
        assert!(!provider.passed_second_step(&["pwd"], at(60).as_deref(), now));
        // A sign-in the provider waved through on an old session, or that it did not date.
        assert!(!provider.passed_second_step(&["mfa"], at(3 * 3600).as_deref(), now));
        assert!(!provider.passed_second_step(&["mfa"], None, now));
        assert!(!provider.passed_second_step(&["mfa"], Some("yesterday"), now));
        assert!(!provider.passed_second_step(&["mfa"], at(-2 * 3600).as_deref(), now));
    }

    #[test]
    fn second_step_is_off_unless_the_provider_is_trusted() {
        let provider = Provider::new(
            "default",
            "/cb",
            &configured("https://a.example.test"),
            &no_files,
        );
        let now = 1_800_000_000;
        assert!(!provider.passed_second_step(&["mfa"], Some(&now.to_string()), now));
    }

    #[test]
    fn auth_time_window_has_the_exact_bounds() {
        let now = 1_800_000_000i64;
        let recent = |t: i64| signed_in_recently(Some(&t.to_string()), now);
        assert!(recent(now - 360));
        assert!(!recent(now - 361));
        assert!(recent(now + 60));
        assert!(!recent(now + 61));
        for text in [
            "",
            "-5",
            "+5",
            " 5",
            "5 ",
            "1e3",
            "1.5",
            "٣",
            "99999999999999999999",
        ] {
            assert_eq!(signed_in_at(Some(text)), None, "{text:?}");
        }
        assert_eq!(signed_in_at(Some("0")), Some(0));
        assert_eq!(signed_in_at(Some("9999999999999")), Some(MAX_UNIX_SECONDS));
    }

    #[test]
    fn fresh_sign_in_counts_from_when_the_provider_signed_the_person_in() {
        let now = 1_800_000_000i64;
        let fresh = |ago: i64| fresh_sign_in_time(Some(&(now - ago).to_string()), now);
        // Recent enough for the provider check, but already past the fresh window.
        assert_eq!(fresh(330), None);
        assert_eq!(fresh(301), None);
        assert_eq!(fresh(300), Some(now - 300));
        assert_eq!(fresh(180), Some(now - 180));
        // A provider's clock a little ahead counts as now.
        assert_eq!(fresh(-30), Some(now));
        assert_eq!(fresh(-120), None);
        assert_eq!(fresh(2 * 3600), None);
        assert_eq!(fresh_sign_in_time(None, now), None);
        assert_eq!(fresh_sign_in_time(Some("yesterday"), now), None);
    }

    #[test]
    fn oidc_requires_https_and_all_confidential_client_settings() {
        let ready = |settings: ProviderSettings| {
            Provider::new("default", "/cb", &settings, &no_files).is_configured()
        };
        assert!(ready(configured(
            "https://auth.example.test/application/o/stellarr/"
        )));
        assert!(!ready(configured(
            "http://auth.example.test/application/o/stellarr/"
        )));
        assert!(!ready(configured("auth.example.test")));
        assert!(!ready(ProviderSettings {
            client_id: "  ".into(),
            ..configured("https://a.test")
        }));
        assert!(!ready(ProviderSettings {
            client_secret: " ".into(),
            ..configured("https://a.test")
        }));
        assert!(!ready(ProviderSettings {
            enabled: false,
            ..configured("https://a.test")
        }));
        let relative = Provider::new("default", "cb", &configured("https://a.test"), &no_files);
        assert!(!relative.is_configured());
    }

    #[test]
    fn client_secret_can_come_from_a_file() {
        let settings = ProviderSettings {
            authority: "https://auth.example.test".into(),
            client_id: "stellarr".into(),
            client_secret_file: "/run/secrets/oidc".into(),
            ..ProviderSettings::default()
        };
        let read = |path: &str| {
            (path == "/run/secrets/oidc").then(|| "  file-backed-secret\n".to_string())
        };
        let provider = Provider::new("default", "/cb", &settings, &read);
        assert!(provider.is_configured());
        assert_eq!(provider.client_secret(), "file-backed-secret");

        let unreadable = Provider::new("default", "/cb", &settings, &no_files);
        assert!(!unreadable.is_configured());
        // The inline secret wins over the file.
        let both = ProviderSettings {
            client_secret: " inline ".into(),
            ..settings
        };
        assert_eq!(
            Provider::new("default", "/cb", &both, &read).client_secret(),
            "inline"
        );
    }

    #[test]
    fn debug_output_never_shows_the_secret() {
        let settings = ProviderSettings {
            client_secret: "hunter2".into(),
            ..ProviderSettings::default()
        };
        let provider = Provider::new("default", "/cb", &settings, &no_files);
        assert!(!format!("{settings:?} {provider:?}").contains("hunter2"));
    }

    #[test]
    fn issuer_is_stored_without_a_trailing_slash() {
        assert_eq!(canonical_issuer(" https://a.test/ "), "https://a.test");
        assert_eq!(canonical_issuer("https://a.test//"), "https://a.test");
    }

    #[test]
    fn https_urls_are_strict() {
        for good in [
            "https://a.test",
            "HTTPS://a.test/x?y=1",
            "https://a.test:8443/p",
        ] {
            assert!(is_https_url(good), "{good}");
        }
        for good in [
            "  https://a.test/x  ",
            "https://a.test:65535/x",
            "https://a.test:0/x",
            "https://[::1]/x",
            "https://[::1]:8443/x",
        ] {
            assert!(is_https_url(good), "{good:?}");
        }
        for bad in [
            "https://a.test:99999/x",
            "https://a.test:65536/x",
            "https://a.test:abc/x",
            "https://a.test:+80/x",
            "https://a.test:80:80/x",
            "https://[::1/x",
            "https://[]/x",
            "https://[::1]x/x",
            "https://[::1]:99999/x",
            "http://a.test",
            "https://",
            "https:///x",
            "https://u@a.test",
            "https://a.test/#f",
            "https://a .test",
            "https://a.test\\x",
            "ftp://a",
            "",
            "https://:80",
        ] {
            assert!(!is_https_url(bad), "{bad:?}");
        }
    }
}
